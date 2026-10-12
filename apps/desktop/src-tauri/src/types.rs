//! Types exchanged over the IPC boundary.
//!
//! Field names are camelCased on the wire (`#[serde(rename_all = "camelCase")]`)
//! so the TypeScript definitions in `apps/desktop/src` can mirror them verbatim.

use serde::{Deserialize, Serialize};

/// Version handshake returned by `app_info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// Crate version of the running backend.
    pub version: String,
    /// Bumped whenever the command surface changes incompatibly.
    pub api_version: u32,
}

/// Library-wide counters shown on the home view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    /// Number of assets in the index; `0` while `indexed` is false.
    pub asset_count: u64,
    /// Whether a usable index exists on disk.
    pub indexed: bool,
    /// 作为图库的目录，按用户排定的顺序。
    ///
    /// 以前这里是 `root: Option<String>`——图库只有一个根，换位置就是把它换掉。
    /// 现在图库是一组目录（像知识库那样可以挂多个来源），所以是一个列表。
    /// 列表为空表示还没配过图库。
    pub roots: Vec<String>,
    /// Bytes currently occupied by caches (thumbnails, embeddings, ...).
    ///
    /// 内存 LRU 与磁盘缩略图目录都算在内——两层确实各占一份字节，
    /// 报一个更小的数才是谎话。
    pub cache_bytes: u64,
    /// Ceiling the caches may grow to before they start evicting.
    ///
    /// 之前设置页写死「上限默认 10GB」，而宿主里根本没有任何上限逻辑，
    /// 10GB 这个数字只是一句没人兑现的文案。这个字段让「上限」成为
    /// 宿主真的会执行、也会在设置页被改写的真实约束。
    pub cache_limit_bytes: u64,
    /// Whether a scan is running; `asset_count` grows while it is `true`.
    pub scanning: bool,
    /// The most recent scan was stopped by the user rather than finishing.
    ///
    /// 取消不是失败：`asset_count` 停在当时已扫到的数量，索引仍然可用。
    /// 界面靠它区分「扫完了」和「你自己叫停的」，否则点完取消和扫完看起来一模一样。
    pub cancelled: bool,
    /// Files the most recent walk served straight from the persisted index.
    ///
    /// 这个数存在就是为了能被看见。「增量扫描」如果只写在注释里，用户无法
    /// 区分它到底跑没跑；把它摆在状态栏上，下次打开图库就能一眼看出
    /// 12,340 张里只有 12 张被重新打开过。
    pub scan_reused: u64,
    /// Files the most recent walk had to open and probe because size or mtime moved.
    pub scan_probed: u64,
}

/// Grid/list tile projection of a single asset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
    /// Stable identifier (content hash) used everywhere else.
    pub id: String,
    /// Absolute path on disk.
    pub path: String,
    /// File name including extension.
    pub file_name: String,
    /// Pixel width, `0` when unknown.
    pub width: u32,
    /// Pixel height, `0` when unknown.
    pub height: u32,
    /// File size in bytes.
    pub bytes: u64,
    /// Capture time as Unix seconds, when the file carries one.
    pub taken_at: Option<i64>,
    /// Star rating, `0`..=5.
    pub rating: u8,
    /// Whether the user flagged the asset.
    pub flagged: bool,
    /// Normalized orientation, e.g. `landscape` / `portrait`.
    pub orientation: String,
    /// 类别标记位，见 `lumenfold_index::classify`。
    ///
    /// 位定义（`lumenfold_index` 里的 `TAG_*`）：动图 / 截图 / RAW / 全景，
    /// 最高位是「已经分过类」。前端只做位测试，不自己发明类别。
    pub tags: u32,
}

/// 一个类别当前有多少张照片。
///
/// 只回 id 和数量，中文名由前端自己给：宿主返回界面文案会把翻译绑死在
/// Rust 里，而这个项目所有面向用户的字符串都在 `apps/desktop/src` 下。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetTagCount {
    /// Wire id, one of `lumenfold_index::FILTERABLE_TAGS`'s names.
    pub id: String,
    /// How many indexed assets carry this tag.
    pub count: u64,
    /// How many indexed assets have been classified at all.
    ///
    /// 老快照里的记录 tags 是 0（还没分类）。这个数让界面能说清楚
    /// 「分类还在补齐」而不是让用户以为某个类别真的是 0 张。
    pub classified: u64,
    /// Total indexed assets, classified or not.
    ///
    /// 和 `classified` 一起用：两者不等就说明还有记录没分过类。
    pub total: u64,
}

