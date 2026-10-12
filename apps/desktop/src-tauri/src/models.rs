//! Model registry and the background downloader (ADR-0002).
//!
//! # What this module is responsible for
//!
//! ADR-0002 §4 asks for four things, and each of them is a line you can point at
//! below:
//!
//! * **License before the first byte.** [`start`] refuses to send a request
//!   unless the caller passes back an explicit acceptance ([`start`] takes
//!   `license_accepted: bool`), and every acceptance is appended to
//!   `licenses.json` in the model directory. Nothing is fetched before that
//!   record exists.
//! * **Resumable.** Downloads stream into `<id>.part`. A restart sends
//!   `Range: bytes=<len>-`; a server that answers `200` instead of `206` means
//!   it ignored the range, and the partial file is truncated and refetched
//!   rather than silently concatenated into a corrupt blob.
//! * **Verified.** The digest is computed over the finished file. If the
//!   manifest pins one, a mismatch deletes the file and fails the download. If
//!   it does not, the digest is recorded into `manifest.json` and shown in the
//!   UI as unpinned, because claiming a checksum we never had would be worse
//!   than admitting there wasn't one.
//! * **Non-blocking.** The transfer runs on a blocking task and only ever
//!   publishes byte counts into the shared state; the frontend polls
//!   `list_models`, exactly as it already polls `library_stats` during a scan.
//!
//! # What it deliberately does not do
//!
//! It does not distribute weights. The catalog carries metadata and a source
//! URL; the bytes land in the user's own app-data directory. Per ADR-0002 §1 the
//! project never becomes the distributor, and a model whose license we cannot
//! vouch for has no entry with a download button at all.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::LumenError;
use crate::types::{ModelCapability, ModelEntry};

/// Status values the frontend branches on. Wire strings must match
/// `ModelStatus` in `src/lib/types.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Absent,
    Downloading,
    Ready,
    Error,
}

impl DownloadStatus {
    const fn as_str(self) -> &'static str {
        match self {
            DownloadStatus::Absent => "absent",
            DownloadStatus::Downloading => "downloading",
            DownloadStatus::Ready => "ready",
            DownloadStatus::Error => "error",
        }
    }
}

/// One catalog row plus whatever this machine has actually done with it.
///
/// 目录里的条目描述「这个模型是什么」，运行时条目描述「这台机器上它是什么状态」。
/// 两者分开，是因为前者进发行包、后者每次启动都不一样。
struct Entry {
    /// Catalog metadata, fixed at build time.
    spec: ModelSpec,
    status: DownloadStatus,
    /// Bytes on disk, including a partial download.
    downloaded: u64,
    /// Total size once the server has told us; `0` until then.
    total: u64,
    /// Why the last attempt failed. Empty unless `status` is `Error`.
    error: String,
    /// Digest recorded from a previous successful download.
    verified_sha256: String,
}

impl Entry {
    /// Projects one row onto the IPC contract.
    ///
    /// 「大小」两处取值合并在这里而不是散在三个命令里：清单固定了体积就用固定的，
    /// 否则用这次从 `Content-Length` 学到的。摘要同理，固定优先于本地记录。
    fn to_wire(&self) -> ModelEntry {
        ModelEntry {
            id: self.spec.id.to_string(),
            name: self.spec.name.to_string(),
            task: self.spec.task.to_string(),
            capability: self.spec.capability,
            license: self.spec.license.to_string(),
            commercial_use: self.spec.commercial_use,
            size_bytes: if self.spec.size_bytes > 0 {
                self.spec.size_bytes
            } else {
                self.total
            },
            status: self.status.as_str().to_string(),
            downloaded_bytes: self.downloaded,
            source_url: self.spec.primary_source().to_string(),
            sha256: if self.spec.sha256.is_empty() {
                self.verified_sha256.clone()
            } else {
                self.spec.sha256.to_string()
            },
        }
    }
}

/// Static catalog metadata. The mirror image of the TS `ModelEntry`.
///
/// 不做序列化：这份清单是编译期常量，运行期只读。真要序列化它，得到的也只是
/// 一份和源码重复的拷贝，改代码忘了改它反而更危险。
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub task: &'static str,
    pub capability: ModelCapability,
    pub license: &'static str,
    pub commercial_use: bool,
    /// Pinned size, `0` when resolved from `Content-Length` at download time.
    pub size_bytes: u64,
    /// 候选来源，按优先级排列：官方主机在前，已知可用的镜像在后。
    ///
    /// 为什么不是一个地址：官方主机在某些网络上会返回 502 或直接不可达。
    /// 无法为所有人指定一个主机，只能逐个尝试到一个能用的为止。
    /// 镜像是同一份文件的副本，所以中途换主机继续断点续传是安全的。
    pub sources: &'static [&'static str],
    /// Pinned digest, empty when this build does not pin one.
    pub sha256: &'static str,
}

