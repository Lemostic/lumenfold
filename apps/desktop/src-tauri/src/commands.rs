//! The IPC command surface.
//!
//! The library commands are backed by a real scan: `open_library` points the
//! app at a folder, a background walk fills the state, and `library_stats` /
//! `query_assets` / `asset_metadata` read it. Commands that still have no
//! implementation behind them (people clustering) fail with
//! [`LumenError::NotIndexed`] rather than returning empty results, so the UI can
//! tell "nothing indexed yet" apart from "no matches". The model registry and
//! its downloader live in [`crate::models`], and those are implemented.

use std::path::PathBuf;
use std::io::Cursor;

use tauri::{AppHandle, Manager, Runtime, State};

use crate::error::LumenError;
use crate::library::{self, SharedLibrary};
use crate::discover::DiscoveryReport;
use crate::types::{
    AppInfo, AssetMetadata, AssetQuery, AssetSummary, AssetTagCount, BatchReport, BatchRequest,
    LibraryStats, EditMode, EditRequest, PersonSummary, SearchRequest, SearchResult,
};

/// Version of the command surface. Bump on any breaking signature change.
const API_VERSION: u32 = 1;

/// Upper bound on one search response.
///
/// 前端目前要 400 条；给一个宽裕的上限只是为了让「前端传了什么」不至于变成
/// 宿主的资源风险，而不是替前端定策略。
const MAX_SEARCH_LIMIT: u32 = 2000;

/// Reports the backend version and the IPC contract version it speaks.
#[tauri::command]
pub async fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        api_version: API_VERSION,
    }
}

/// Library counters. `asset_count` grows while `scanning` is `true`.
///
/// Tauri v2 requires every `async` command to return a `Result`; reading the
/// mutex cannot actually fail, so the error arm is unreachable in practice.
#[tauri::command]
pub async fn library_stats<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedLibrary>,
) -> Result<LibraryStats, LumenError> {
    Ok(with_cache_totals(&app, state.lock().expect("library state poisoned").stats()))
}

/// Folds the on-disk thumbnail store into the counters the state keeps in memory.
///
/// 图库状态只知道自己那份内存 LRU；磁盘目录是另一把锁、另一个模块。
/// 界面只有一个轮询口（`library_stats`），所以两份加起来在这里汇合，
/// 而不是让前端再去轮询一次只为拿一个「上限」。
fn with_cache_totals<R: Runtime>(app: &AppHandle<R>, mut stats: LibraryStats) -> LibraryStats {
    let store = app.state::<crate::thumbstore::ThumbStore>();
    stats.cache_bytes = stats.cache_bytes.saturating_add(store.bytes());
    stats.cache_limit_bytes = store.limit_bytes();
    stats
}

/// Sets how much disk the thumbnail cache may use, and persists the choice.
///
/// 下限由宿主兜住，不靠前端：写进来一个 0 或者 10 GB 都不该被照单全收。
#[tauri::command]
pub async fn set_thumb_cache_limit<R: Runtime>(
    app: AppHandle<R>,
    limit_bytes: u64,
) -> Result<LibraryStats, LumenError> {
    let store = app.state::<crate::thumbstore::ThumbStore>();
    // 夹逼在命令边界上做，store 自己不管产品下限——它只知道怎么淘汰。
    let limit = crate::thumbstore::clamp_limit(limit_bytes);
    store.set_limit_bytes(limit);
    crate::thumbstore::save_limit(&app, store.limit_bytes())?;
    Ok(with_cache_totals(
        &app,
        app.state::<SharedLibrary>()
            .lock()
            .expect("library state poisoned")
            .stats(),
    ))
}

