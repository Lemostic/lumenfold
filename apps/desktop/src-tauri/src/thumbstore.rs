//! On-disk thumbnail cache, sized in bytes.
//!
//! # Why the byte budget moved to disk
//!
//! The cache used to be an in-process `lru::LruCache` with a fixed *entry*
//! count, which made three of its claims false:
//!
//! * It could not survive the process, so every launch re-decoded every
//!   thumbnail the user had already looked at. The cost landed exactly where
//!   it is most visible — the first paint of the grid.
//! * Its budget was expressed as a count, so the number of bytes actually held
//!   depended on how compressible the user's photos happened to be. No
//!   "limit" could be shown to the user because there was no byte limit to show.
//! * With an entry-count cap of a few thousand, the working set of a large
//!   library thrashed: scroll back up and everything was gone.
//!
//! A directory of encoded JPEGs next to the app cache fixes all three at once,
//! and it is what makes the byte budget a real number rather than a label — the
//! same way a photo library's own cache has always worked.
//!
//! # File layout
//!
//! ```text
//! <cache>/thumbs/ab/abcdef0123456789-256.jpg
//! ```
//!
//! The first two hex characters of the asset id become a directory level. One
//! flat directory with six figures of entries is slow to list on Windows
//! (NTFS directory lookups degrade), and two levels is the cheapest fix that
//! keeps any single directory small.
//!
//! # In memory *and* on disk
//!
//! A disk cache does not remove the need for the memory LRU: reading and
//! decoding a 384px JPEG is fast, but it is not free, and the same thumbnail is
//! requested repeatedly while the user scrolls back and forth. So the two are
//! layers, not alternatives, and [`crate::library::LibraryStats`] counts both —
//! the app genuinely is holding that many bytes.
//!
//! # Eviction order
//!
//! Files are evicted oldest-first **by write time**, not by last access. Touching
//! a file on every cache hit would mean an extra open-and-write syscall on the
//! hot path to serve a benefit that only matters once the cache is full — and at
//! the default 64 GiB budget, on a machine with a disk that size, it essentially
//! never matters. The settings copy says "oldest first" for exactly this reason;
//! promising LRU-by-access would be a claim this design does not honour.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::Manager;

use crate::error::LumenError;

/// Default ceiling on the on-disk thumbnail cache: 64 GiB.
///
/// 这个默认值是被需求推着走的：旧文案写「上限默认 10GB」，而 10 GB 在真实图库上
/// 连第一遍浏览都装不下——用户明确说过它太小。缩略图一张 5–40 KiB，64 GiB
/// 意味着几十万张的可见缩略图集，而绝大多数机器根本碰不到这个数，
/// 于是淘汰逻辑平时不会运行，性能影响为零。
pub const DEFAULT_LIMIT_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// Lowest budget the settings UI offers. Below a few GiB the cache stops being
/// useful for its purpose, and offering smaller numbers only invites a user to
/// pick one that produces the thrashing this module exists to remove.
///
/// 这是**产品下限**，不是存储下限：它由 [`clamp_limit`] 在命令边界上挡一次，
/// [`ThumbStore::set_limit_bytes`] 自己不拿它管束——否则这个模块就再也没法
/// 用一个小数字单测淘汰路径，而「预算真的能生效」正是最该被测到的那件事。
pub const MIN_LIMIT_BYTES: u64 = 1 * 1024 * 1024 * 1024;

/// Applies the settings floor to a budget that came from outside.
///
/// 前端传来的 `0` 意味着「随便给个数」，而一份 0 预算的缓存会在每次写入后
/// 都扫一遍全目录；在边界上把它抬到下限，比让一个手滑变成一次性能事故好。
pub fn clamp_limit(limit_bytes: u64) -> u64 {
    limit_bytes.max(MIN_LIMIT_BYTES)
}

