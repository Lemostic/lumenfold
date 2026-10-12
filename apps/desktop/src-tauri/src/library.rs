//! Managed library state: the active root, the scanned records, and the
//! in-memory thumbnail cache.
//!
//! All of it lives behind a single `tauri::State<Mutex<LibraryState>>`, so
//! commands are cheap and the `lumen://` thumbnail protocol can look an asset up
//! without a round trip through IPC. Scans run on `spawn_blocking` and publish
//! each record into that same mutex as it is discovered, which is what lets
//! `library_stats` report a growing `asset_count` while the walk is still going.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::sync::Arc;

use lumenfold_index::{orientation, AssetRecord};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::LumenError;
use crate::types::{AssetMetadata, AssetQuery, AssetSummary, AssetTagCount, LibraryStats, SearchResult};

/// Thumbnail cache size, as a count of entries.
///
/// `lru` evicts by entry count, not by byte weight, so the budget is expressed
/// as a count. 界面只请求 128–512px 四档，一张缩略图编码后约 5–40 KiB，
/// 8192 条大致落在 200–300 MiB，和 256 MiB 的目标量级一致。
///
/// 之前这里只缓存 1024 条：按当年「1024px 缩略图有 150–300 KiB」估算出来的数字，
/// 但界面从来没请求过那么大的尺寸。实际每条只有几十 KiB，于是缓存只占了几十 MiB，
/// 往回滚几百张就要重新解码——用户看到的就是「刚才看过的照片又卡一遍」。
const THUMB_CACHE_ENTRIES: usize = 8192;

/// Encoded thumbnail plus the `Content-Type` it has to be served with.
#[derive(Clone, Debug)]
pub struct CachedThumb {
    /// Encoded image bytes: JPEG, or PNG for sources with alpha.
    pub bytes: Arc<Vec<u8>>,
    /// `image/jpeg` or `image/png`.
    pub content_type: &'static str,
}

/// Everything the backend knows about the currently open library.
pub struct LibraryState {
    /// 作为图库的目录，按用户排定的顺序。
    ///
    /// 以前这里是 `root: Option<PathBuf>`，一个图库只有一个根。现在是一组：
    /// 图库更像一个知识库，可以同时挂 `iCloud Photos`、`DCIM` 和一个素材盘，
    /// 各自保有自己的增量快照（见 `library_index`，键就是目录本身）。
    pub roots: Vec<PathBuf>,
    /// Records found by the current or most recent scan.
    pub assets: Vec<AssetRecord>,
    /// `id -> position in assets`, so lookups do not scan the vector.
    pub index: HashMap<String, usize>,
    /// `true` while a scan is in flight.
    pub scanning: bool,
    /// The most recent scan was stopped by the user.
    ///
    /// 单独一个布尔量而不是「扫描结束时顺手清掉」：界面需要区分
    /// 「扫完了」和「你自己叫停的」，而这两种情况下 `scanning` 都是 false。
    pub cancelled: bool,
    /// Reused/probed split of the most recent walk.
    ///
    /// 增量扫描最容易被写成一句没人能验证的口号。把这两条留在状态里，
    /// 状态栏就能把「复用 12,340 张 / 重新读取 12 张」直接说给用户听。
    scan_reused: u64,
    scan_probed: u64,
    /// Monotonic scan generation. Each [`start_scan`] takes the next number.
    ///
    /// 取代了「扫描线程靠比较 root 字符串判断自己是否被顶替」的做法：
    /// 同一个目录连点两次打开，旧扫描与新扫描的 root 完全相同，
    /// 只有代号不同，能把它们分开。
    scan_generation: u64,
    /// The cancel flag of the scan currently in flight.
    scan_cancel: Option<Arc<AtomicBool>>,
    /// Encoded thumbnails keyed by `"{id}@{width}"`.
    pub thumbs: lru::LruCache<String, CachedThumb>,
    /// Running total of `thumbs` weights, so `stats` does not walk the LRU.
    cache_bytes: u64,
    /// Thumbnails currently being encoded, keyed the same way.
    ///
    /// 界面重排时同一个 URL 往往被同时请求好几次。没有这道闸，
    /// 同一张照片就会被并排解码三四次；有了它，后来的人等第一个人的结果。
    pub decoding: HashMap<String, Arc<OnceLock<Result<CachedThumb, LumenError>>>>,
}

impl LibraryState {
    /// An empty library. `open_library` or [`restore`] fills it in.
    pub fn empty() -> Self {
        Self {
            roots: Vec::new(),
            assets: Vec::new(),
            index: HashMap::new(),
            scanning: false,
            cancelled: false,
            scan_reused: 0,
            scan_probed: 0,
            scan_generation: 0,
            scan_cancel: None,
            thumbs: lru::LruCache::new(
                std::num::NonZeroUsize::new(THUMB_CACHE_ENTRIES).expect("cache size is nonzero"),
            ),
            cache_bytes: 0,
            decoding: HashMap::new(),
        }
    }