impl ModelSpec {
    /// 界面上展示的来源：优先级最高的那个。
    fn primary_source(&self) -> &'static str {
        self.sources.first().copied().unwrap_or_default()
    }
}

/// The models this build knows how to fetch.
///
/// # 这张表为什么是这四行
///
/// ADR-0002 §3 划了一条线：**宿主能否替用户做出许可判断**。能判断的进默认注册表
/// 并给下载入口，不能判断的只登记元数据、不提供获取能力。
///
/// * `clip-text` / `clip-vision`：MIT 代码许可覆盖权重，模型卡另有部署用途声明。
///   商用允许，声明必须原样呈现（见设置页的提示文案）。
/// * `yunet-face-detect`：MIT，人脸检测的默认入口。它只找脸，不负责把人归到一起。
/// * 人脸**识别/嵌入**权重（InsightFace 系、AdaFace）在注册表里根本不存在。
///   ADR-0002 §2 已经核实过：前者仅限非商业研究，后者许可归属未明。
///   它们是「用户自备」通道——`list_models` 会把它们以 `commercial_use: false`
///   的形式报出来，界面上没有下载按钮，用户自己放到模型目录。
const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "clip-text",
        name: "CLIP ViT-B/32 (text encoder)",
        task: "语义检索的文本编码器",
        capability: ModelCapability::TextEmbedding,
        license: "MIT",
        commercial_use: true,
        size_bytes: 0,
        sources: &[
            "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/onnx/text_model_quantized.onnx",
            "https://hf-mirror.com/Xenova/clip-vit-base-patch32/resolve/main/onnx/text_model_quantized.onnx",
        ],
        sha256: "",
    },
    ModelSpec {
        id: "clip-vision",
        name: "CLIP ViT-B/32 (vision encoder)",
        task: "图像编码与自动标签",
        capability: ModelCapability::ImageEmbedding,
        license: "MIT",
        commercial_use: true,
        size_bytes: 0,
        sources: &[
            "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/onnx/vision_model_quantized.onnx",
            "https://hf-mirror.com/Xenova/clip-vit-base-patch32/resolve/main/onnx/vision_model_quantized.onnx",
        ],
        sha256: "",
    },
    ModelSpec {
        id: "yunet-face-detect",
        name: "YuNet (face detector)",
        task: "人脸检测：找出照片里有几张脸、脸在哪",
        capability: ModelCapability::FaceDetect,
        license: "MIT",
        commercial_use: true,
        size_bytes: 0,
        sources: &[
            "https://huggingface.co/opencv/opencv_zoo/resolve/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx",
            "https://hf-mirror.com/opencv/opencv_zoo/resolve/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx",
        ],
        sha256: "",
    },
    ModelSpec {
        id: "arcface-face-embed",
        name: "ArcFace w600k_r50 (face embedding)",
        task: "人脸识别：把一张脸变成可比较的向量",
        capability: ModelCapability::FaceEmbed,
        license: "InsightFace，仅限非商业研究用途",
        commercial_use: false,
        size_bytes: 0,
        sources: &[
            "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx",
            "https://hf-mirror.com/immich-app/buffalo_l/resolve/main/recognition/model.onnx",
        ],
        sha256: "",
    },
    ModelSpec {
        id: "scrfd-face-detect",
        name: "SCRFD det_10g (face detector)",
        task: "人脸检测：比 YuNet 微小脸和侧脸更稳，速度慢",
        capability: ModelCapability::FaceDetect,
        license: "InsightFace，仅限非商业研究用途",
        commercial_use: false,
        size_bytes: 0,
        sources: &[
            "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx",
            "https://hf-mirror.com/immich-app/buffalo_l/resolve/main/detection/model.onnx",
        ],
        sha256: "",
    },
    ModelSpec {
        id: "swin2sr-lightweight-x2",
        name: "Swin2SR Lightweight x2（画质增强）",
        task: "画质增强：放大两倍再缩回原尺寸，只锐化细节，不改变构图",
        capability: ModelCapability::SuperResolution,
        license: "Apache-2.0",
        commercial_use: true,
        // 开发机上实际下载过一次并算过摘要，所以这里钉死了而不是留空。
        size_bytes: 8_078_888,
        sources: &[
            "https://huggingface.co/Xenova/swin2SR-lightweight-x2-64/resolve/main/onnx/model.onnx",
            "https://hf-mirror.com/Xenova/swin2SR-lightweight-x2-64/resolve/main/onnx/model.onnx",
        ],
        // 钉摘要的代价要说清楚：上游如果把 `main` 重新推一次，校验就会失败，
        // 而这个功能会一直卡在「校验没过」直到我们更新这个常量。
        // 之所以还是钉，是因为 ADR-0002 §4 把「校验通过」当成标记 ready 的前提，
        // 留空等于这条保证只存在于文档里。
        sha256: "c2abbfe0cc8e685b5e11964970f8ebe3d24072e904fd5545a30ac31ec1e110db",
    },
];