/// Fraction of the budget the sweeper aims for.
///
/// Sweeping down to exactly the limit means every subsequent write triggers
/// another full sweep. Backing off to 90% amortises it: a sweep per ~10% of
/// writes instead of per write.
const SWEEP_TARGET_RATIO: f64 = 0.9;

/// Encodings the store can hold, paired with the `Content-Type` each is served
/// as. Order matters only for [`ThumbStore::get`], which probes in turn.
const FORMATS: [(&str, &str); 2] = [("jpg", "image/jpeg"), ("png", "image/png")];

/// One decoded-from-disk thumbnail.
#[derive(Clone, Debug)]
pub struct StoredThumb {
    /// Encoded image bytes.
    pub bytes: std::sync::Arc<Vec<u8>>,
    /// `image/jpeg` or `image/png`.
    pub content_type: &'static str,
}

/// Disk-backed thumbnail cache. Cheap to clone conceptually: it is shared
/// through Tauri's state as one `ThumbStore` with its own interior mutex, so
/// disk I/O never blocks the library-state lock and vice versa.
///
/// # Why the directory is not in the constructor
///
/// Tauri 的 `manage` 只能挂在 builder 上，而缓存目录要 `AppHandle` 才拿得到。
/// 所以这里注册一个还没绑目录的空壳，在 `setup` 里调 [`ThumbStore::init`] 补上。
/// 未绑定时的每个方法都退化成「缓存是空的、上限是默认值」——不是 panic：
/// 协议处理器和命令都可能在 setup 之前被问到一点什么，那条路径上撒谎比崩溃好。
pub struct ThumbStore {
    inner: OnceLock<Arc<Mutex<Inner>>>,
}

struct Inner {
    dir: PathBuf,
    limit_bytes: u64,
    /// Best-effort running total of file sizes. Corrected by [`ThumbStore::sweep`]
    /// and reconciled against the directory at startup.
    bytes: u64,
}

impl Default for ThumbStore {
    fn default() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }
}

impl Inner {
    /// Opens (creating if needed) a store rooted at `dir`.
    ///
    /// The running byte total is read from `usage.bin` rather than measured,
    /// because measuring means listing the whole directory on the startup path —
    /// hundreds of thousands of entries before the window appears. A missing or
    /// stale `usage.bin` is corrected by [`ThumbStore::reconcile`], which runs in
    /// the background right after startup.
    fn open(dir: PathBuf, limit_bytes: u64) -> Self {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("lumenfold: cannot create thumbnail cache {}: {err}", dir.display());
        }
        let bytes = std::fs::read(dir.join("usage.bin"))
            .ok()
            .and_then(|raw| <[u8; 8]>::try_from(raw).ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0u64);
        Self {
            dir,
            limit_bytes,
            bytes,
        }
    }
}