    /// Live counters for the home view. `asset_count` grows mid-scan.
    ///
    /// The two cache fields describe only this struct's in-memory LRU. The disk
    /// cache is a separate store behind its own lock, so `cache_limit_bytes`
    /// stays `0` here and every command that returns stats folds both layers in
    /// through `commands::with_cache_totals` — that is the only place that
    /// knows about the disk store, and it is what keeps the status bar from
    /// reporting half the truth.
    pub fn stats(&self) -> LibraryStats {
        LibraryStats {
            asset_count: self.assets.len() as u64,
            indexed: !self.roots.is_empty(),
            roots: self
                .roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect(),
            // 计数是增量维护的，不是每次现算。扫描期间每 400ms 轮询一次状态，
            // 而 LRU 里有几千条时每次遍历一遍纯属白烧 CPU——
            // 那正是扫描时界面「感觉卡」的一个来源。
            cache_bytes: self.cache_bytes,
            cache_limit_bytes: 0,
            scan_reused: self.scan_reused,
            scan_probed: self.scan_probed,
            scanning: self.scanning,
            cancelled: self.cancelled,
        }
    }

    /// Inserts a thumbnail, keeping the byte counter in step with evictions.
    ///
    /// `lru` evicts on insert, so the evicted entry's weight has to come back
    /// off the total before the new one goes on. Skipping that would let the
    /// number in the status bar drift upward forever and eventually be a lie.
    pub fn put_thumb(&mut self, key: String, thumb: CachedThumb) {
        let weight = thumb.bytes.len() as u64;
        // 同一把 key 被换掉时 `put` 返回的是旧值，正好先把它减掉。
        if let Some(evicted) = self.thumbs.put(key, thumb) {
            self.cache_bytes = self.cache_bytes.saturating_sub(evicted.bytes.len() as u64);
        }
        self.cache_bytes += weight;
    }

    /// Drops every cached thumbnail, e.g. when the library root changes.
    pub fn clear_thumbs(&mut self) {
        self.thumbs.clear();
        self.cache_bytes = 0;
    }

    /// Appends one freshly walked record, keeping the id index in step.
    fn push(&mut self, record: AssetRecord) {
        let position = self.assets.len();
        self.index.insert(record.id.clone(), position);
        self.assets.push(record);
    }

    /// Inserts a record, replacing the existing one with the same id.
    ///
    /// 增量扫描的常态：这张照片上次就在索引里，但大小或 mtime 变了，
    /// 于是重新探测了一次。此时要**换掉**旧记录而不是追加——追加会让同一个
    /// id 在 assets 里出现两次，index 只记着后一个，前一个成了永远查不到、
    /// 但仍然占着分页和网格位置的幽灵。
    fn upsert(&mut self, record: AssetRecord) {
        match self.index.get(&record.id) {
            Some(&position) => self.assets[position] = record,
            None => self.push(record),
        }
    }

    /// Replaces the whole record set, e.g. with a persisted snapshot.
    fn replace_assets(&mut self, records: Vec<AssetRecord>) {
        self.assets = records;
        self.reindex();
    }

    /// Rebuilds `index` from `assets`. Needed after any sort or bulk edit.
    fn reindex(&mut self) {
        self.index = self
            .assets
            .iter()
            .enumerate()
            .map(|(position, record)| (record.id.clone(), position))
            .collect();
    }

    /// Drops records whose id is not in `seen`.
    ///
    /// `seen` 是刚才那一遍走盘在磁盘上实际见到的全部 id。没被见到的记录，
    /// 要么是文件在应用之外被删了，要么是被移走了：索引要是留着它们，
    /// 网格上就会有一格永远点不开、缩略图永远转圈的死照片。
    ///
    /// 只有完整走完一遍才能这么裁——被叫停的那一遍只见到目录的一部分，
    /// 拿它当「全集的补集」会把整个图库删干净。
    fn retain_ids(&mut self, seen: &HashSet<String>) {
        self.assets.retain(|record| seen.contains(&record.id));
        self.reindex();
    }

    /// Grid/list projection of one record.
    fn summary(&self, position: usize) -> AssetSummary {
        let record = &self.assets[position];
        AssetSummary {
            id: record.id.clone(),
            path: record.path.to_string_lossy().into_owned(),
            file_name: record.file_name.clone(),
            width: record.width,
            height: record.height,
            bytes: record.bytes,
            taken_at: record.taken_at,
            // Rating and flagging are user-owned and nothing has ever written
            // them, so they report their empty values rather than a per-asset
            // guess.
            rating: 0,
            flagged: false,
            orientation: orientation(record.width, record.height).to_string(),
            tags: record.tags,
        }
    }

    /// Sorts, then pages the current assets.
    pub fn query(&self, query: &AssetQuery) -> Vec<AssetSummary> {
        // `bucket` and `person_id` come from the timeline and people views,
        // which have no backing index yet. They are accepted and ignored rather
        // than rejected, so those views can call through before their indexes
        // exist.
        //
        // 类别筛选在排序和分页**之前**做：反过来的话 offset 会落在已经被滤掉的
        // 条目上，翻页就会一页多一页少。
        let mut positions: Vec<usize> = match query.tag.as_deref() {
            None => (0..self.assets.len()).collect(),
            Some(name) => match lumenfold_index::tag_bit(name) {
                Some(bit) => (0..self.assets.len())
                    .filter(|&position| self.assets[position].tags & bit != 0)
                    .collect(),
                // 认不出的类别 id：零张才是正确答案，而不是把整库端出去。
                None => Vec::new(),
            },
        };
        positions.sort_by(|&a, &b| compare(&self.assets, a, b, &query.sort));

        positions
            .into_iter()
            .skip(query.offset as usize)
            .take(query.limit as usize)
            .map(|position| self.summary(position))
            .collect()
    }

