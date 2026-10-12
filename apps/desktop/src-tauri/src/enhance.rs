//! 画质增强：把照片送进超分模型，再缩回原尺寸。
//!
//! # 为什么是「放大两倍再缩回来」
//!
//! 超分模型的输出天生是 2 倍尺寸。直接写盘会把一张 12MP 的照片变成 48MP——
//! 那是「大的调整」，而用户要的只是把已有的细节弄干净。
//!
//! 所以这里把 2 倍结果用 Lanczos3 缩回**原尺寸**再交给调用方：
//! 输出和输入的像素尺寸完全一致，构图、比例、内容都没有变化，变的只是清晰度。
//! 这也让「增强」可以和旋转、镜像一样，作为一次普通的编辑来保存副本或覆盖原图。
//!
//! # 模型
//!
//! Swin2SR Lightweight ×2，Apache-2.0，注册在 `models::CATALOG` 里。
//!
//! 特意**没有**选 Real-ESRGAN 那一类 GAN 超分：GAN 会「脑补」出不存在的纹理，
//! 在脸和文字上尤其明显。Swin2SR 是经典（PSNR 导向）复原模型，它做的是把
//! 已有的细节恢复出来，而不是编新的——这正是「不能有任何大的调整」的意思。
//!
//! # 执行提供者
//!
//! DirectML 优先，CPU 兜底。实测 DirectML 比 CPU 快 2.5 倍（12MP 从 22 分钟
//! 降到 8-9 分钟），并且输出与 CPU 逐像素一致（943 万像素里只有 71 个差 1/255）。
//!
//! 真实用的是哪一个，`provider` 会如实报给界面——不能假设 DirectML 一定可用。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use image::imageops::FilterType;
use image::RgbImage;
use ort::ep::DirectML;
use ort::session::Session;
use ort::value::Tensor;
use serde::Serialize;
use tauri::Manager;

use crate::error::LumenError;

/// 注册表里的模型 id，同时也是落盘的文件名（`models::weight_path`）。
pub const MODEL_ID: &str = "swin2sr-lightweight-x2";

/// Swin 的窗口边长。
///
/// 输入的高和宽必须**各自**是它的整数倍。不是的话，图里会把输入补到窗口的整数倍
/// 再做 token 展开，而 `patch_unembed/Reshape` 却按**没补过**的高宽还原形状，
/// 于是 `input_shape_size == size was false` 直接失败。
const WINDOW: u32 = 8;

/// 每块瓦片的边长。
///
/// 256 是实测的最优点（0.040 ms/输入像素）。再往上会撞显存天花板：
/// 在 2GB 的 MX330 上 384 就慢 2.3 倍，512 比 CPU 还慢一倍。
/// 往下则完全没有好处——代价与瓦片大小无关，瓦片只是显存旋钮。
const TILE: u32 = 256;

const PROVIDER_UNLOADED: u8 = 0;
const PROVIDER_CPU: u8 = 1;
const PROVIDER_DIRECTML: u8 = 2;

/// 一次增强任务的进度快照。同一时刻只允许一个增强在跑，所以是全局唯一的。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceProgress {
    pub running: bool,
    /// 已经跑完的瓦片数。
    pub done: u32,
    /// 总瓦片数。还没开始算（比如正在加载模型）时是 0。
    pub total: u32,
    pub cancelled: bool,
    /// `directml` / `cpu` / `unloaded`。
    pub provider: String,
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static DONE: AtomicU32 = AtomicU32::new(0);
static TOTAL: AtomicU32 = AtomicU32::new(0);
static CANCELLED: AtomicBool = AtomicBool::new(false);
static PROVIDER: AtomicU8 = AtomicU8::new(PROVIDER_UNLOADED);

struct Engine {
    session: Session,
}

/// 已经建好的推理会话。
///
/// 建模一次要 9-11 秒，所以缓存下来；`Session::run` 需要 `&mut self`，
/// 而增强本来就是一次一个，用 Mutex 而不是给每个瓦片各建一个会话。
static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();

fn engine_cell() -> &'static Mutex<Option<Engine>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}

/// 把随包发的 ONNX Runtime 位置告诉 ort。
///
/// 必须在**任何** ort 调用之前执行：`load-dynamic` 是在第一次建 session 时才去
/// 加载那个动态库的，之后再设这个环境变量已经没用了。`setup()` 里第一件事就调它。
pub fn install_runtime_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<PathBuf, LumenError> {
    let path = app
        .path()
        .resource_dir()
        .map_err(|err| LumenError::io(format!("cannot resolve the resource dir: {err}")))?
        .join("onnxruntime.dll");
    if !path.exists() {
        return Err(LumenError::io(format!(
            "随包的 ONNX Runtime 不在 {} —— 画质增强因此不可用；其余功能不受影响",
            path.display()
        )));
    }
    // 启动阶段单线程，没有别的线程在读环境变量。
    std::env::set_var("ORT_DYLIB_PATH", &path);
    Ok(path)
}