impl ThumbStore {
    /// Binds the store to the app's cache directory and the persisted budget.
    ///
    /// Idempotent: the first caller wins, so a second `setup` (or a test) cannot
    /// swap the directory out from under a request already in flight.
    pub fn init<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        if self.inner.get().is_some() {
            return;
        }
        let dir = app
            .path()
            .app_cache_dir()
            .map(|dir| dir.join("thumbs"))
            .unwrap_or_else(|_| std::env::temp_dir().join("lumenfold-thumbs"));
        let _ = self
            .inner
            .set(Arc::new(Mutex::new(Inner::open(dir, load_limit(app)))));
    }

    /// The bound state, or `None` before [`ThumbStore::init`].
    fn bound(&self) -> Option<&Mutex<Inner>> {
        self.inner.get().map(Arc::as_ref)
    }

    /// Test-facing view of the bound state.
    #[cfg(test)]
    fn state(&self) -> &Mutex<Inner> {
        self.inner.get().expect("store is bound")
    }

    /// Builds a fully bound store. The app path uses [`ThumbStore::init`] instead;
    /// this exists so the unit tests do not need a Tauri runtime.
    pub fn open(dir: PathBuf, limit_bytes: u64) -> Self {
        let store = Self::default();
        let _ = store
            .inner
            .set(Arc::new(Mutex::new(Inner::open(dir, limit_bytes))));
        store
    }

    /// The configured ceiling, in bytes.
    pub fn limit_bytes(&self) -> u64 {
        self.bound()
            .map(|inner| inner.lock().expect("thumb store poisoned").limit_bytes)
            .unwrap_or(DEFAULT_LIMIT_BYTES)
    }

    /// Best-effort current size, in bytes.
    pub fn bytes(&self) -> u64 {
        self.bound()
            .map(|inner| inner.lock().expect("thumb store poisoned").bytes)
            .unwrap_or(0)
    }

    /// Sets the ceiling and applies it immediately.
    ///
    /// Lowering the limit evicts at once instead of waiting for the next write:
    /// the user just said "use less", and waiting means the disk stays full for
    /// however long it takes to look at another photo.
    ///
    /// 取值**不**在这里夹逼到 [`MIN_LIMIT_BYTES`]——那是设置页该管的事，见
    /// [`clamp_limit`]。这里只把 0 当成 1，因为 0 预算会退化成每次写入扫全目录，
    /// 而夹逼到 1 GiB 同样让这个模块没法用一个小数字去单测淘汰路径。
    pub fn set_limit_bytes(&self, limit_bytes: u64) {
        let Some(inner) = self.bound() else { return };
        let mut inner = inner.lock().expect("thumb store poisoned");
        inner.limit_bytes = limit_bytes.max(1);
        if inner.bytes > inner.limit_bytes {
            sweep(&mut inner);
        }
    }

    /// Reads a cached thumbnail, or `None` if this width was never encoded.
    pub fn get(&self, key: &str) -> Option<StoredThumb> {
        let inner = self.bound()?.lock().expect("thumb store poisoned");
        for (extension, content_type) in FORMATS {
            let path = inner.path_for(key, extension);
            match std::fs::read(&path) {
                Ok(bytes) if !bytes.is_empty() => {
                    return Some(StoredThumb {
                        bytes: std::sync::Arc::new(bytes),
                        content_type,
                    })
                }
                Ok(_) => {
                    // A zero-length file is a half-written entry from an
                    // interrupted run. Treat it as absent and let it be
                    // overwritten; serving it would show a broken image.
                }
                Err(_) => continue,
            }
        }
        None
    }

    /// Stores an encoded thumbnail, evicting if this pushed the cache over budget.
    pub fn put(&self, key: &str, bytes: &[u8], content_type: &str) {
        let Some(extension) = FORMATS
            .iter()
            .find(|(_, mime)| *mime == content_type)
            .map(|(extension, _)| *extension)
        else {
            return;
        };

        let Some(bound) = self.bound() else { return };
        let mut inner = bound.lock().expect("thumb store poisoned");
        let path = inner.path_for(key, extension);
        let Some(parent) = path.parent() else { return };
        if let Err(err) = std::fs::create_dir_all(parent) {
            eprintln!("lumenfold: cannot create {}: {err}", parent.display());
            return;
        }
        // Temp + rename so a reader (or a crash) never sees a partial file.
        // Without this a killed process leaves truncated JPEGs that decode to
        // garbage on every subsequent launch, because the cache key is unchanged.
        let temp = path.with_extension(format!("{extension}.tmp"));
        let written = std::fs::File::create(&temp).and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
        if let Err(err) = written.and_then(|()| std::fs::rename(&temp, &path)) {
            let _ = std::fs::remove_file(&temp);
            eprintln!("lumenfold: cannot cache {}: {err}", path.display());
            return;
        }

        let size = bytes.len() as u64;
        // A replacement of the same key is a different file length, so subtract
        // what the old one contributed rather than assuming it was new.
        match std::fs::metadata(&path).map(|meta| meta.len()) {
            Ok(existing) if existing != size => inner.bytes = inner.bytes.saturating_add(size).saturating_sub(existing),
            _ => inner.bytes = inner.bytes.saturating_add(size),
        }
        if inner.bytes > inner.limit_bytes {
            sweep(&mut inner);
        }
    }

    /// Removes every cached thumbnail and zeroes the counter.
    pub fn clear(&self) {
        let Some(bound) = self.bound() else { return };
        let mut inner = bound.lock().expect("thumb store poisoned");
        if std::fs::remove_dir_all(&inner.dir).is_err() {
            // The directory may not exist yet; that is not a failure.
        }
        let _ = std::fs::create_dir_all(&inner.dir);
        inner.bytes = 0;
        write_usage(&inner);
    }

    /// Re-measures the cache against the directory and rewrites `usage.bin`.
    ///
    /// Called once at startup on a blocking thread. The counter is only ever a
    /// heuristic between sweeps, and this is what stops a wrong one from turning
    /// into a wrong eviction decision or a wrong number in the status bar.
    pub fn reconcile(&self) {
        let Some(bound) = self.bound() else { return };
        reconcile_shared(bound);
    }

    /// Runs the startup re-measure on a blocking thread.
    ///
    /// 不能挡在 setup 里：一次对账要读整个缓存目录（几十万条），在窗口出现
    /// 之前做完会让「双击就有界面」这件事多等好几秒。未绑定时静默跳过——
    /// 这时缓存本来就是空的，对账无话可说。
    pub fn reconcile_detached(&self) {
        let Some(shared) = self.inner.get().cloned() else {
            return;
        };
        tauri::async_runtime::spawn_blocking(move || reconcile_shared(&shared));
    }
}