/// Deletes every cached thumbnail from disk.
///
/// 单独给一个动作，而不是让用户「把上限调到 1 GB」来间接清空：
/// 后者做不到真正清空（只会淘汰到 10%），而且用户要的从来不是「更小的缓存」。
#[tauri::command]
pub async fn clear_thumb_cache<R: Runtime>(
    app: AppHandle<R>,
) -> Result<LibraryStats, LumenError> {
    app.state::<crate::thumbstore::ThumbStore>().clear();
    Ok(with_cache_totals(
        &app,
        app.state::<SharedLibrary>()
            .lock()
            .expect("library state poisoned")
            .stats(),
    ))
}

/// Opens `path` as the library and starts a background scan.
///
/// 这是「换个位置」那条路：它把图库**替换成**这一个目录。要挂着多个目录
/// 请用 [`set_library_roots`]。
///
/// Returns immediately with `scanning: true`; the UI polls
/// [`library_stats`] until it flips to `false`.
#[tauri::command]
pub async fn open_library<R: Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<LibraryStats, LumenError> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(LumenError::io(format!(
            "{} is not an existing folder",
            root.display()
        )));
    }
    // 目录列表的落盘在 `start_scan` 里做，在真正开扫之前——扫到一半崩了
    // 也不该丢掉用户刚配好的列表。
    library::start_scan(&app, vec![root]);
    Ok(current_stats(&app))
}