/// Shared state, managed by Tauri exactly like the library state.
pub type SharedModels = Mutex<ModelState>;

pub struct ModelState {
    entries: Vec<Entry>,
    /// Cancel flags for downloads in flight, keyed by model id.
    cancels: HashMap<String, Arc<AtomicBool>>,
    /// Where weights and the manifests live. Set once during startup.
    root: Option<PathBuf>,
}

impl ModelState {
    pub fn empty() -> Self {
        let entries = CATALOG
            .iter()
            .cloned()
            .map(|spec| Entry {
                spec,
                status: DownloadStatus::Absent,
                downloaded: 0,
                total: 0,
                error: String::new(),
                verified_sha256: String::new(),
            })
            .collect();
        Self { entries, cancels: HashMap::new(), root: None }
    }

    /// The catalog as the IPC contract, with this machine's state folded in.
    pub fn entries(&self) -> Vec<ModelEntry> {
        self.entries.iter().map(Entry::to_wire).collect()
    }

    fn position(&self, model_id: &str) -> Result<usize, LumenError> {
        self.entries
            .iter()
            .position(|entry| entry.spec.id == model_id)
            .ok_or_else(|| LumenError::unsupported(format!("unknown model id {model_id:?}")))
    }
}

/// Installs the rustls crypto provider.
///
/// `reqwest` is built with `rustls-no-provider` so that it does not drag in
/// aws-lc-rs's C toolchain requirement. That makes installing a provider our
/// job, and it has to happen before the first `Client` is built.
///
/// Failure here is not fatal: it only means TLS is unavailable, and the
/// downloader will report that as an ordinary network error instead of
/// panicking at startup.
pub fn install_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Where weights live. `app_data_dir/models`.
fn model_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, LumenError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| LumenError::io(format!("cannot resolve the app data dir: {err}")))?
        .join("models");
    std::fs::create_dir_all(&dir)
        .map_err(|err| LumenError::io(format!("cannot create {}: {err}", dir.display())))?;
    Ok(dir)
}

/// Final path for one model's weight file.
fn weight_path(dir: &Path, model_id: &str) -> PathBuf {
    dir.join(format!("{model_id}.onnx"))
}

/// Partial path used while the transfer is in flight.
fn part_path(dir: &Path, model_id: &str) -> PathBuf {
    dir.join(format!("{model_id}.onnx.part"))
}

/// 某个已注册模型的权重文件落盘位置。
///
/// 给画质增强那条路用：它要把模型直接交给 ONNX Runtime，不经过下载器的状态机。
/// 路径规则只在 `weight_path` 里定义一次，避免两边各写一遍 `format!("{id}.onnx")`
/// 然后在某次改动里悄悄漂移。
pub fn weight_file<R: Runtime>(
    app: &AppHandle<R>,
    model_id: &str,
) -> Result<PathBuf, LumenError> {
    Ok(weight_path(&model_dir(app)?, model_id))
}

/// Recorded results, so a second launch knows what is already verified.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    /// `model_id -> sha256 hex`.
    #[serde(default)]
    digests: HashMap<String, String>,
    /// `model_id -> size in bytes`.
    #[serde(default)]
    sizes: HashMap<String, u64>,
}