/// Re-measures the cache against the directory and rewrites `usage.bin`, from a
/// movable handle. This is the body [`ThumbStore::reconcile`] runs.
fn reconcile_shared(bound: &Mutex<Inner>) {
    let mut inner = bound.lock().expect("thumb store poisoned");
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(&inner.dir) {
        for shard in entries.flatten() {
            let Ok(children) = std::fs::read_dir(shard.path()) else { continue };
            for child in children.flatten() {
                if is_cache_file(&child.path()) {
                    total = total.saturating_add(child.metadata().map(|m| m.len()).unwrap_or(0));
                }
            }
        }
    }
    inner.bytes = total;
    write_usage(&inner);
}

impl Inner {
    /// `<dir>/<first two id chars>/<key with @ replaced by ->.<ext>`.
    ///
    /// The shard comes off the asset id rather than the whole key so that all
    /// four width buckets of one photo land in the same directory.
    fn path_for(&self, key: &str, extension: &str) -> PathBuf {
        let shard = &key[..key.len().min(2)];
        self.dir
            .join(shard)
            .join(format!("{}.{extension}", key.replace('@', "-")))
    }
}

/// Whether a directory entry is one of ours. `usage.bin` and in-flight `.tmp`
/// files are not, and must not be counted or deleted as thumbnails.
fn is_cache_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("jpg") | Some("png")
    )
}

/// Deletes oldest-first until the cache fits the budget.
fn sweep(inner: &mut Inner) {
    let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let shards = match std::fs::read_dir(&inner.dir) {
        Ok(shards) => shards,
        Err(_) => {
            write_usage(inner);
            return;
        }
    };
    for shard in shards.flatten() {
        let Ok(children) = std::fs::read_dir(shard.path()) else { continue };
        for child in children.flatten() {
            let path = child.path();
            if !is_cache_file(&path) {
                continue;
            }
            let Ok(meta) = child.metadata() else { continue };
            let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            entries.push((mtime, meta.len(), path));
        }
    }

    // Oldest write first. `SystemTime` is Ord, and ties (whole-second
    // granularity on some filesystems) break on the path so the order is total
    // and therefore deterministic.
    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));

    let target = (inner.limit_bytes as f64 * SWEEP_TARGET_RATIO) as u64;
    let mut total: u64 = entries.iter().map(|entry| entry.1).sum();
    for (_, size, path) in entries {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }

    inner.bytes = total;
    write_usage(inner);
}