/// 把界面给的目录列表收拾干净：丢掉不存在的、去掉重复项，保留用户排的顺序。
///
/// 不在这里做嵌套折叠（`D:\P` 和 `D:\P\2024` 都留着）：用户可能有意把某个
/// 子目录单独挂上来，而扫描那边的按 id 去重已经把重复计数解决掉了。
fn normalise_roots(roots: &[String]) -> Result<Vec<PathBuf>, LumenError> {
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in roots {
        let path = PathBuf::from(raw);
        if !path.is_dir() {
            return Err(LumenError::io(format!(
                "{} 不是一个存在的目录",
                path.display()
            )));
        }
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// Replaces the whole set of library folders and rescans.
///
/// 增、删、改顺序都走这一条：图库目录是一个整体，分开做「加一个」「删一个」
/// 会让并发调用有机会插在中间，落下一个谁也没打算要的中间态。
#[tauri::command]
pub async fn set_library_roots<R: Runtime>(
    app: AppHandle<R>,
    roots: Vec<String>,
) -> Result<LibraryStats, LumenError> {
    let paths = normalise_roots(&roots)?;
    library::start_scan(&app, paths);
    Ok(current_stats(&app))
}

/// 自动发现：走 `base` 下面最多 `max_depth` 层，列出所有含照片的目录。
///
/// 只发现、不采纳。采纳哪些由用户在设置页勾选后交给 [`set_library_roots`]。
#[tauri::command]
pub async fn discover_libraries(
    state: State<'_, SharedLibrary>,
    base: String,
    max_depth: u32,
) -> Result<DiscoveryReport, LumenError> {
    let existing = state
        .lock()
        .expect("library state poisoned")
        .roots
        .clone();
    let base = PathBuf::from(base);
    // 纯 IO，可能走几十万个条目：绝不能占着异步运行时的线程，
    // 否则同期所有命令一起排队，界面看起来就是卡死。
    tauri::async_runtime::spawn_blocking(move || {
        crate::discover::discover(&base, max_depth, &existing)
    })
    .await
    .map_err(|err| LumenError::io(format!("发现任务异常结束: {err}")))?
}

/// Rescans every configured library folder.
#[tauri::command]
pub async fn rescan<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedLibrary>,
) -> Result<LibraryStats, LumenError> {
    let roots = state
        .lock()
        .expect("library state poisoned")
        .roots
        .clone();
    if roots.is_empty() {
        return Err(LumenError::no_library(
            "no library folder is open; call open_library first",
        ));
    }
    library::start_scan(&app, roots);
    Ok(current_stats(&app))
}

/// Snapshots the counters. Kept out of the command so a caller holding an
/// `AppHandle` can read the stats without building a `State` wrapper.
fn current_stats<R: Runtime>(app: &AppHandle<R>) -> LibraryStats {
    with_cache_totals(
        app,
        app.state::<SharedLibrary>()
            .lock()
            .expect("library state poisoned")
            .stats(),
    )
}

/// Stops the scan in flight, keeping whatever it found so far.
///
/// 取消不是一个「什么都没发生」的动作：半个图库照样能用。
/// `LibraryStats.cancelled` 让界面把「你叫停的」和「扫完了」区分开。
#[tauri::command]
pub async fn cancel_scan(app: AppHandle) -> Result<LibraryStats, LumenError> {
    library::cancel_scan(&app);
    Ok(current_stats(&app))
}

/// Copies, moves or trashes a set of marked assets, then re-indexes.
///
/// 一次批量操作之后必须重扫：asset id 是路径的哈希，移动和删除都会让路径变，
/// 继续用旧索引的话这批照片会以「文件不存在」的形式留在图库里。
/// 只有真正动过文件的操作才触发重扫，全失败时省掉这一轮。
#[tauri::command]
pub async fn batch_operate(
    app: AppHandle,
    state: State<'_, SharedLibrary>,
    request: BatchRequest,
) -> Result<BatchReport, LumenError> {
    let (roots, entries) = {
        let guard = state.lock().expect("library state poisoned");
        if guard.roots.is_empty() {
            return Err(LumenError::no_library("还没有打开图库"));
        }
        let roots = guard.roots.clone();
        let entries = resolve_entries(&guard, &request.asset_ids);
        (roots, entries)
    };

    let report = crate::files::operate(
        &entries,
        request.action,
        request.destination.as_deref().map(PathBuf::from).as_deref(),
        &roots,
    )?;

    if report.succeeded > 0 {
        library::start_scan(&app, roots);
    }
    Ok(report)
}

/// Turns asset ids into real paths using the live index.
///
/// The index is a `HashMap<id, position>` into the record vector, so this is a
/// pair of lookups rather than a walk. Unknown ids are reported as failures
/// rather than dropped, because a batch where the user picked something that
/// vanished under their feet must say so instead of quietly doing less.
fn resolve_entries(
    guard: &crate::library::LibraryState,
    asset_ids: &[String],
) -> Vec<crate::files::BatchEntry> {
    asset_ids
        .iter()
        .map(|asset_id| {
            let entry = guard
                .index
                .get(asset_id)
                .and_then(|position| guard.assets.get(*position))
                .map(|record| crate::files::BatchEntry {
                    asset_id: asset_id.clone(),
                    path: record.path.clone(),
                    file_name: record.file_name.clone(),
                })
                .unwrap_or_else(|| crate::files::BatchEntry {
                    asset_id: asset_id.clone(),
                    // The path is never read for an id that is not in the index;
                    // `files::operate` reports the missing file per item.
                    path: PathBuf::from(asset_id),
                    file_name: asset_id.clone(),
                });
            entry
        })
        .collect()
}

/// Pages through indexed assets.
#[tauri::command]
pub async fn query_assets(
    state: State<'_, SharedLibrary>,
    query: AssetQuery,
) -> Result<Vec<AssetSummary>, LumenError> {
    let guard = state.lock().expect("library state poisoned");
    if guard.roots.is_empty() {
        return Err(LumenError::no_library(
            "no library folder is open; call open_library first",
        ));
    }
    Ok(guard.query(&query))
}

/// Reads capture metadata for one asset, for the inspector panel.
///
/// Only what the scan actually read is reported: the capture date, and `None`
/// for every EXIF field the header probe does not parse.
#[tauri::command]
pub async fn asset_metadata(
    state: State<'_, SharedLibrary>,
    asset_id: String,
) -> Result<AssetMetadata, LumenError> {
    let guard = state.lock().expect("library state poisoned");
    if guard.roots.is_empty() {
        return Err(LumenError::no_library(
            "no library folder is open; call open_library first",
        ));
    }
    guard.metadata(&asset_id)
}

/// Applies a quarter-turn and/or mirror transform to one indexed image.
///
/// The transform is rendered into pixels before writing, so the result is
/// consistent across JPEG, PNG and formats whose EXIF orientation is ignored
/// by the browser. The UI confirms the destructive overwrite before this
/// command is reached.
#[tauri::command]
pub async fn edit_image(
    app: AppHandle,
    state: State<'_, SharedLibrary>,
    request: EditRequest,
) -> Result<(), LumenError> {
    let (source, root) = {
        let guard = state.lock().expect("library state poisoned");
        let position = *guard
            .index
            .get(&request.asset_id)
            .ok_or_else(|| LumenError::io(format!("unknown asset id {:?}", request.asset_id)))?;
        (guard.assets[position].path.clone(), guard.roots.clone())
    };

    let mut image = image::ImageReader::open(&source)
        .map_err(|err| LumenError::io(format!("cannot open {}: {err}", source.display())))?
        .with_guessed_format()
        .map_err(|err| LumenError::unsupported(format!("cannot guess image format: {err}")))?
        .decode()
        .map_err(|err| LumenError::unsupported(format!("cannot decode {}: {err}", source.display())))?;

    let turns = request.transform.rotate.rem_euclid(360) / 90;
    for _ in 0..turns {
        image = image.rotate90();
    }
    if request.transform.flip_h {
        image = image.fliph();
    }
    if request.transform.flip_v {
        image = image.flipv();
    }

    // 画质增强。放在几何变换**之后**：先把照片摆正，模型看到的才是最终构图。
    // 顺序反过来结果一样，但先转正可以少一次无谓的推理。
    if request.enhance {
        let model = crate::models::weight_file(&app, crate::enhance::MODEL_ID)?;
        if !model.exists() {
            return Err(LumenError::not_indexed(format!(
                "画质增强的模型还没下载（{}）。到设置页下载之后再用；照片没有被改动。",
                crate::enhance::MODEL_ID
            )));
        }
        let rgb = image.to_rgb8();
        // 推理要几分钟，绝不能占着异步运行时的线程：那会让同期的其它命令
        // 一起排队，界面看起来就是卡死。
        let enhanced = tauri::async_runtime::spawn_blocking(move || {
            crate::enhance::enhance(&rgb, &model)
        })
        .await
        .map_err(|err| LumenError::io(format!("增强任务异常结束: {err}")))??;
        image = image::DynamicImage::ImageRgb8(enhanced);
    }

    let target = match request.mode {
        EditMode::Overwrite => source.clone(),
        EditMode::Copy => copy_target(&source, &request.transform, request.enhance),
    };
    let format = image_format_for(&source);
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, format)
        .map_err(|err| LumenError::io(format!("cannot encode {}: {err}", target.display())))?;
    std::fs::write(&target, bytes.into_inner())
        .map_err(|err| LumenError::io(format!("cannot write {}: {err}", target.display())))?;
    // Re-index after either overwrite or copy so dimensions, thumbnails and
    // the new sibling file become visible without restarting the app.
    if !root.is_empty() {
        library::start_scan(&app, root);
    }
    Ok(())
}