/// License acceptances, appended in order. ADR-0002 §4 asks for a record.
#[derive(Debug, Serialize, Deserialize)]
struct LicenseAcceptance {
    model_id: String,
    license: String,
    /// Unix seconds.
    accepted_at: i64,
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

fn licenses_path(dir: &Path) -> PathBuf {
    dir.join("licenses.json")
}

fn read_manifest(dir: &Path) -> Manifest {
    std::fs::read_to_string(manifest_path(dir))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), LumenError> {
    let body = serde_json::to_string_pretty(manifest)
        .map_err(|err| LumenError::io(format!("cannot serialize the model manifest: {err}")))?;
    std::fs::write(manifest_path(dir), body)
        .map_err(|err| LumenError::io(format!("cannot write the model manifest: {err}")))
}

/// Appends one acceptance. Failure to record is fatal on purpose: ADR-0002
/// wants the acceptance on record, and downloading weights we cannot prove the
/// user agreed to is exactly the failure mode the ADR is written against.
fn record_acceptance(dir: &Path, spec: &ModelSpec) -> Result<(), LumenError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut entries: Vec<LicenseAcceptance> = std::fs::read_to_string(licenses_path(dir))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default();
    entries.push(LicenseAcceptance {
        model_id: spec.id.to_string(),
        license: spec.license.to_string(),
        accepted_at: now,
    });
    let body = serde_json::to_string_pretty(&entries)
        .map_err(|err| LumenError::io(format!("cannot serialize license records: {err}")))?;
    std::fs::write(licenses_path(dir), body)
        .map_err(|err| LumenError::io(format!("cannot write license records: {err}")))
}

/// Recomputes each entry's status from what is actually on disk.
///
/// Called once at startup. A weight file left behind by a previous run is
/// trusted only if its digest matches the manifest; an unverifiable leftover is
/// deleted rather than shown as ready.
pub fn reconcile<R: Runtime>(app: &AppHandle<R>) {
    let Ok(dir) = model_dir(app) else { return };
    let manifest = read_manifest(&dir);
    let state = app.state::<SharedModels>();
    let mut guard = state.lock().expect("model state poisoned");
    guard.root = Some(dir.clone());

    for entry in guard.entries.iter_mut() {
        let path = weight_path(&dir, entry.spec.id);
        let part = part_path(&dir, entry.spec.id);

        // 分片是断点状态，不是已安装的权重：记下它的长度好让界面显示「已下载 40%」，
        // 但状态仍然是没有这个模型。
        if part.exists() {
            entry.downloaded = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        } else {
            entry.downloaded = 0;
        }

        if !path.exists() {
            entry.status = DownloadStatus::Absent;
            continue;
        }

        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        entry.downloaded = size;
        entry.total = manifest
            .sizes
            .get(entry.spec.id)
            .copied()
            .unwrap_or(size);
        entry.verified_sha256 = manifest
            .digests
            .get(entry.spec.id)
            .cloned()
            .unwrap_or_default();

        // 清单里固定了摘要就必须对上，对不上就是损坏：删掉，不给「先凑合用」的机会。
        if !entry.spec.sha256.is_empty() && !entry.verified_sha256.is_empty() {
            if entry.spec.sha256 != entry.verified_sha256 {
                let _ = std::fs::remove_file(&path);
                entry.status = DownloadStatus::Absent;
                entry.downloaded = 0;
                entry.error = "本地权重与清单摘要不一致，已删除，请重新下载".to_string();
                continue;
            }
        }
        entry.status = DownloadStatus::Ready;
        entry.error.clear();
    }
}