/// Pixel transforms requested by the Lightbox editor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageTransform {
    /// Clockwise quarter turns. The frontend only sends multiples of 90.
    pub rotate: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// Whether an edit replaces the source file or creates a sibling copy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditMode {
    Overwrite,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditRequest {
    pub asset_id: String,
    pub transform: ImageTransform,
    pub mode: EditMode,
    /// 在几何变换之后再做一次画质增强（超分两倍再缩回原尺寸）。
    ///
    /// `#[serde(default)]` 让不带这个字段的请求仍然能反序列化：少写就等于「不增强」，
    /// 而不是让整条编辑请求失败。
    #[serde(default)]
    pub enhance: bool,
}

/// Capture metadata for a single asset, shown in the inspector panel.
///
/// Every field is optional because any given file may omit any given tag.
/// `#[derive(Default)]` yields empty `keywords` and `None` for the rest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetMetadata {
    /// Asset this metadata belongs to.
    pub asset_id: String,
    /// EXIF `Make`.
    pub camera_make: Option<String>,
    /// EXIF `Model`.
    pub camera_model: Option<String>,
    /// EXIF lens identification.
    pub lens: Option<String>,
    /// ISO sensitivity.
    pub iso: Option<u32>,
    /// Shutter speed exactly as written in the file, e.g. `"1/250s"`.
    pub shutter: Option<String>,
    /// Aperture as an f-number, e.g. `1.8`.
    pub aperture: Option<f32>,
    /// Focal length in millimetres.
    pub focal_length_mm: Option<u32>,
    /// Keywords read from IPTC/XMP or applied by the user.
    pub keywords: Vec<String>,
    /// IPTC/XMP caption or description.
    pub caption: Option<String>,
    /// Latitude in signed decimal degrees.
    pub gps_lat: Option<f64>,
    /// Longitude in signed decimal degrees.
    pub gps_lon: Option<f64>,
}

/// Paging, sorting and filter selection for `query_assets`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetQuery {
    /// Maximum rows to return.
    pub limit: u32,
    /// Rows to skip; combined with `limit` for pagination.
    pub offset: u32,
    /// Sort key: `taken-desc`, `taken-asc`, `name`, `size` or `rating`.
    pub sort: String,
    /// Restrict to a single timeline bucket (year, month, album, ...).
    pub bucket: Option<String>,
    /// Restrict to assets containing faces of one person.
    pub person_id: Option<String>,
    /// Restrict to one category, by wire id from `lumenfold_index::FILTERABLE_TAGS`
    /// (`animated` / `screenshot` / `raw` / `panorama`). `None` means no filter.
    ///
    /// 未知 id 不是错误也不是「不过滤」：它会让查询返回空结果，因为
    /// 「按一个不存在的类别筛选」的真实答案就是零张，而不是把整库端出去。
    pub tag: Option<String>,
}

impl Default for AssetQuery {
    fn default() -> Self {
        Self {
            limit: 100,
            offset: 0,
            sort: "taken-desc".to_string(),
            bucket: None,
            person_id: None,
            tag: None,
        }
    }
}

/// A clustered person in the face index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonSummary {
    /// Stable person identifier.
    pub id: String,
    /// User-assigned name, when set.
    pub name: Option<String>,
    /// Asset used as the avatar in the UI.
    pub cover_asset_id: Option<String>,
    /// Number of detected faces for this person.
    pub face_count: u32,
    /// Number of distinct assets this person appears in.
    pub asset_count: u32,
}

/// Query payload for `search`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    /// Raw user query.
    pub query: String,
    /// `text` for metadata/FTS matching, `semantic` for embedding similarity.
    pub mode: String,
    /// Maximum number of hits.
    pub limit: u32,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            mode: "text".to_string(),
            limit: 50,
        }
    }
}