    /// 每个类别当前有多少张照片。
    ///
    /// 每一行都带上「已经分过类的总数」：老快照里的记录 tags 是 0，分类要等
    /// 增量扫描一轮轮把容器头重读一遍才补齐。没有这个数，界面就只能显示
    /// 「动图 0 张」——那是在把「还没数」说成「没有」。
    pub fn tag_counts(&self) -> Vec<AssetTagCount> {
        let mut counts: Vec<AssetTagCount> = lumenfold_index::FILTERABLE_TAGS
            .iter()
            .map(|(id, _)| AssetTagCount {
                id: (*id).to_string(),
                count: 0,
                classified: 0,
                total: 0,
            })
            .collect();

        let total = self.assets.len() as u64;
        let mut classified = 0u64;
        for record in &self.assets {
            if record.tags & lumenfold_index::TAG_CLASSIFIED == 0 {
                continue;
            }
            classified += 1;
            for (index, (_, bit)) in lumenfold_index::FILTERABLE_TAGS.iter().enumerate() {
                if record.tags & bit != 0 {
                    counts[index].count += 1;
                }
            }
        }
        for count in &mut counts {
            count.classified = classified;
            count.total = total;
        }
        counts
    }

    /// 文本检索：在**已经扫到的**索引里按文件名与路径找子串。
    ///
    /// # 为什么这一版只查文件名和路径
    ///
    /// 「关键词」和「说明文字」不在扫描读到的东西里——它们是元数据库
    /// （ADR-0001 §4 的 SQLite + FTS5）里的字段，而那张表还没建。
    /// 与其回一句「索引还没建好」把搜索整个关掉，不如把真正能查的那部分查出来：
    /// 用户输入的多半就是文件名，这在今天已经够用，而且**答案是真的**。
    /// 等 FTS5 落地，这里换成一次 SQL 查询即可，调用方不用动。
    ///
    /// # 打分只用来排序
    ///
    /// 文件名整体相等 > 文件名前缀 > 文件名子串 > 路径子串。
    /// 分数不冒充相关性，也不跨检索模式比较——`SearchResult::score` 的文档
    /// 已经写明这一点。同分时新拍的在前：搜同一个词，用户更可能想先看最近那张。
    pub fn search_text(&self, query: &str, limit: u32) -> Vec<SearchResult> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }

        let mut hits: Vec<(f64, usize)> = Vec::new();
        for (position, record) in self.assets.iter().enumerate() {
            // 大小写不敏感地比。每张照片一次小写分配，一万多张是毫秒级，
            // 比在记录里再存一份小写副本（那会改掉落盘的快照格式）划算。
            let name = record.file_name.to_lowercase();
            let score = if name == needle {
                3.0
            } else if name.starts_with(&needle) {
                2.0
            } else if name.contains(&needle) {
                1.0
            } else if record.path.to_string_lossy().to_lowercase().contains(&needle) {
                // 命中的是目录名（比如「iCloud Photos」），排在文件名命中之后。
                0.5
            } else {
                continue;
            };
            hits.push((score, position));
        }

        hits.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| self.assets[b.1].taken_at.cmp(&self.assets[a.1].taken_at))
                .then_with(|| self.assets[a.1].path.cmp(&self.assets[b.1].path))
        });

        hits.into_iter()
            .take(limit as usize)
            .map(|(score, position)| SearchResult {
                asset: self.summary(position),
                score,
            })
            .collect()
    }

    /// Capture metadata for one asset.
    ///
    /// Only the file's identity and capture time are known from the scan, so
    /// every camera/lens/GPS field stays `None`: the header probe never read
    /// them, and inventing plausible EXIF would be worse than showing nothing.
    pub fn metadata(&self, asset_id: &str) -> Result<AssetMetadata, LumenError> {
        let position = *self
            .index
            .get(asset_id)
            .ok_or_else(|| LumenError::io(format!("unknown asset id {asset_id:?}")))?;
        let record = &self.assets[position];
        Ok(AssetMetadata {
            asset_id: record.id.clone(),
            camera_make: None,
            camera_model: None,
            lens: None,
            iso: None,
            shutter: None,
            aperture: None,
            focal_length_mm: None,
            keywords: Vec::new(),
            caption: None,
            gps_lat: None,
            gps_lon: None,
        })
    }
}