fn image_format_for(path: &std::path::Path) -> image::ImageFormat {
    match path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()).as_deref() {
        Some("png") => image::ImageFormat::Png,
        Some("webp") => image::ImageFormat::WebP,
        Some("tif") | Some("tiff") => image::ImageFormat::Tiff,
        Some("bmp") => image::ImageFormat::Bmp,
        Some("gif") => image::ImageFormat::Gif,
        _ => image::ImageFormat::Jpeg,
    }
}

fn copy_target(source: &std::path::Path, transform: &crate::types::ImageTransform, enhance: bool) -> PathBuf {
    let stem = source.file_stem().and_then(|value| value.to_str()).unwrap_or("image");
    let ext = source.extension().and_then(|value| value.to_str()).unwrap_or("jpg");
    let mut suffix = String::new();
    if transform.rotate.rem_euclid(360) != 0 {
        suffix.push_str(&format!("_rotated{}", transform.rotate.rem_euclid(360)));
    }
    if transform.flip_h { suffix.push_str("_mirror-h"); }
    if transform.flip_v { suffix.push_str("_mirror-v"); }
    // 副本的文件名要说清楚它被增强过，否则用户面对 `IMG_0001-2.jpg`
    // 没法判断哪一份是原图。
    if enhance { suffix.push_str("_enhanced"); }
    let base = source.parent().unwrap_or_else(|| std::path::Path::new("."));
    let candidate = base.join(format!("{stem}{suffix}.{ext}"));
    if !candidate.exists() { return candidate; }
    for index in 2..10_000u32 {
        let next = base.join(format!("{stem}{suffix}-{index}.{ext}"));
        if !next.exists() { return next; }
    }
    base.join(format!("{stem}{suffix}-copy.{ext}"))
}