pub fn snapshot() -> EnhanceProgress {
    EnhanceProgress {
        running: RUNNING.load(Ordering::Relaxed),
        done: DONE.load(Ordering::Relaxed),
        total: TOTAL.load(Ordering::Relaxed),
        cancelled: CANCELLED.load(Ordering::Relaxed),
        provider: match PROVIDER.load(Ordering::Relaxed) {
            PROVIDER_DIRECTML => "directml".to_string(),
            PROVIDER_CPU => "cpu".to_string(),
            _ => "unloaded".to_string(),
        },
    }
}

/// 请当前那次增强停下来。已经在跑的那一块瓦片会跑完，然后干净地返回。
pub fn request_cancel() {
    CANCELLED.store(true, Ordering::Relaxed);
}

/// 跑完（无论成功、失败还是取消）都要把 running 落回去，否则界面会永远显示「进行中」。
struct RunningGuard;

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Relaxed);
    }
}

/// 建一个跑在 DirectML 上的会话。任何一步不成立都返回 `None`，由调用方回退到 CPU。
///
/// `.error_on_failure()` 不是可选的：ort 的 `ExecutionProviderDispatch` 默认
/// `error_on_failure = false`，那意味着 DirectML 初始化失败时会**静默回退到 CPU**
/// 而调用方毫不知情。界面上写着「GPU 加速」而实际在跑 CPU，就是一句谎话，
/// 而且用户会以为 8 分钟能跑完、实际等 22 分钟。宁可在这里响亮地失败再去回退。
///
/// `with_dimension_override("batch_size", 1)` 是把批次钉死：DirectML 在形状已知时
/// 表现最好，而我们的批次永远是 1。高宽仍然动态——边缘瓦片比 256 小。
fn try_directml(model: &Path) -> Option<Session> {
    let provider = DirectML::default().with_device_id(0).build().error_on_failure();
    let builder = Session::builder().ok()?;
    let builder = builder.with_execution_providers([provider]).ok()?;
    let builder = builder.with_dimension_override("batch_size", 1).ok()?;
    let mut builder = builder;
    builder.commit_from_file(model).ok()
}

fn build_session(model: &Path) -> Result<Engine, LumenError> {
    if let Some(session) = try_directml(model) {
        PROVIDER.store(PROVIDER_DIRECTML, Ordering::Relaxed);
        return Ok(Engine { session });
    }

    // 回退：没有 DirectML（旧系统、远程桌面、驱动异常）时仍然要能增强，只是慢。
    let builder = Session::builder()
        .map_err(|err| LumenError::unsupported(format!("cannot create an ONNX Runtime session: {err}")))?;
    let builder = builder
        .with_dimension_override("batch_size", 1)
        .map_err(|err| LumenError::unsupported(format!("cannot pin the batch dimension: {err}")))?;
    let mut builder = builder;
    let session = builder.commit_from_file(model).map_err(|err| {
        LumenError::unsupported(format!(
            "cannot load the enhancement model at {}: {err}",
            model.display()
        ))
    })?;
    PROVIDER.store(PROVIDER_CPU, Ordering::Relaxed);
    Ok(Engine { session })
}

/// 取一块瓦片，摊成 NCHW 的 f32 数组。
///
/// 越界时按**边缘像素**复制而不是填黑：填黑会让照片边缘在一圈黑边上做超分，
/// 结果就是边缘发暗发糊；复制边缘则不会引入照片里本来没有的东西。
fn fill_tile(source: &RgbImage, x0: u32, y0: u32, tw: u32, th: u32, out: &mut Vec<f32>) {
    let (width, height) = source.dimensions();
    out.clear();
    out.reserve((3 * tw * th) as usize);
    for channel in 0..3usize {
        for y in 0..th {
            let sy = (y0 + y).min(height.saturating_sub(1));
            for x in 0..tw {
                let sx = (x0 + x).min(width.saturating_sub(1));
                // 模型要的是 0..1 的浮点，不是 0..255。
                out.push(source.get_pixel(sx, sy)[channel] as f32 / 255.0);
            }
        }
    }
}

