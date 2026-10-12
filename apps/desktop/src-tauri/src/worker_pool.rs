//! A small fixed-size thread pool, used to keep heavy work off the WebView's
//! event thread.
//!
//! # 为什么必须有它
//!
//! `lumen://` 是用 `register_asynchronous_uri_scheme_protocol` 注册的。名字里的
//! "asynchronous" 只说明**响应**可以晚点给，**不代表处理函数跑在别的线程上**。
//! 整条链路是：
//!
//! 1. `tauri::Builder::register_asynchronous_uri_scheme_protocol` →
//!    `tauri-runtime-wry` 原样转成 wry 的 `with_asynchronous_custom_protocol`
//!    （`tauri-runtime-wry-2.12.1/src/lib.rs:5134`）。
//! 2. wry 在 WebView2 的 `WebResourceRequested` 事件回调里**同步调用**这个闭包
//!    （`wry-0.57.0/src/webview2/mod.rs:1094`，同一函数在 1085 行还专门比较
//!    `std::thread::current().id()` 和 `main_thread_id`，因为响应有可能从别的
//!    线程发回来）。
//! 3. WebView2 的事件在创建 WebView 的那个线程上触发，也就是 UI 线程。
//!
//! 所以 `responder.respond(handle(..))` 这一行，实际上是在 UI 线程上做一次
//! 完整的解码 + 缩放 + 重编码：一张 2400 万像素的照片要 100–200ms
//! （见 `thumbnail.rs` 里 JPEG 那条路的注释）。一屏几十个格子，就是几秒钟
//! 窗口完全没有响应；扫描期间磁盘和 CPU 都被抢，单次解码更慢，卡顿也就更长。
//! 这正是「扫描的时候主界面卡住」的来源——不是扫描本身在抢渲染线程，
//! 而是**解码缩略图**跑在了渲染线程上。
//!
//! # 为什么是池子而不是 `thread::spawn`
//!
//! 一屏会同时要几十上百张缩略图。一个请求起一个线程，就是几百个线程同时
//! 抢 CPU：解码总量不变，反而把界面和扫描一起拖慢。固定几个线程时，
//! 排队的请求只是晚一点回来，而设备始终在做有用的事。
//!
//! 池子里的线程数与核数挂钩但夹在 `[2, 8]`，理由见 [`default_threads`]。

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

/// 池子里排队等待执行的任务。
type Job = Box<dyn FnOnce() + Send + 'static>;

/// 队列本身，被所有 worker 和投递方共享。
struct Shared {
    queue: Mutex<VecDeque<Job>>,
    ready: Condvar,
}

/// 固定大小的后台线程池。用法见模块头注释。
pub struct WorkerPool {
    shared: Arc<Shared>,
    /// 真正起来的 worker 数。0 表示线程一个都没建成，见 [`WorkerPool::dispatch`]。
    workers: usize,
}

impl WorkerPool {
    /// 起一个 `threads` 个 worker 的池子，`threads` 至少按 1 算。
    ///
    /// 线程起不来不是致命错误：少一个 worker，缩略图慢一点，但池子还在跑。
    /// 一个都起不来时 `workers` 是 0，[`WorkerPool::dispatch`] 会退化成
    /// 在调用线程上直接跑——那条路会卡住 UI，但好过让请求永远不回。
    pub fn new(threads: usize) -> Self {
        let threads = threads.max(1);
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        });

        let mut workers = 0usize;
        for index in 0..threads {
            let shared = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name(format!("lumen-worker-{index}"))
                .spawn(move || worker_loop(&shared));
            match spawned {
                Ok(_) => workers += 1,
                Err(err) => {
                    eprintln!("lumenfold: cannot spawn background worker {index}: {err}");
                }
            }
        }

        Self { shared, workers }
    }

    /// 把 `job` 排进池子。调用线程立刻返回，不等它跑完。
    pub fn dispatch(&self, job: impl FnOnce() + Send + 'static) {
        if self.workers == 0 {
            // 建不出线程时的降级路径。会阻塞调用线程，但没有别的选择：
            // 返回而不跑，等于让这个 HTTP 请求永远挂着。
            eprintln!("lumenfold: no background worker available; running inline");
            job();
            return;
        }
        lock(&self.shared.queue).push_back(Box::new(job));
        self.shared.ready.notify_one();
    }

    /// 已经起来的 worker 数，给测试和日志用。
    pub fn workers(&self) -> usize {
        self.workers
    }
}

