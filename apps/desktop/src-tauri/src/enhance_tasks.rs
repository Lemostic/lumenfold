//! Independent asynchronous photo-enhancement jobs and history.
use std::sync::{Mutex, OnceLock, atomic::{AtomicU64, Ordering}};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use crate::error::LumenError;
use crate::library::SharedLibrary;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhancementTask {
    pub id: String,
    pub asset_id: String,
    pub file_name: String,
    pub source_path: String,
    pub output_path: Option<String>,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub duration_ms: Option<u64>,
    pub status: String,
    pub input_bytes: u64,
    pub output_bytes: Option<u64>,
    pub error: Option<String>,
    pub preview_path: Option<String>,
}

static TASKS: OnceLock<Mutex<Vec<EnhancementTask>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn tasks() -> &'static Mutex<Vec<EnhancementTask>> { TASKS.get_or_init(|| Mutex::new(Vec::new())) }
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }

fn persist<R: tauri::Runtime>(app: &AppHandle<R>) {
    let Ok(dir) = app.path().app_data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join("enhancement-tasks.json.tmp");
    let path = dir.join("enhancement-tasks.json");
    if let Ok(bytes) = serde_json::to_vec_pretty(&*tasks().lock().unwrap_or_else(|p| p.into_inner())) {
        let _ = std::fs::write(&tmp, bytes);
        let _ = std::fs::rename(tmp, path);
    }
}

pub fn restore<R: tauri::Runtime>(app: &AppHandle<R>) {
    let Ok(path) = app.path().app_data_dir().map(|d| d.join("enhancement-tasks.json")) else { return };
    let Ok(bytes) = std::fs::read(path) else { return };
    let Ok(mut rows) = serde_json::from_slice::<Vec<EnhancementTask>>(&bytes) else { return };
    for row in &mut rows { if row.status == "running" || row.status == "queued" { row.status = "interrupted".into(); row.finished_at = Some(now()); } }
    *tasks().lock().unwrap_or_else(|p| p.into_inner()) = rows;
}

pub fn list() -> Vec<EnhancementTask> { tasks().lock().unwrap_or_else(|p| p.into_inner()).clone() }

pub fn start<R: tauri::Runtime>(app: AppHandle<R>, state: State<'_, SharedLibrary>, asset_id: String) -> Result<EnhancementTask, LumenError> {
    let (source, file_name) = { let guard = state.lock().map_err(|_| LumenError::io("图库状态已损坏"))?; let pos = *guard.index.get(&asset_id).ok_or_else(|| LumenError::io("找不到这张照片"))?; (guard.assets[pos].path.clone(), guard.assets[pos].file_name.clone()) };
    let input_bytes = std::fs::metadata(&source).map_err(LumenError::from)?.len();
    let model = crate::models::weight_file(&app, crate::enhance::MODEL_ID)?;
    if !model.exists() { return Err(LumenError::not_indexed("增强模型尚未下载，请先在设置中下载")); }
    let id = format!("enh-{}-{}", now(), NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let row = EnhancementTask { id: id.clone(), asset_id, file_name, source_path: source.to_string_lossy().into_owned(), output_path: None, started_at: now(), finished_at: None, duration_ms: None, status: "running".into(), input_bytes, output_bytes: None, error: None, preview_path: None };
    tasks().lock().unwrap_or_else(|p| p.into_inner()).push(row.clone()); persist(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<(String, u64), LumenError> {
            let image = image::ImageReader::open(&source).map_err(LumenError::from)?.with_guessed_format().map_err(|e| LumenError::unsupported(e.to_string()))?.decode().map_err(|e| LumenError::unsupported(e.to_string()))?;
            let rgb = image.to_rgb8();
            let enhanced = crate::enhance::enhance(&rgb, &model)?;
            let output = source.with_file_name(format!("{}.enhanced.jpg", source.file_stem().and_then(|x| x.to_str()).unwrap_or("photo")));
            let preview_dir = app.path().app_cache_dir().map_err(|e| LumenError::io(e.to_string()))?.join("enhancement-previews");
            std::fs::create_dir_all(&preview_dir).map_err(LumenError::from)?;
            let preview = preview_dir.join(format!("{id}.jpg"));
            enhanced.save(&output).map_err(|e| LumenError::io(e.to_string()))?;
            enhanced.save(&preview).map_err(|e| LumenError::io(e.to_string()))?;
            let size = std::fs::metadata(&output).map_err(LumenError::from)?.len();
            Ok((format!("{}|{}", output.display(), preview.display()), size))
        })();
        let ended = now();
        let mut rows = tasks().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(row) = rows.iter_mut().find(|r| r.id == id) { row.finished_at = Some(ended); row.duration_ms = Some(ended.saturating_sub(row.started_at)); match result { Ok((paths, size)) => { let mut it = paths.split('|'); row.output_path = it.next().map(str::to_owned); row.preview_path = it.next().map(str::to_owned); row.output_bytes = Some(size); row.status = "succeeded".into(); }, Err(err) => { row.status = "failed".into(); row.error = Some(err.to_string()); } } }
        persist(&app);
    });
    Ok(row)
}

#[tauri::command]
pub async fn start_enhancement<R: tauri::Runtime>(app: AppHandle<R>, state: State<'_, SharedLibrary>, asset_id: String) -> Result<EnhancementTask, LumenError> { start(app, state, asset_id) }
#[tauri::command]
pub async fn enhancement_tasks() -> Vec<EnhancementTask> { list() }