/// 增强一张照片，返回**同尺寸**的结果。
pub fn enhance(source: &RgbImage, model: &Path) -> Result<RgbImage, LumenError> {
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        return Err(LumenError::unsupported("这张照片没有可用的像素尺寸"));
    }

    let padded_w = width.div_ceil(WINDOW) * WINDOW;
    let padded_h = height.div_ceil(WINDOW) * WINDOW;
    let cols = padded_w.div_ceil(TILE);
    let rows = padded_h.div_ceil(TILE);

    DONE.store(0, Ordering::Relaxed);
    TOTAL.store(cols * rows, Ordering::Relaxed);
    CANCELLED.store(false, Ordering::Relaxed);
    RUNNING.store(true, Ordering::Relaxed);
    let _running = RunningGuard;

    let cell = engine_cell();
    let mut guard = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    // 建模很贵（9-11 秒），只做一次。
    if guard.is_none() {
        *guard = Some(build_session(model)?);
    }
    let engine = guard.as_mut().expect("engine was just created");

    // 2 倍画布用 u8 存，不是 f32：12MP 的图在 2 倍下是 5000 万像素，
    // f32 三通道要 600MB，u8 只要 150MB。每块瓦片算完立刻转 u8。
    let mut canvas = RgbImage::new(padded_w * 2, padded_h * 2);
    let mut tile_input: Vec<f32> = Vec::new();

    for tile_y in 0..rows {
        for tile_x in 0..cols {
            if CANCELLED.load(Ordering::Relaxed) {
                return Err(LumenError::io("画质增强已取消"));
            }

            let x0 = tile_x * TILE;
            let y0 = tile_y * TILE;
            let tw = TILE.min(padded_w - x0);
            let th = TILE.min(padded_h - y0);

            fill_tile(source, x0, y0, tw, th, &mut tile_input);
            let shape = vec![1usize, 3, th as usize, tw as usize];
            let input = Tensor::from_array((shape, tile_input.clone()))
                .map_err(|err| LumenError::unsupported(format!("cannot build the input tensor: {err}")))?;

            let outputs = engine.session.run(ort::inputs![input]).map_err(|err| {
                LumenError::unsupported(format!("inference failed on tile ({tile_x}, {tile_y}): {err}"))
            })?;
            let (out_shape, out_data) = outputs[0].try_extract_tensor::<f32>().map_err(|err| {
                LumenError::unsupported(format!("cannot read the inference output: {err}"))
            })?;

            let dims: Vec<i64> = out_shape.iter().copied().collect();
            if dims.len() != 4 {
                return Err(LumenError::unsupported(format!(
                    "模型返回了 {} 维输出，期望 4 维",
                    dims.len()
                )));
            }
            let (out_h, out_w) = (dims[2] as u32, dims[3] as u32);
            let plane = (out_h * out_w) as usize;

            for y in 0..out_h {
                for x in 0..out_w {
                    let index = (y * out_w + x) as usize;
                    let px = image::Rgb([
                        to_u8(out_data[index]),
                        to_u8(out_data[plane + index]),
                        to_u8(out_data[2 * plane + index]),
                    ]);
                    canvas.put_pixel(x0 * 2 + x, y0 * 2 + y, px);
                }
            }

            DONE.fetch_add(1, Ordering::Relaxed);
        }
    }

    // 裁掉补边，然后缩回原尺寸。
    //
    // 这一步是「不做大的调整」的落点：输出与输入的像素尺寸完全相同。
    // 用 Lanczos3 而不是最近邻/双线性——它保边，正是我们要留住的东西。
    //
    // 没有补边时（4000x3000 这类尺寸本来就是 8 的整数倍）直接缩，不复制那一整张
    // 画布：20MP 的 2 倍画布是 242MB，白复制一份没有意义。
    if padded_w == width && padded_h == height {
        return Ok(image::imageops::resize(
            &canvas,
            width,
            height,
            FilterType::Lanczos3,
        ));
    }
    let cropped = image::imageops::crop_imm(&canvas, 0, 0, width * 2, height * 2).to_image();
    Ok(image::imageops::resize(
        &cropped,
        width,
        height,
        FilterType::Lanczos3,
    ))
}