/// One scored search hit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// The matching asset.
    pub asset: AssetSummary,
    /// Relevance score; higher is better, not comparable across modes.
    pub score: f64,
}

/// What a catalog model is actually for.
///
/// This is a closed enum on purpose. The UI has to tell "the face recognition
/// model is missing" apart from "the face detector is missing", and a free-form
/// `task` string cannot answer that without guessing — matching on text would
/// read CLIP's image encoder as a face recognizer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ModelCapability {
    /// Encodes text into the CLIP shared embedding space.
    TextEmbedding,
    /// Encodes images into the CLIP shared embedding space.
    ImageEmbedding,
    /// Finds face bounding boxes. Not sufficient on its own to group faces.
    FaceDetect,
    /// Maps a face crop into an embedding that can be compared across photos.
    FaceEmbed,
    /// 画质增强：把照片送进超分/复原模型再回到原尺寸。
    ///
    /// 它和上面四个的区别是**不产生新数据**，只改已有的像素，所以它归在
    /// 「编辑」那一侧，不是一个索引能力——没有人会去「查询」一个超分模型。
    SuperResolution,
    /// Anything the catalog carries that the UI has no opinion about.
    #[default]
    Other,
}

/// One entry of the managed model catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    /// Catalog identifier, e.g. `clip-text`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Human-readable task description shown in the settings list.
    pub task: String,
    /// Machine-readable capability. Branch on this, never on `task`.
    pub capability: ModelCapability,
    /// SPDX-style license identifier.
    pub license: String,
    /// Whether the license permits commercial use.
    pub commercial_use: bool,
    /// Download size in bytes; `0` when resolved from the server at download time.
    pub size_bytes: u64,
    /// One of `absent`, `downloading`, `ready`, `error`.
    pub status: String,
    /// Bytes fetched so far; `0` unless `status` is `downloading`.
    pub downloaded_bytes: u64,
    /// Direct download URL.
    pub source_url: String,
    /// Expected SHA-256 digest; empty when not pinned in this build.
    pub sha256: String,
}
/// What a batch action does to the selected files on disk.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BatchAction {
    /// Duplicate the files into the chosen folder.
    Copy,
    /// Move the files into the chosen folder.
    Move,
    /// Delete the files. Needs a destination? No — it is the only action that
    /// does not, which is why the UI has to confirm it separately.
    Delete,
}

impl BatchAction {
    /// Wire value used by the frontend.
    pub const fn as_str(self) -> &'static str {
        match self {
            BatchAction::Copy => "copy",
            BatchAction::Move => "move",
            BatchAction::Delete => "delete",
        }
    }

    /// Whether the action needs the user to pick a target folder.
    pub const fn needs_destination(self) -> bool {
        !matches!(self, BatchAction::Delete)
    }
}

/// Request for `batch_operate`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchRequest {
    /// Asset ids marked in the UI.
    pub asset_ids: Vec<String>,
    /// Which operation to perform.
    pub action: BatchAction,
    /// Target folder. Ignored by [`BatchAction::Delete`], and required by the
    /// other two — the host rejects it rather than guessing a folder.
    pub destination: Option<String>,
}

/// Per-asset outcome of a batch action.
///
/// 批量操作必须是逐条的成败，不能只回一个总数：
/// 目标盘满了、某一张正被别的程序占用，用户要知道是哪几张没成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchItemResult {
    pub asset_id: String,
    pub file_name: String,
    /// `true` when the file was copied / moved / deleted.
    pub ok: bool,
    /// Why it failed, empty on success.
    pub message: String,
    /// Where the file ended up. `None` for deletes.
    pub path: Option<String>,
}

/// Aggregate result of a batch action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchReport {
    pub items: Vec<BatchItemResult>,
    pub succeeded: u32,
    pub failed: u32,
    /// Folder the files went to, when one was chosen.
    pub destination: Option<String>,
}

impl BatchReport {
    /// Builds a report from per-item outcomes.
    pub fn from_items(items: Vec<BatchItemResult>, destination: Option<String>) -> Self {
        let succeeded = items.iter().filter(|item| item.ok).count() as u32;
        let failed = items.len() as u32 - succeeded;
        Self { items, succeeded, failed, destination }
    }
}