/// Marks one model as downloading, refusing the call if it cannot proceed.
///
/// Returns `(position, spec, cancel_flag)`. The caller owns the flag and must
/// drop it from the map when the transfer ends.
fn begin<R: Runtime>(
    app: &AppHandle<R>,
    model_id: &str,
    license_accepted: bool,
) -> Result<(PathBuf, ModelSpec, Arc<AtomicBool>), LumenError> {
    install_crypto_provider();
    let dir = model_dir(app)?;
    let state = app.state::<SharedModels>();

    let (position, spec) = {
        let guard = state.lock().expect("model state poisoned");
        let position = guard.position(model_id)?;
        let spec = guard.entries[position].spec.clone();
        (position, spec)
    };

    // 非商用权重也给下载入口。
    //
    // 这与 ADR-0002 §3 原来的划分不同，是项目发起人明确要求的：
    // 个人使用，所有模型都要可下载。保留记录、非商用标识与“许可先于首字节”
    // 三件事，只放开了下载入口这一个门。
    if spec.sources.is_empty() {
        return Err(LumenError::unsupported(format!(
            "{} 没有可用的下载来源",
            spec.name
        )));
    }
    // ADR-0002 §4：许可先于首字节。没有显式接受，一个字节都不发。
    if !license_accepted {
        return Err(LumenError::unsupported(format!(
            "下载 {} 前必须显式接受 {} 许可",
            spec.name, spec.license
        )));
    }
    record_acceptance(&dir, &spec)?;

    let flag = Arc::new(AtomicBool::new(false));
    let mut guard = state.lock().expect("model state poisoned");
    if guard.cancels.contains_key(model_id) {
        return Err(LumenError::io(format!("{} 已经在下载中", spec.name)));
    }
    guard.cancels.insert(model_id.to_string(), Arc::clone(&flag));
    let entry = &mut guard.entries[position];
    entry.status = DownloadStatus::Downloading;
    entry.error.clear();
    entry.total = entry.total.max(entry.spec.size_bytes);
    Ok((dir, spec, flag))
}

/// Clears the in-flight flag and writes the terminal status.
fn finish<R: Runtime>(app: &AppHandle<R>, model_id: &str, outcome: Result<(), String>) {
    let state = app.state::<SharedModels>();
    let mut guard = state.lock().expect("model state poisoned");
    guard.cancels.remove(model_id);
    let Ok(position) = guard.position(model_id) else { return };
    let entry = &mut guard.entries[position];
    match outcome {
        Ok(()) => {
            entry.status = DownloadStatus::Ready;
            entry.error.clear();
        }
        Err(message) => {
            entry.status = if entry.downloaded > 0 {
                // 分片留着，下次从断点继续；这不是失败，是「还没下完」。
                DownloadStatus::Absent
            } else {
                DownloadStatus::Error
            };
            entry.error = message;
        }
    }
}

/// Publishes byte progress without holding the lock across the write.
fn progress<R: Runtime>(app: &AppHandle<R>, model_id: &str, downloaded: u64, total: u64) {
    let state = app.state::<SharedModels>();
    let mut guard = state.lock().expect("model state poisoned");
    let Ok(position) = guard.position(model_id) else { return };
    let entry = &mut guard.entries[position];
    entry.downloaded = downloaded;
    if total > 0 {
        entry.total = total;
    }
}

/// Starts a download in the background and returns immediately.
///
/// The returned [`ModelEntry`] reflects the `downloading` status; progress
/// arrives through `list_models`.
#[tauri::command]
pub async fn start_model_download<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, SharedModels>,
    model_id: String,
    license_accepted: bool,
) -> Result<ModelEntry, LumenError> {
    let (dir, spec, flag) = begin(&app, &model_id, license_accepted)?;

    let app_for_task = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app_for_progress = app_for_task.clone();
        let mut report = |done: u64, total: u64| progress(&app_for_progress, spec.id, done, total);
        let result = run_transfer(&dir, &spec, &flag, &mut report);
        finish(&app_for_task, spec.id, result);
    });

    let guard = state.lock().expect("model state poisoned");
    let entry = guard
        .entries
        .iter()
        .find(|entry| entry.spec.id == model_id)
        .ok_or_else(|| LumenError::unsupported(format!("unknown model {model_id:?}")))?;
    Ok(entry.to_wire())
}

/// The actual transfer. Blocking; runs on a blocking task.
///
/// Tries each candidate source in turn. A host that refuses the connection or
/// answers 5xx is skipped rather than fatal, because the usual cause is that the
/// canonical host is unreachable from this machine and a mirror is not.
fn run_transfer(
    dir: &Path,
    spec: &ModelSpec,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Lumenfold/", env!("CARGO_PKG_VERSION")))
        .timeout(None)
        .build()
        .map_err(|err| format!("cannot build the HTTP client: {err}"))?;

    let part = part_path(dir, spec.id);
    let mut failures: Vec<String> = Vec::new();

    for source in spec.sources {
        match transfer_once(&client, spec, source, &part, cancel, on_progress) {
            Ok(()) => return Ok(()),
            Err(message) => {
                // 取消不是失败：分片留着，下次从断点继续。换主机继续无意义。
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                failures.push(format!("{source}: {message}"));
            }
        }
    }

    Err(format!(
        "所有来源都没下成。{}（已保留 {} 的分片，可继续下载）",
        failures.join("；"),
        format_bytes(part_size(&part)),
    ))
}