/// 模型输出理论上在 0..1，但它会略微越界（实测最小到 -0.0064，GPU 上更低），
/// 所以夹一下再量化，而不是让它回绕成 255 的白点。
fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型输出会略微越界，必须夹紧而不是回绕。
    ///
    /// 实测 CPU 上最小到 -0.0064、GPU 上更低。不夹的话负值会被 `as u8` 回绕成
    /// 接近 255 的白点，暗部（夜空、阴影）会冒出孤立的白像素。
    #[test]
    fn output_is_clamped_not_wrapped() {
        assert_eq!(to_u8(-0.05), 0);
        assert_eq!(to_u8(-0.0001), 0);
        assert_eq!(to_u8(0.0), 0);
        assert_eq!(to_u8(1.0), 255);
        assert_eq!(to_u8(1.4), 255);
        assert_eq!(to_u8(0.5), 128);
    }

    /// 瓦片越界时按**边缘像素**复制，而不是填黑。
    ///
    /// 填黑的话照片最外一列之外是一圈纯黑，超分会在那条黑边上工作，
    /// 于是边缘发暗发糊——那是照片里本来没有的东西，正好违背「不做大的调整」。
    #[test]
    fn tiles_replicate_the_edge_instead_of_padding_with_black() {
        let mut source = RgbImage::new(2, 2);
        source.put_pixel(0, 0, image::Rgb([10, 20, 30]));
        source.put_pixel(1, 0, image::Rgb([40, 50, 60]));
        source.put_pixel(0, 1, image::Rgb([70, 80, 90]));
        source.put_pixel(1, 1, image::Rgb([100, 110, 120]));

        // 取一块 4x4：右下角整个越过原图边界。
        let mut out = Vec::new();
        fill_tile(&source, 0, 0, 4, 4, &mut out);
        assert_eq!(out.len(), 3 * 4 * 4, "NCHW 三通道");

        // R 平面按行优先摆放：out[y * 4 + x]。
        let r = |x: usize, y: usize| out[y * 4 + x];
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;

        assert!(close(r(0, 0), 10.0 / 255.0));
        assert!(close(r(1, 0), 40.0 / 255.0));
        // x=1 是最后一列，所以 x=2、3 复制它。
        assert!(close(r(2, 0), 40.0 / 255.0), "右边缘要复制，不是填 0");
        assert!(close(r(3, 0), 40.0 / 255.0), "右边缘要复制，不是填 0");
        // y=1 是最后一行，所以 y=2、3 复制它。
        assert!(close(r(0, 3), 70.0 / 255.0), "下边缘要复制，不是填 0");
        assert!(close(r(1, 3), 100.0 / 255.0), "右下角要复制，不是填 0");
    }

    /// 取消请求要能被进度快照看见——否则界面上的「取消增强」是一个没有回音的按钮。
    #[test]
    fn cancel_requests_show_up_in_the_snapshot() {
        CANCELLED.store(false, Ordering::Relaxed);
        assert!(!snapshot().cancelled);
        request_cancel();
        assert!(snapshot().cancelled);
        CANCELLED.store(false, Ordering::Relaxed);
    }

    /// 真机端到端：用真实模型和真实照片跑一遍本模块的 `enhance`。
    ///
    /// 这一条守的是「输出与原图**同尺寸**」这个承诺——也正是「不做大的调整」
    /// 在代码里的落点。前两个探针验证的是模型能跑，这条验证的是**我们的**封装能跑：
    /// 瓦片切分、边缘复制、2 倍画布、缩回原尺寸这一整条路。
    ///
    /// `#[ignore]` 是因为它依赖本机的 ONNX Runtime 与已下载的模型；
    /// 其它用例不该因为这些不在就失败。跑法：
    /// `cargo test -- --ignored --nocapture enhances_a_real_photo`
    #[test]
    #[ignore = "需要本机的 ONNX Runtime 与已下载的增强模型"]
    fn enhances_a_real_photo_at_the_same_size() {
        let dll = PathBuf::from(
            r"D:\dev\Projects\github\lumenfold\apps\desktop\src-tauri\resources\onnxruntime.dll",
        );
        let model = PathBuf::from(r"D:\dev\ort-probe\models\swin2sr-lightweight-x2-64.onnx");
        let photo = PathBuf::from(r"D:\dev\ort-probe\test.jpg");
        for (what, path) in [("dll", &dll), ("model", &model), ("photo", &photo)] {
            if !path.exists() {
                eprintln!("SKIP: no {what} at {}", path.display());
                return;
            }
        }
        std::env::set_var("ORT_DYLIB_PATH", &dll);

        let source = image::open(&photo).expect("open the photo").to_rgb8();
        // 只取一小块：整张 20MP 要跑 17 分钟，那是手工验证的事，不是用例该做的。
        let small = image::imageops::crop_imm(&source, 0, 0, 512, 384).to_image();

        let started = std::time::Instant::now();
        let out = enhance(&small, &model).expect("enhance");
        let elapsed = started.elapsed();

        eprintln!(
            "enhanced {}x{} in {:.1}s, provider={}, tiles={}",
            small.width(),
            small.height(),
            elapsed.as_secs_f64(),
            snapshot().provider,
            snapshot().total
        );

        assert_eq!(
            out.dimensions(),
            small.dimensions(),
            "增强的输出必须与原图同尺寸，否则「不做大的调整」就是空话"
        );
        assert!(out.pixels().any(|p| p.0 != [0, 0, 0]), "输出不该是全黑");
    }
}