/// 画质增强的进度快照。
///
/// 前端在增强期间轮询它来画进度条。返回结构里 `provider` 是真实生效的那一个
/// （`directml` / `cpu`），界面据此如实显示，而不是假设 GPU 一定可用。
#[tauri::command]
pub async fn enhance_progress() -> crate::enhance::EnhanceProgress {
    crate::enhance::snapshot()
}

/// 请当前这次增强停下来。已经在跑的那块瓦片会跑完再干净返回。
#[tauri::command]
pub async fn cancel_enhance() {
    crate::enhance::request_cancel();
}

/// Lists clustered people. Requires a built index.
#[tauri::command]
pub async fn list_people() -> Result<Vec<PersonSummary>, LumenError> {
    Err(LumenError::not_indexed(
        "no library index yet; people become available once indexing is implemented",
    ))
}

/// 每个类别当前有多少张照片。
///
/// 单独一个命令而不是塞进 `library_stats`：类别计数在扫描期间会一直变，
/// 而它只在用户打开类别栏时才需要，跟着每 400ms 的状态轮询一起算纯属浪费。
#[tauri::command]
pub async fn asset_tag_counts(
    state: State<'_, SharedLibrary>,
) -> Result<Vec<AssetTagCount>, LumenError> {
    let guard = state.lock().expect("library state poisoned");
    if guard.roots.is_empty() {
        return Err(LumenError::no_library(
            "no library folder is open; call open_library first",
        ));
    }
    Ok(guard.tag_counts())
}

/// Text or semantic search.
///
/// 文本检索现在真的会查：在已扫到的索引里按文件名与路径匹配（见
/// `LibraryState::search_text`）。语义检索仍然需要 CLIP 向量索引，
/// 没有它就给出相似度分数只能是编造，所以那一支依旧如实报未建索引。
#[tauri::command]
pub async fn search(
    state: State<'_, SharedLibrary>,
    req: SearchRequest,
) -> Result<Vec<SearchResult>, LumenError> {
    let guard = state.lock().expect("library state poisoned");
    if guard.roots.is_empty() {
        return Err(LumenError::no_library(
            "no library folder is open; call open_library first",
        ));
    }
    match req.mode.as_str() {
        "text" => {
            // 上限夹住：`limit` 来自前端，一个笔误不该让宿主去构造十万条结果。
            let limit = req.limit.clamp(1, MAX_SEARCH_LIMIT);
            Ok(guard.search_text(&req.query, limit))
        }
        "semantic" => Err(LumenError::not_indexed(
            "semantic search needs a CLIP embedding index; photo embeddings are not built yet",
        )),
        other => Err(LumenError::unsupported(format!(
            "unknown search mode {other:?}; expected \"text\" or \"semantic\""
        ))),
    }
}