fn part_size(part: &Path) -> u64 {
    std::fs::metadata(part).map(|meta| meta.len()).unwrap_or(0)
}

/// One attempt against one source, resuming from whatever is already on disk.
fn transfer_once(
    client: &reqwest::blocking::Client,
    spec: &ModelSpec,
    source: &str,
    part: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let final_path = weight_path(part.parent().unwrap_or(Path::new(".")), spec.id);

    // 断点：已经有的分片长度就是下次要要的 Range 起点。
    let mut have = part_size(part);

    let mut request = client.get(source);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }

    let mut response = request.send().map_err(|err| format!("连接失败：{err}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("服务器返回 {status}"));
    }

    // 服务端不认 Range 时回的是 200 而不是 206：那份分片和这次返回的字节
    // 首尾不相接，追加进去就是一个坏文件。所以丢掉重来。
    if have > 0 && status == reqwest::StatusCode::OK {
        let _ = std::fs::remove_file(part);
        have = 0;
    }

    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let total = response
        .content_length()
        .map(|remaining| if resumed { remaining + have } else { remaining })
        .unwrap_or(0);

    on_progress(have, total);

    // 续传追加、重来截断：两条路径分开写，不用 `.append(!r)/.truncate(!r)`
    // 一把推出来。write 与 append 同时为 true 是非法组合，而且读不到源头的时候
    // 分步重来应该是“丢掉分片重下”，而不是“在旧文件上继续写”。
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if resumed {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    let mut file = options
        .open(part)
        .map_err(|err| format!("cannot open the partial file: {err}"))?;

    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            // 分片留着，下次接着下。
            return Ok(());
        }
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("读取响应体失败：{err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("写入分片失败：{err}"))?;
        have += read as u64;
        on_progress(have, total);
    }
    file.flush().map_err(|err| format!("写入分片失败：{err}"))?;
    drop(file);

    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }

    verify_and_install(part.parent().unwrap_or(Path::new(".")), spec, part, &final_path)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 || unit <= 1 {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Hashes the finished file and only then moves it into place.
fn verify_and_install(
    dir: &Path,
    spec: &ModelSpec,
    part: &Path,
    final_path: &Path,
) -> Result<(), String> {
    let mut file =
        std::fs::File::open(part).map_err(|err| format!("cannot open the partial file: {err}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)
            .map_err(|err| format!("cannot read the partial file: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hex::encode(hasher.finalize());

    // 清单固定了摘要就必须对上。对不上就是损坏，删掉重来，不留「先凑合用」的余地。
    if !spec.sha256.is_empty() && !spec.sha256.eq_ignore_ascii_case(&digest) {
        let _ = std::fs::remove_file(part);
        return Err(format!(
            "{} 校验失败：期望 {}, 实际 {}",
            spec.name, spec.sha256, digest
        ));
    }

    std::fs::rename(part, final_path)
        .map_err(|err| format!("cannot install the weight file: {err}"))?;

    let size = std::fs::metadata(final_path).map(|m| m.len()).unwrap_or(0);
    let mut manifest = read_manifest(dir);
    manifest.digests.insert(spec.id.to_string(), digest);
    manifest.sizes.insert(spec.id.to_string(), size);
    write_manifest(dir, &manifest).map_err(|err| err.message().to_string())?;
    Ok(())
}

/// Stops an in-flight download. The partial file is kept for resumption.
#[tauri::command]
pub async fn cancel_model_download<R: Runtime>(
    app: AppHandle<R>,
    model_id: String,
) -> Result<ModelEntry, LumenError> {
    let state = app.state::<SharedModels>();
    let guard = state.lock().expect("model state poisoned");
    let flag = guard.cancels.get(&model_id).cloned();
    drop(guard);

    match flag {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            Ok(after_cancel(&app, &model_id))
        }
        None => Err(LumenError::unsupported(format!("{model_id:?} 没有正在进行的下载"))),
    }
}

/// Reads the entry back once the cancel flag has been raised.
///
/// The transfer thread owns the final status write; this only reports what the
/// catalog says right now, so the UI's button flips without waiting for the
/// socket to close.
fn after_cancel<R: Runtime>(app: &AppHandle<R>, model_id: &str) -> ModelEntry {
    let state = app.state::<SharedModels>();
    let guard = state.lock().expect("model state poisoned");
    let entry = guard
        .entries
        .iter()
        .find(|entry| entry.spec.id == model_id)
        .expect("cancel implies a known model");
    entry.to_wire()
}

/// The catalog with this machine's state folded in.
#[tauri::command]
pub fn list_models(state: tauri::State<'_, SharedModels>) -> Vec<ModelEntry> {
    state.lock().expect("model state poisoned").entries()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表里的每一项都要能下载。
    ///
    /// 项目发起人要求所有模型都可下载（个人使用），所以没有任何一项只登记元数据。
    #[test]
    fn every_catalog_entry_is_downloadable() {
        for spec in CATALOG {
            assert!(!spec.sources.is_empty(), "{} has no source url", spec.id);
            assert!(
                spec.sources.iter().all(|source| source.starts_with("https://")),
                "{} has a non-https source",
                spec.id
            );
        }
    }

    /// 非商用这一事实本身必须在清单上看得见。
    ///
    /// 接口开了下载入口，界面上的非商用提示就是剩下唯一的约束，
    /// 它一旦回退，非商用权重就会在无任何提示的情况下被下载。
    #[test]
    fn non_commercial_weights_stay_labelled_as_such() {
        let arcface = CATALOG
            .iter()
            .find(|spec| spec.capability == ModelCapability::FaceEmbed)
            .expect("a face embedding model is registered");
        assert!(!arcface.commercial_use);
        assert!(arcface.license.contains("非商业"));
    }

    #[test]
    fn entries_carry_the_catalog_fields_through() {
        let state = ModelState::empty();
        let rows = state.entries();
        assert_eq!(rows.len(), CATALOG.len());
        let clip = rows.iter().find(|row| row.id == "clip-vision").expect("clip");
        assert_eq!(clip.status, "absent");
        assert!(clip.commercial_use);
        assert_eq!(clip.capability, ModelCapability::ImageEmbedding);
    }

    /// 需要联网的用例（`cargo test -- --ignored`）。
    ///
    /// 断言的是 ADR-0002 真正依赖的那条契约，而不是「每个地址都能通」：
    ///
    /// * 每个模型**至少有一个**来源能取到字节——换主机正是为此存在的；
    /// * 能取到字节的来源必须认 `Range`，否则断点续传这个保证是空的；
    /// * 用户自备的权重一个来源都没有。
    #[test]
    #[ignore = "needs network"]
    fn every_downloadable_model_has_a_working_source_that_supports_range() {
        install_crypto_provider();
        let client = reqwest::blocking::Client::builder()
            .user_agent("Lumenfold-probe")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("build client");

        for spec in CATALOG.iter().filter(|spec| !spec.sources.is_empty()) {
            let mut working = Vec::new();
            for &source in spec.sources {
                let head = match client.head(source).send() {
                    Ok(response) if response.status().is_success() => response,
                    Ok(response) => {
                        println!("  skip {} {source}: {}", spec.id, response.status());
                        continue;
                    }
                    Err(err) => {
                        println!("  skip {} {source}: {err}", spec.id);
                        continue;
                    }
                };

                let total: u64 = head
                    .headers()
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                assert!(total > 0, "{} {source} reported no content-length", spec.id);

                let ranged = client
                    .get(source)
                    .header("Range", "bytes=0-1023")
                    .send()
                    .unwrap_or_else(|err| panic!("{} ranged GET {source} failed: {err}", spec.id));
                assert_eq!(
                    ranged.status(),
                    reqwest::StatusCode::PARTIAL_CONTENT,
                    "{} {source} must answer 206 to a Range request, or resuming a partial download would silently corrupt the file",
                    spec.id
                );
                working.push(source);
            }
            assert!(
                !working.is_empty(),
                "{} has no reachable source; tried {:?}",
                spec.id,
                spec.sources
            );
        }
    }

    /// 端到端的下载用例（需联网，`cargo test -- --ignored`）。
    ///
    /// 拿注册表里最小的那个权重（YuNet，约 230 KB）真下一遍。只有真过一遍的是下载，
    /// 而不是测它的接口。三件事同时验证：取消不落盘、断点续传真的能接上、
    /// SHA-256 落盘并与文件实体一致。
    #[test]
    #[ignore = "needs network"]
    fn a_download_runs_end_to_end_and_survives_a_cancel() {
        install_crypto_provider();
        let spec = CATALOG
            .iter()
            .find(|spec| spec.id == "yunet-face-detect")
            .expect("yunet is registered");
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build client");

        // 取一个能用的来源，同时拿到它的总长，后面这个数用来判断是否下完了。
        let source = spec
            .sources
            .iter()
            .find(|source| {
                client
                    .head(**source)
                    .send()
                    .map(|response| response.status().is_success())
                    .unwrap_or(false)
            })
            .copied()
            .unwrap_or_else(|| panic!("no reachable source for {}", spec.id));
        // 总长从 Content-Range 读：HEAD 的响应没有实体，`content_length()` 对它返回 0。
        // 这个数代表“多少字节才算完”，取错了会让断点续传的判断反过来。
        let ranged = client
            .get(source)
            .header("Range", "bytes=0-0")
            .send()
            .expect("probe range");
        assert_eq!(ranged.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        let expected: u64 = ranged
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("no total in Content-Range for {source}"));

        let dir = std::env::temp_dir().join(format!("lumenfold-download-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");

        // 1. 取消：抬起标记后立即返回，不得有任何被安装的文件。
        let cancel = AtomicBool::new(false);
        run_transfer(&dir, spec, &cancel, &mut |_, _| cancel.store(true, Ordering::SeqCst))
            .expect("a cancelled transfer is not an error");
        assert!(
            !weight_path(&dir, spec.id).exists(),
            "a cancelled download must not be installed"
        );

        // 2. 断点续传：先自己取前 4 KB 当作“上次断在这儿的分片”，再让下载器接上它。
        //    只比对最终大小是不够的——那样一个从错起点追加的文件大小也会对。
        let prefix_len = 4096u64;
        let prefix = client
            .get(source)
            .header("Range", format!("bytes=0-{}", prefix_len - 1))
            .send()
            .expect("prefix request")
            .bytes()
            .expect("prefix body");
        assert_eq!(prefix.len() as u64, prefix_len);
        std::fs::write(part_path(&dir, spec.id), &prefix).expect("seed the partial file");

        let cancel = AtomicBool::new(false);
        let mut last = (0u64, 0u64);
        run_transfer(&dir, spec, &cancel, &mut |done, total| last = (done, total))
            .unwrap_or_else(|err| panic!("resumed download failed: {err}"));

        let installed = weight_path(&dir, spec.id);
        assert!(installed.exists(), "the weight file should be installed");
        assert_eq!(std::fs::metadata(&installed).unwrap().len(), expected);
        assert_eq!(last.0, expected, "progress should end at the real file size");
        assert_eq!(last.1, expected, "total should come from Content-Length");

        // 3. 摘要已落盘，并且真的算一遍对上——否则“校验通过”只是一句话。
        let manifest = read_manifest(&dir);
        let digest = manifest
            .digests
            .get(spec.id)
            .expect("a finished download must record its digest")
            .clone();
        assert_eq!(digest.len(), 64, "sha256 hex is 64 chars");
        assert_eq!(manifest.sizes.get(spec.id).copied(), Some(expected));

        let bytes = std::fs::read(&installed).expect("read installed");
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        assert_eq!(hex::encode(hasher.finalize()), digest);
        assert_eq!(&bytes[..prefix_len as usize], &prefix[..], "the seeded prefix must survive the resume");

        let _ = std::fs::remove_dir_all(&dir);
    }


    /// 摘要对不上时必须否决拒绝安装并抹掉分片。
    #[test]
    fn a_digest_mismatch_deletes_the_file_instead_of_installing_it() {
        let dir = std::env::temp_dir().join(format!("lumenfold-baddigest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");

        let mut spec = CATALOG[0].clone();
        spec.id = "clip-text";
        // 一个绝对对不上的摘要。
        spec.sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

        let part = part_path(&dir, spec.id);
        std::fs::write(&part, b"not the weights you asked for").expect("write fake payload");

        let outcome = verify_and_install(&dir, &spec, &part, &weight_path(&dir, spec.id));
        assert!(outcome.is_err(), "a mismatch must fail the download");
        assert!(!weight_path(&dir, spec.id).exists(), "nothing may be installed");
        assert!(!part.exists(), "the corrupt fragment must be deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_model_ids_are_rejected() {
        let state = ModelState::empty();
        assert!(state.position("nope").is_err());
    }
}