/// 进程里共用的那一个池子。
///
/// 每次注册协议就起一个池子，会让线程数随窗口数悄悄增长；而这个进程只有一个
/// WebView。用 `OnceLock` 把「进程里到底有几个解码线程」变成一眼能数清的事。
pub fn shared() -> &'static WorkerPool {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(|| WorkerPool::new(default_threads()))
}

/// 解码线程数：跟着核数走，但夹在 `[2, 8]`。
///
/// 下限 2 是因为单核机器上也得有一个线程在等 I/O——只有 1 个 worker 时，
/// 一次慢解码就把整条流堵死，正是这个池子要消除的现象。
/// 上限 8 是因为缩略图真正的瓶颈是磁盘和 WebView 的请求节奏，不是核数：
/// 32 核上开 32 个解码线程，只会跟同时进行的目录扫描互相抢。
fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .clamp(2, 8)
}

/// worker 主循环：取一个任务，跑掉，再取下一个。
fn worker_loop(shared: &Shared) {
    loop {
        let job = {
            let mut queue = lock(&shared.queue);
            while queue.is_empty() {
                // 中毒只说明某个任务 panic 过，队列本身还是好的。
                queue = lock_poisoned(shared.ready.wait(queue));
            }
            queue.pop_front()
        };

        let Some(job) = job else { continue };

        // 一个任务 panic 不能带走一个 worker。池子只有几个线程，少一个就是
        // 整体吞吐掉一截，而协议处理器里 panic 是有可能的（截断的文件、
        // 越界的尺寸）。响应由任务自己负责发出去，这里只保证池子活下来。
        let _ = catch_unwind(AssertUnwindSafe(|| job()));
    }
}

/// 队列锁中毒不算故障，取回内部数据继续用。
///
/// 中毒只意味着某个任务 panic 过，而 `VecDeque<Job>` 本身没有不变式会被
/// panic 破坏——把池子跟着一起废掉才是真故障。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    lock_poisoned(mutex.lock())
}

fn lock_poisoned<T>(result: std::sync::LockResult<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    result.unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// 超时是必须的：池子出问题时表现是「任务永远不跑」，
    /// 不带超时的 `recv` 会让测试挂死而不是失败。
    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn runs_every_job_exactly_once() {
        let pool = WorkerPool::new(3);
        let (tx, rx) = mpsc::channel();
        for index in 0..64usize {
            let tx = tx.clone();
            pool.dispatch(move || {
                let _ = tx.send(index);
            });
        }
        // 丢掉手里这一份发送端，通道才会在最后一个任务跑完时关闭。
        drop(tx);

        let mut seen: Vec<usize> = rx.iter().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..64).collect::<Vec<_>>());
    }

    #[test]
    fn work_runs_off_the_calling_thread() {
        let pool = WorkerPool::new(1);
        let caller = std::thread::current().id();
        let (tx, rx) = mpsc::channel();
        pool.dispatch(move || {
            let _ = tx.send(std::thread::current().id());
        });
        assert_ne!(
            rx.recv_timeout(WAIT).expect("worker never reported"),
            caller,
            "任务跑在调用线程上了,这个池子就没有意义了"
        );
    }

    #[test]
    fn a_panicking_job_does_not_take_the_worker_down() {
        // 只有一个 worker：如果 panic 把它带走了,后面这个任务永远不会跑,
        // 测试就会在 recv_timeout 上失败而不是假装通过。
        let pool = WorkerPool::new(1);
        let (tx, rx) = mpsc::channel();

        pool.dispatch(|| panic!("job exploded on purpose"));
        let tx_after = tx.clone();
        pool.dispatch(move || {
            let _ = tx_after.send("survived");
        });
        drop(tx);

        assert_eq!(rx.recv_timeout(WAIT).expect("worker died with the job"), "survived");
        assert_eq!(pool.workers(), 1);
    }

    #[test]
    fn dispatch_returns_before_the_job_finishes() {
        let pool = WorkerPool::new(1);
        let (tx, rx) = mpsc::channel();
        pool.dispatch(move || {
            std::thread::sleep(Duration::from_millis(200));
            let _ = tx.send(());
        });

        // 投递本身必须是立刻返回的。这里给 100ms 的宽限:
        // 真要是同步执行,上面那 200ms 会让这一行超时。
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "dispatch 是同步跑的,UI 线程还是会被挡住"
        );
        rx.recv_timeout(WAIT).expect("job never finished");
    }

    #[test]
    fn default_thread_count_stays_in_the_documented_range() {
        assert!((2..=8).contains(&default_threads()), "{}", default_threads());
    }
}