/// Ordering for one sort key. Ties fall back to path so paging is stable.
fn compare(assets: &[AssetRecord], a: usize, b: usize, sort: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (left, right) = (&assets[a], &assets[b]);

    // Compare capture times, keeping undated assets last in *both* directions:
    // they belong at the end of the timeline, not at the epoch. So the
    // nullness is a primary key of its own and only real dates are reversed.
    let by_take = |descending: bool| -> Ordering {
        match (left.taken_at, right.taken_at) {
            (Some(x), Some(y)) => {
                if descending {
                    y.cmp(&x)
                } else {
                    x.cmp(&y)
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    };

    let primary = match sort {
        "taken-asc" => by_take(false),
        "name" => left.file_name.cmp(&right.file_name),
        "size" => right.bytes.cmp(&left.bytes),
        // Ratings are all zero until editing exists, so this degrades to
        // newest-first rather than to a meaningless shuffle.
        "rating" => Ordering::Equal,
        _ => by_take(true),
    };
    primary.then_with(|| left.path.cmp(&right.path))
}

/// The shared state type, named once so commands and the protocol agree.
pub type SharedLibrary = std::sync::Mutex<LibraryState>;

/// 按 id 去重，保留第一次出现的那条。
///
/// 图库允许嵌套：把 `D:\Photos` 和 `D:\Photos\2024` 都挂上时，同一张照片会被
/// 两个快照各记一次。id 是路径的哈希，所以两份记录的 id 相同，去重即可；
/// 不去重的话同一个 id 在 `assets` 里出现两次，`index` 只记住后一个，
/// 前一个就成了永远查不到、却仍然占着分页位置的幽灵。
fn dedupe_by_id(records: Vec<AssetRecord>) -> Vec<AssetRecord> {
    let mut seen: HashSet<String> = HashSet::with_capacity(records.len());
    let mut out: Vec<AssetRecord> = Vec::with_capacity(records.len());
    for record in records {
        if seen.insert(record.id.clone()) {
            out.push(record);
        }
    }
    out
}

/// Starts a background scan over every configured root.
///
/// A scan already in flight is superseded rather than cancelled: whichever call
/// arrives last owns the state, because two walkers publishing into the same
/// list would interleave into a mixture of both folders. The superseded scan's
/// records are dropped when it reaches the end and finds the generation moved.
///
/// # 扫描是增量的，而且是先铺旧账再核对
///
/// 一次全量扫描真正的开销不在走目录（那只是每文件一次 stat），而在
/// [`lumenfold_index::probe`]：每张照片都要打开、解头、读 EXIF。所以每个目录各自三步走：
///
/// 1. 上次落盘的结果原样铺开。格子的宽高就在记录里，铺开后网格立刻是满的、
///    尺寸也是对的，不用等目录走完才画得出东西。
/// 2. 增量走一遍目录：大小和 mtime 都没变的文件直接复用记录，不打开照片。
/// 3. 按这一遍实际见到的 id 集合裁掉消失的照片，然后把结果写回磁盘。
///
/// 快照是**按目录分开存的**（`library_index` 的键就是目录本身），所以给图库
/// 加一个目录不会让另外几个重扫一遍。
pub fn start_scan<R: Runtime>(app: &AppHandle<R>, roots: Vec<PathBuf>) {
    let scan_roots = roots.clone();
    let cancel = Arc::new(AtomicBool::new(false));

    let generation = {
        let state = app.state::<SharedLibrary>();
        let mut guard = state.lock().expect("library state poisoned");
        let previous = guard.roots.clone();
        guard.roots = roots;
        guard.scanning = true;
        guard.cancelled = false;
        guard.scan_generation = guard.scan_generation.wrapping_add(1);
        guard.scan_cancel = Some(cancel.clone());
        // 只有目录集合变了才清缩略图。资产 id 是路径的哈希，同一批目录重扫出来的
        // id 一模一样，缓存条目全都还有效——每次重扫都清空，等于让用户
        // 每按一次「重新扫描」就把整个缓存重新解码一遍。
        if previous != scan_roots {
            guard.clear_thumbs();
            guard.decoding.clear();
        }
        guard.scan_generation
    };

    // 配置先落盘再开扫：扫到一半崩了也不该丢掉用户刚配好的目录列表。
    if let Err(err) = persist_roots(app, &scan_roots) {
        eprintln!("lumenfold: cannot persist the library roots: {err}");
    }

    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SharedLibrary>();

        // 第一步：旧账先铺开，让界面立刻有一份完整的库可看。
        let mut seed: Vec<AssetRecord> = Vec::new();
        let mut caches: Vec<(PathBuf, HashMap<String, AssetRecord>)> = Vec::new();
        for root in &scan_roots {
            let snapshot = crate::library_index::load(&app, root);
            caches.push((root.clone(), crate::library_index::by_id(snapshot.clone())));
            seed.extend(snapshot);
        }
        if !seed.is_empty() {
            let mut guard = state.lock().expect("library state poisoned");
            if guard.scan_generation != generation {
                return;
            }
            guard.replace_assets(dedupe_by_id(seed));
        }

        // 第二步：逐个目录增量走一遍。每个目录的快照只装它自己名下的记录，
        // 这样删掉一个目录不会牵动别的目录的快照。
        let mut seen_all: HashSet<String> = HashSet::new();
        let mut walked_all = true;
        let mut reused = 0u64;
        let mut probed = 0u64;

        for (root, cache) in &caches {
            if cancel.load(Ordering::Relaxed) {
                walked_all = false;
                break;
            }

            let mut seen: HashSet<String> = HashSet::with_capacity(cache.len());
            let outcome = lumenfold_index::scan_each_while_cached(
                root,
                cache,
                &mut |record, from_cache| {
                    seen.insert(record.id.clone());
                    // 复用命中的记录本来就在索引里（要么是刚铺开的快照，要么是
                    // 上一次扫描留下的），再塞一遍只是白占一次锁。
                    if from_cache {
                        return;
                    }
                    let mut guard = state.lock().expect("library state poisoned");
                    // A scan that lost the race to a newer one must not keep filling
                    // the list. The generation, not the root set, is what tells them
                    // apart: rescanning the same folders produces equal sets.
                    if guard.scan_generation != generation {
                        return;
                    }
                    guard.upsert(record);
                },
                // The count is redundant with `assets.len()`; the callback exists to
                // keep `scan_each`'s contract uniform with `scan`.
                &mut |_| {},
                &|| cancel.load(Ordering::Relaxed),
            );

            match outcome {
                Ok(out) => {
                    if out.cancelled {
                        walked_all = false;
                    }
                    reused += out.reused as u64;
                    probed += out.probed as u64;

                    // 这个目录自己的那一份快照。持有锁收集是有意的：十万条记录
                    // clone 一遍要额外二十几 MB 堆内存，而放锁之后再收集就要求
                    // 期间没有别的扫描改过列表，那反而更难保证正确。
                    let records: Vec<AssetRecord> = {
                        let guard = state.lock().expect("library state poisoned");
                        guard
                            .assets
                            .iter()
                            .filter(|record| seen.contains(&record.id))
                            .cloned()
                            .collect()
                    };
                    if let Err(err) = crate::library_index::save(&app, root, &records) {
                        eprintln!(
                            "lumenfold: cannot persist index for {}: {err}",
                            root.display()
                        );
                    }
                    seen_all.extend(seen);
                }
                Err(err) => {
                    // A root that vanished or turned unreadable mid-walk leaves the
                    // root set but empty, with `scanning` cleared so the UI stops
                    // spinning. `LibraryStats` has no field to carry this reason, so
                    // it goes to the log rather than being folded into a counter.
                    eprintln!("lumenfold: scan of {} failed: {err}", root.display());
                    walked_all = false;
                }
            }
        }

        let mut guard = state.lock().expect("library state poisoned");
        if guard.scan_generation != generation {
            // A newer scan already owns the state; this one just exits.
            return;
        }

        // 裁剪只在**每个**目录都走完时做。任何一个被取消或失败时，那个目录下的
        // 文件还没有被完整看到，这时候按 seen 裁剪等于把它们删掉。
        if walked_all {
            guard.retain_ids(&seen_all);
        }
        guard.cancelled = !walked_all;
        // 存下来是给界面看的，也是给下一次扫描看的：用户下次打开图库时
        // 能拿这一行跟自己「等了很久」的印象对一下。
        guard.scan_reused = reused;
        guard.scan_probed = probed;

        // Walk order is not path order, so the final list is sorted once and
        // the id index rebuilt to match. A cancelled walk is sorted and indexed
        // too — the partial result is what the user gets to keep looking at.
        guard.assets.sort_by(|a, b| a.path.cmp(&b.path));
        guard.reindex();
        guard.scanning = false;
        guard.scan_cancel = None;
    });
}

/// Asks the scan in flight to stop. Returns `true` if there was one.
///
/// 取消是协作式的：这里只把标志位置起来，扫描线程在读到下一个文件时自己退出。
/// 已经扫到的记录保留——半张图库也比空图库有用——但 `cancelled` 会置位，
/// 所以界面可以告诉你「是你叫停的」，而不是假装扫完了。
pub fn cancel_scan<R: Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.state::<SharedLibrary>();
    let guard = state.lock().expect("library state poisoned");
    match &guard.scan_cancel {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// `library.json` in the Tauri config dir; holds the last opened folder.
fn config_file<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, tauri::Error> {
    Ok(app.path().app_config_dir()?.join("library.json"))
}

/// Shape of `library.json`. Unknown fields are ignored on load.
///
/// # 为什么同时留着 `root`
///
/// 旧版本写的是 `{"root": "D:\\Photos"}`。已经配好图库的人不该因为这次升级
/// 被要求重选一遍，所以两个字段都收：`roots` 非空就用它，否则回退到单项的 `root`。
/// 写出去的时候只有 `roots`，`root` 留 `None` 并被跳过——迁移是单向的。
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct LibraryConfig {
    /// 图库目录，按用户排定的顺序。
    #[serde(default)]
    roots: Vec<String>,
    /// 旧版的单个根目录，仅在 `roots` 为空时生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root: Option<String>,
}

impl LibraryConfig {
    /// 配置里的目录列表，兼容旧版的单根形状。
    fn roots(&self) -> Vec<PathBuf> {
        if !self.roots.is_empty() {
            return self.roots.iter().map(PathBuf::from).collect();
        }
        self.root.iter().map(PathBuf::from).collect()
    }
}

/// Persists the whole root set so the next launch can reopen it.
pub fn persist_roots<R: Runtime>(app: &AppHandle<R>, roots: &[PathBuf]) -> Result<(), LumenError> {
    let file = config_file(app).map_err(|err| LumenError::io(err.to_string()))?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            LumenError::io(format!("cannot create {}: {err}", parent.display()))
        })?;
    }
    let body = serde_json::to_string_pretty(&LibraryConfig {
        roots: roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect(),
        root: None,
    })
    .map_err(|err| LumenError::io(err.to_string()))?;
    std::fs::write(&file, body)
        .map_err(|err| LumenError::io(format!("cannot write {}: {err}", file.display())))
}