/// Persists the running total so the next start does not have to measure.
fn write_usage(inner: &Inner) {
    if let Ok(mut file) = std::fs::File::create(inner.dir.join("usage.bin")) {
        let _ = file.write_all(&inner.bytes.to_le_bytes());
    }
}

/// `cache.json` in the Tauri config dir; holds the user's cache budget.
fn config_file<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, LumenError> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("cache.json"))
        .map_err(|err| LumenError::io(err.to_string()))
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CacheConfig {
    /// Bytes the thumbnail cache may occupy. `0` means "never configured".
    thumb_cache_limit_bytes: u64,
}

/// Reads the persisted budget, falling back to [`DEFAULT_LIMIT_BYTES`].
pub fn load_limit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> u64 {
    let limit = config_file(app)
        .ok()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|body| serde_json::from_str::<CacheConfig>(&body).ok())
        .map(|config| config.thumb_cache_limit_bytes)
        .unwrap_or(0);
    if limit == 0 {
        DEFAULT_LIMIT_BYTES
    } else {
        clamp_limit(limit)
    }
}

/// Persists the budget so the next launch starts with it.
pub fn save_limit<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    limit_bytes: u64,
) -> Result<(), LumenError> {
    let file = config_file(app)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            LumenError::io(format!("cannot create {}: {err}", parent.display()))
        })?;
    }
    let body = serde_json::to_string_pretty(&CacheConfig {
        thumb_cache_limit_bytes: clamp_limit(limit_bytes),
    })
    .map_err(|err| LumenError::io(err.to_string()))?;
    std::fs::write(&file, body)
        .map_err(|err| LumenError::io(format!("cannot write {}: {err}", file.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试目录。每个用例自己建一个，删掉上一个，避免 Windows 上
    /// 目录句柄还没释放时的删除失败。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("lumenfold-thumbstore-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn put_then_get_returns_the_same_bytes_and_type() {
        let dir = TempDir::new("roundtrip");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);

        assert!(store.get("abcdef0123456789@256").is_none());
        store.put("abcdef0123456789@256", &[1, 2, 3, 4], "image/jpeg");

        let hit = store.get("abcdef0123456789@256").expect("cached");
        assert_eq!(&*hit.bytes, &[1, 2, 3, 4]);
        assert_eq!(hit.content_type, "image/jpeg");
    }

    #[test]
    fn png_and_jpeg_of_one_key_coexist() {
        let dir = TempDir::new("formats");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);

        store.put("abcdef0123456789@256", &[9, 9], "image/png");
        let hit = store.get("abcdef0123456789@256").expect("cached");
        assert_eq!(hit.content_type, "image/png");
        assert_eq!(&*hit.bytes, &[9, 9]);
    }

    #[test]
    fn cache_file_paths_are_sharded_by_the_asset_id() {
        let dir = TempDir::new("shard");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);
        store.put("abcdef0123456789@256", &[7], "image/jpeg");

        let expected = dir.path().join("ab").join("abcdef0123456789-256.jpg");
        assert!(expected.exists(), "expected {}", expected.display());
    }

    /// 预算必须是真的会生效的数，而不是一个写死的字符串。
    #[test]
    fn lowering_the_limit_evicts_until_the_cache_fits() {
        let dir = TempDir::new("budget");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);

        for i in 0..40 {
            store.put(&format!("aaaa0000000000{i:02x}@256"), &[0u8; 1024], "image/jpeg");
        }
        assert_eq!(store.bytes(), 40 * 1024);

        // 20 KiB 的预算放不下 40 KiB，淘汰必须真的删文件，而不是只改计数。
        store.set_limit_bytes(20 * 1024);
        assert!(store.bytes() <= 20 * 1024);
        let left = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|shard| std::fs::read_dir(shard.path()).is_ok())
            .flat_map(|shard| std::fs::read_dir(shard.path()).unwrap().flatten())
            .filter(|entry| is_cache_file(&entry.path()))
            .count();
        assert_eq!(left as u64 * 1024, store.bytes());
    }

    #[test]
    fn clear_empties_the_directory_and_resets_the_counter() {
        let dir = TempDir::new("clear");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);
        store.put("abcdef0123456789@256", &[1, 2, 3], "image/jpeg");
        assert!(store.bytes() > 0);

        store.clear();
        assert_eq!(store.bytes(), 0);
        assert!(store.get("abcdef0123456789@256").is_none());
    }

    /// 开机时那个不扫目录的计数器必须能被真值纠正，否则状态栏会一直说谎。
    #[test]
    fn reconcile_replaces_a_stale_counter_with_the_truth() {
        let dir = TempDir::new("reconcile");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);
        store.put("abcdef0123456789@256", &[0u8; 500], "image/jpeg");
        store.put("bbcdef0123456789@256", &[0u8; 300], "image/jpeg");
        assert_eq!(store.bytes(), 800);

        // Simulate a counter that drifted (a crash before the last write_usage).
        {
            let mut inner = store.state().lock().unwrap();
            inner.bytes = 42;
        }
        store.reconcile();
        assert_eq!(store.bytes(), 800);
    }

    /// 一个被打断的写入会留下半个文件；它必须被当成「没有」，不能当缓存发出去。
    #[test]
    fn a_zero_length_entry_reads_as_absent() {
        let dir = TempDir::new("torn");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);
        let path = dir.path().join("ab").join("abcdef0123456789-256.jpg");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();

        assert!(store.get("abcdef0123456789@256").is_none());
    }

    /// 设置页下限必须真的挡在边界上，而不是靠 store 顺手夹一下。
    ///
    /// 这条用例盯的是分层：store 只管淘汰，产品的下限由 `clamp_limit` 管。
    /// 两边都夹，看起来结果一样；可一旦 store 也夹，它就再也没法用一个小数字
    /// 单测淘汰路径——上面那条用例就是这么写的。
    #[test]
    fn the_settings_floor_is_applied_by_clamp_and_not_by_the_store() {
        assert_eq!(clamp_limit(0), MIN_LIMIT_BYTES);
        assert_eq!(clamp_limit(512), MIN_LIMIT_BYTES);
        assert_eq!(clamp_limit(DEFAULT_LIMIT_BYTES), DEFAULT_LIMIT_BYTES);

        let dir = TempDir::new("clamp");
        let store = ThumbStore::open(dir.path().to_path_buf(), DEFAULT_LIMIT_BYTES);
        // store 本身照单全收小数字：这是它能被人测的前提。
        store.set_limit_bytes(512);
        assert_eq!(store.limit_bytes(), 512);
    }

    /// setup 之前被问到的那一下：什么都还没有，但也不能崩，也不能谎报容量。
    ///
    /// 协议处理器和命令都可能在 `init` 之前被问到一嘴；那里 panic 等于
    /// 整个图库打不开，而谎报一个非零容量会让状态栏先写一句假话。
    #[test]
    fn an_unbound_store_reports_an_empty_cache_instead_of_panicking() {
        let store = ThumbStore::default();

        assert_eq!(store.bytes(), 0);
        assert_eq!(store.limit_bytes(), DEFAULT_LIMIT_BYTES);
        assert!(store.get("abcdef0123456789@256").is_none());

        // All of these are no-ops rather than failures; none may panic.
        store.put("abcdef0123456789@256", &[1, 2, 3], "image/jpeg");
        store.set_limit_bytes(MIN_LIMIT_BYTES);
        store.reconcile();
        store.clear();

        assert_eq!(store.bytes(), 0);
        assert_eq!(store.limit_bytes(), DEFAULT_LIMIT_BYTES);
    }
}