/// Reopens and rescans every folder recorded in `library.json`.
///
/// A missing file, unparsable JSON, or a folder that has since been deleted are
/// all "nothing to restore": startup must not fail because of them.
///
/// 目录是**逐个**过滤的，不是一票否决：挂过四个目录、其中一块移动硬盘现在没插，
/// 剩下三个仍然该被打开。整份配置作废会让用户每次拔盘都要重配一次图库。
pub fn restore<R: Runtime>(app: &AppHandle<R>) {
    let Ok(file) = config_file(app) else { return };
    let Ok(body) = std::fs::read_to_string(&file) else { return };
    let Ok(config) = serde_json::from_str::<LibraryConfig>(&body) else { return };

    let roots: Vec<PathBuf> = config
        .roots()
        .into_iter()
        .filter(|root| {
            if root.is_dir() {
                true
            } else {
                eprintln!(
                    "lumenfold: saved library {} no longer exists; skipping it",
                    root.display()
                );
                false
            }
        })
        .collect();

    if roots.is_empty() {
        return;
    }
    start_scan(app, roots);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, name: &str, bytes: u64, taken: Option<i64>) -> AssetRecord {
        AssetRecord {
            id: id.to_string(),
            path: PathBuf::from(format!("/lib/{name}")),
            file_name: name.to_string(),
            bytes,
            mtime_secs: 0,
            width: 100,
            height: 50,
            taken_at: taken,
            // 夹具直接标成「已分类」：这些用例关心的是排序/分页/检索，
            // 不想被「还没分类」那条增量重读的规则牵连。
            tags: lumenfold_index::TAG_CLASSIFIED,
        }
    }

    fn state_with(records: Vec<AssetRecord>) -> LibraryState {
        let mut state = LibraryState::empty();
        state.roots = vec![PathBuf::from("/lib")];
        for record in records {
            state.push(record);
        }
        state
    }

    #[test]
    fn query_sorts_and_pages() {
        let state = state_with(vec![
            record("a", "a.jpg", 10, Some(300)),
            record("b", "b.jpg", 30, None),
            record("c", "c.jpg", 20, Some(100)),
        ]);

        let ids = |query: &AssetQuery| -> Vec<String> {
            state.query(query).into_iter().map(|row| row.id).collect()
        };

        // Null dates sort last in both directions.
        assert_eq!(
            ids(&AssetQuery { sort: "taken-desc".into(), ..Default::default() }),
            vec!["a", "c", "b"]
        );
        assert_eq!(
            ids(&AssetQuery { sort: "taken-asc".into(), ..Default::default() }),
            vec!["c", "a", "b"]
        );
        assert_eq!(
            ids(&AssetQuery { sort: "size".into(), ..Default::default() }),
            vec!["b", "c", "a"]
        );
        assert_eq!(
            ids(&AssetQuery { sort: "name".into(), ..Default::default() }),
            vec!["a", "b", "c"]
        );
        // An unknown sort key falls back to taken-desc.
        assert_eq!(
            ids(&AssetQuery { sort: "nonsense".into(), ..Default::default() }),
            vec!["a", "c", "b"]
        );
        // Paging.
        assert_eq!(
            ids(&AssetQuery {
                sort: "taken-desc".into(),
                offset: 1,
                limit: 1,
                ..Default::default()
            }),
            vec!["c"]
        );
    }

    #[test]
    fn query_reports_derived_geometry_and_empty_user_fields() {
        let state = state_with(vec![record("a", "a.jpg", 10, Some(300))]);
        let row = &state.query(&AssetQuery::default())[0];
        assert_eq!(row.orientation, "landscape");
        assert_eq!(row.rating, 0);
        assert!(!row.flagged);
        assert_eq!(row.taken_at, Some(300));
        assert_eq!(row.path, "/lib/a.jpg");
    }

    #[test]
    fn metadata_never_invents_exif() {
        let state = state_with(vec![record("a", "a.jpg", 10, Some(300))]);
        let meta = state.metadata("a").expect("metadata");
        assert_eq!(meta.asset_id, "a");
        assert!(meta.camera_make.is_none());
        assert!(meta.camera_model.is_none());
        assert!(meta.lens.is_none());
        assert!(meta.iso.is_none());
        assert!(meta.shutter.is_none());
        assert!(meta.aperture.is_none());
        assert!(meta.focal_length_mm.is_none());
        assert!(meta.keywords.is_empty());
        assert!(meta.caption.is_none());
        assert!(meta.gps_lat.is_none());
        assert!(meta.gps_lon.is_none());
        assert!(state.metadata("missing").is_err());
    }

    /// 状态栏里的缓存字节数是给用户看的，不能随时间往上飘。
    /// 淘汰出来的条目必须真的从总数里减掉，否则这个数字会慢慢变成一句谎话。
    #[test]
    fn cache_bytes_track_inserts_replacements_and_evictions() {
        let mut state = LibraryState::empty();
        let thumb = |n: u8| CachedThumb {
            bytes: Arc::new(vec![0u8; n as usize]),
            content_type: "image/jpeg",
        };

        state.put_thumb("a".into(), thumb(100));
        assert_eq!(state.stats().cache_bytes, 100);

        // 同一个 key 换一张：旧的不该留在总数里。
        state.put_thumb("a".into(), thumb(40));
        assert_eq!(state.stats().cache_bytes, 40);

        state.put_thumb("b".into(), thumb(60));
        assert_eq!(state.stats().cache_bytes, 100);

        state.clear_thumbs();
        assert_eq!(state.stats().cache_bytes, 0);
    }

    #[test]
    fn stats_track_scan_state_and_cache_bytes() {
        let mut state = LibraryState::empty();
        assert_eq!(state.stats().asset_count, 0);
        assert!(!state.stats().indexed);
        assert!(!state.stats().scanning);
        assert!(state.stats().roots.is_empty());

        state.scanning = true;
        state.roots = vec![PathBuf::from("/lib")];
        state.push(record("a", "a.jpg", 10, Some(300)));
        assert_eq!(state.stats().asset_count, 1);
        assert!(state.stats().scanning);
        assert!(state.stats().indexed);
        assert_eq!(state.stats().roots, vec!["/lib".to_string()]);
    }

    /// 文本检索必须真的比出东西来。
    ///
    /// 之前 `search` 直接回一句 `not_indexed`，搜索页永远是红色的失败框。
    /// 这一条守着「用户输入文件名就能找到」这个契约。
    #[test]
    fn search_text_matches_filenames_case_insensitively() {
        let state = state_with(vec![
            record("a", "IMG_4821.jpg", 10, Some(300)),
            record("b", "sunset-beach.jpg", 20, Some(200)),
            record("c", "IMG_9999.jpg", 30, Some(100)),
        ]);

        let ids = |q: &str, limit: u32| -> Vec<String> {
            state
                .search_text(q, limit)
                .into_iter()
                .map(|hit| hit.asset.id)
                .collect()
        };

        // 大小写不敏感：没人会照着磁盘上的大小写去敲。
        assert_eq!(ids("img_4821", 10), vec!["a"]);
        assert_eq!(ids("IMG_", 10), vec!["a", "c"]);
        assert_eq!(ids("beach", 10), vec!["b"]);
        // 没有命中是空结果，不是错误。
        assert!(ids("nothing-like-this", 10).is_empty());
    }

    /// 排名表达的是「更可能是想要的那张在前面」，不冒充相关性计算。
    #[test]
    fn search_text_ranks_exact_then_prefix_then_substring() {
        let state = state_with(vec![
            record("sub", "my-IMG_4821-backup.jpg", 10, Some(100)),
            record("prefix", "IMG_4821-edit.jpg", 20, Some(100)),
            record("exact", "IMG_4821.jpg", 30, Some(100)),
        ]);

        // 连扩展名一起敲时独占结果：另外两张的文件名里没有这一整串。
        let ids: Vec<String> = state
            .search_text("img_4821.jpg", 10)
            .into_iter()
            .map(|hit| hit.asset.id)
            .collect();
        assert_eq!(ids, vec!["exact"]);

        // 只敲主干时三张都比中，名次完全由打分决定。
        let hits = state.search_text("img_4821", 10);
        let score_of = |id: &str| {
            hits.iter()
                .find(|hit| hit.asset.id == id)
                .map(|hit| hit.score)
                .unwrap_or_default()
        };
        // `IMG_4821.jpg` 在这里也是**前缀**命中，不是整体相等——整体相等要求
        // 文件名与查询逐字相同，而查询里没有扩展名。
        assert_eq!(score_of("exact"), 2.0, "IMG_4821.jpg 是前缀命中");
        assert_eq!(score_of("prefix"), 2.0, "IMG_4821-edit.jpg 也是前缀命中");
        assert_eq!(score_of("sub"), 1.0, "my-IMG_4821-backup.jpg 只是子串命中");

        // 于是顺序是确定的：两个 2.0 在前（拍摄时间相同，按路径排），1.0 在最后。
        let ids: Vec<String> = hits.into_iter().map(|hit| hit.asset.id).collect();
        assert_eq!(ids, vec!["prefix", "exact", "sub"]);
    }

    /// 关键词表还没建，但目录名是扫描时就知道的，所以路径也参与匹配。
    #[test]
    fn search_text_falls_back_to_the_directory_name() {
        let state = state_with(vec![record("a", "a.jpg", 10, Some(300))]);
        // `record()` 把每张照片都放在 /lib/ 下。
        let hits = state.search_text("lib", 10);
        assert_eq!(hits.len(), 1, "目录名命中应该被算进来");
        assert_eq!(hits[0].asset.id, "a");
        // 命中路径的分数低于命中文件名，排序时才不会盖过真正的文件名命中。
        assert!(hits[0].score < 1.0);
    }

    /// 空词不是错误，是「没什么可查」；上限来自前端，必须真的生效。
    #[test]
    fn search_text_is_empty_for_a_blank_query_and_honours_the_limit() {
        let state = state_with(vec![
            record("a", "p1.jpg", 10, Some(300)),
            record("b", "p2.jpg", 20, Some(200)),
            record("c", "p3.jpg", 30, Some(100)),
        ]);

        assert!(state.search_text("", 10).is_empty());
        assert!(state.search_text("   ", 10).is_empty());
        assert_eq!(state.search_text("p", 2).len(), 2);
    }

    /// 同分时新拍的在前：搜同一个词，用户更可能想先看最近那张。
    #[test]
    fn search_text_prefers_the_newer_photo_on_a_tie() {
        let state = state_with(vec![
            record("old", "beach-1.jpg", 10, Some(100)),
            record("new", "beach-2.jpg", 20, Some(900)),
        ]);
        let ids: Vec<String> = state
            .search_text("beach", 10)
            .into_iter()
            .map(|hit| hit.asset.id)
            .collect();
        assert_eq!(ids, vec!["new", "old"]);
    }

    /// 类别筛选必须发生在排序和分页**之前**。
    ///
    /// 反过来的话 offset 会落在已经被滤掉的行上，翻页就会一页多一页少——
    /// 而且用户看到的空缺还会随排序变化而移动。
    #[test]
    fn query_filters_by_tag_before_paging() {
        let mut state = LibraryState::empty();
        state.roots = vec![PathBuf::from("/lib")];
        let mut panoramic = record("p", "pano.jpg", 10, Some(300));
        panoramic.tags |= lumenfold_index::TAG_PANORAMA;
        let mut animated = record("g", "loop.gif", 20, Some(200));
        animated.tags |= lumenfold_index::TAG_ANIMATED;
        state.push(panoramic);
        state.push(animated);

        let ids = |tag: Option<&str>| -> Vec<String> {
            state
                .query(&AssetQuery {
                    tag: tag.map(str::to_string),
                    ..Default::default()
                })
                .into_iter()
                .map(|row| row.id)
                .collect()
        };

        assert_eq!(ids(None).len(), 2, "不筛时两张都在");
        assert_eq!(ids(Some("panorama")), vec!["p"]);
        assert_eq!(ids(Some("animated")), vec!["g"]);
        assert_eq!(ids(Some("raw")).len(), 0, "这一类里确实没有东西");
        // 认不出的类别 id：零张，而不是把整库端出去。
        assert!(ids(Some("nonsense")).is_empty());
    }

    /// 计数要能说明「还有多少张没分过类」。
    ///
    /// 没有 `classified` / `total` 这一对，界面就只能显示「动图 0 张」——
    /// 那是把「还没数」说成「没有」。
    #[test]
    fn tag_counts_report_classification_progress() {
        let mut state = LibraryState::empty();
        state.roots = vec![PathBuf::from("/lib")];
        let mut animated = record("g", "loop.gif", 20, Some(200));
        animated.tags |= lumenfold_index::TAG_ANIMATED;
        state.push(animated);
        // 这一条模拟老快照：tags 是 0，还没被分类过。
        let mut stale = record("s", "old.jpg", 30, Some(100));
        stale.tags = 0;
        state.push(stale);

        let counts = state.tag_counts();
        assert_eq!(counts.len(), lumenfold_index::FILTERABLE_TAGS.len());
        for row in &counts {
            assert_eq!(row.total, 2, "总数不分是否分过类");
            assert_eq!(row.classified, 1, "两条里只有一条真的分过类");
        }

        let animated = counts.iter().find(|row| row.id == "animated").expect("animated");
        assert_eq!(animated.count, 1);
        let raw = counts.iter().find(|row| row.id == "raw").expect("raw");
        assert_eq!(raw.count, 0);
        assert_eq!(raw.total, 2, "别的类别也要带上总数，界面才说得出「还在补齐」");
    }

    /// 旧版的 `{"root": "..."}` 必须还能读出来。
    ///
    /// 读不出来的话，升级之后每个老用户下次启动都会看到「还没有配过图库」，
    /// 而他们其实什么都没做错——这是最容易发生、也最容易被漏掉的一种回归。
    #[test]
    fn library_config_still_reads_the_single_root_shape() {
        let old: LibraryConfig =
            serde_json::from_str(r#"{"root":"D:\\Photos"}"#).expect("旧形状要能读");
        assert_eq!(old.roots(), vec![PathBuf::from("D:\\Photos")]);

        let new: LibraryConfig =
            serde_json::from_str(r#"{"roots":["A","B"]}"#).expect("新形状");
        assert_eq!(new.roots(), vec![PathBuf::from("A"), PathBuf::from("B")]);

        // 两个字段都在时以 `roots` 为准：迁移是单向的，新字段赢。
        let both: LibraryConfig =
            serde_json::from_str(r#"{"roots":["A"],"root":"B"}"#).expect("两个都在");
        assert_eq!(both.roots(), vec![PathBuf::from("A")]);
    }

    /// 嵌套的两个图库目录会把同一张照片记两次，合并时必须按 id 折叠。
    ///
    /// 不折叠的话同一个 id 在 `assets` 里出现两次，`index` 只记住后一个，
    /// 前一个成了永远查不到、却仍然占着网格位置的幽灵。
    #[test]
    fn duplicate_ids_collapse_when_merging_roots() {
        let merged = dedupe_by_id(vec![
            record("a", "a.jpg", 10, Some(1)),
            record("b", "b.jpg", 20, Some(2)),
            record("a", "a.jpg", 10, Some(1)),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "a");
        assert_eq!(merged[1].id, "b");
    }
}
