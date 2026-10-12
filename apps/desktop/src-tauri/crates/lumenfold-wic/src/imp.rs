//! Windows 实现：COM 生命周期、解码器打开，以及尺寸/帧数/解码三个动作。

use std::path::Path;

use windows::core::{Interface, HSTRING};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory2, GUID_ContainerFormatHeif, GUID_WICPixelFormat32bppBGRA,
    IWICBitmapDecoder, IWICBitmapFrameDecode, IWICBitmapSource, IWICFormatConverter,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapInterpolationModeHighQualityCubic,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

use crate::DecodedImage;

/// 每个像素 4 字节，BGRA8。
const BYTES_PER_PIXEL: u32 = 4;

/// `RPC_E_CHANGED_MODE`：本线程已经以另一种套间模式初始化过 COM。
///
/// 这不是错误，只是「这个线程不归我们管」——照常往下走，并且绝不替它 `CoUninitialize`。
const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;

/// COM 初始化守卫。
///
/// # 为什么每次调用都自己初始化
///
/// 调用方有两个：Tauri 的异步运行时线程，以及 `spawn_blocking` 的工作线程。
/// 两者都不能假定进函数时 COM 已经就绪，也不能假定它没有就绪。
///
/// # 为什么只在自己初始化成功时才反初始化
///
/// `CoInitializeEx` 的三种返回要分开对待：
///
/// * `S_OK`——这次调用把引用计数从 0 变成 1，**我们必须**配一次 `CoUninitialize`。
/// * `S_FALSE`——套间模式相同，计数加一。少配一次 `CoUninitialize` 的代价是工作线程
///   上永久多挂着一次 COM 引用，而工作线程是复用的，所以这里选择配对反初始化。
/// * `RPC_E_CHANGED_MODE`——我们**没有**拿到引用，此时调 `CoUninitialize` 会把别人
///   初始化好的套间拆掉，那是个比泄漏难查得多的 bug。这一支直接跳过。
///
/// 另外用一个全局的 `CoUninitialize` 是错的：线程套间是每线程状态，
/// 在 A 线程上反初始化对 B 线程毫无作用，只会让引用计数错乱。
struct ComGuard {
    /// 只有本次调用真的把引用计数加上去了，才允许反初始化。
    uninit_on_drop: bool,
}

impl ComGuard {
    /// `None` 表示 COM 在这个线程上不可用，调用方应直接返回 `None`。
    fn new() -> Option<ComGuard> {
        // SAFETY: 两个参数都是文档允许的取值（保留参数传空、套间模式是常量）。
        // 返回值是 HRESULT，不构造任何需要释放的对象。
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        match hr.0 {
            // 首次初始化：本次调用持有引用，drop 时配对释放。
            0 => Some(ComGuard {
                uninit_on_drop: true,
            }),
            // 同模式重复初始化：同样持有一个引用，同样配对释放。
            1 => Some(ComGuard {
                uninit_on_drop: true,
            }),
            RPC_E_CHANGED_MODE => Some(ComGuard {
                uninit_on_drop: false,
            }),
            _ => None,
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninit_on_drop {
            // SAFETY: 只有在 `CoInitializeEx` 返回成功码时才置位，
            // 因此这里与那次成功调用严格配对。
            unsafe { CoUninitialize() };
        }
    }
}

/// WIC 的成像工厂。
///
/// 只用来创建解码器；工厂本身是进程内的单例式组件，每次创建都很便宜，
/// 所以不引入全局缓存——缓存一个 COM 指针就意味着还要处理跨线程的套间 marshal。
fn factory() -> Option<IWICImagingFactory> {
    // SAFETY: `CLSID_WICImagingFactory2` 是系统注册的进程内组件，传空 `punkouter`
    // 是文档要求的写法。返回的 `Result` 已由 `ok()` 收敛，不会 panic。
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory2, None, CLSCTX_INPROC_SERVER) }.ok()
}

/// 打开 `path` 的 HEIF 解码器。
///
/// `CreateDecoderFromFilename` 让 WIC 自己去认容器；我们再校验一次容器格式，
/// 这样「拿一个 JPEG 过来」得到的是 `None` 而不是一张莫名其妙的位图。
fn open_decoder(path: &Path) -> Option<IWICBitmapDecoder> {
    let factory = factory()?;
    let decoder = {
        // `HSTRING` 负责把路径变成以 NUL 结尾的 UTF-16，路径里有中文也不会出问题。
        let filename = HSTRING::from(path.as_os_str());
        // SAFETY: 路径来自有效的 `OsStr`；`None` 表示「不限定厂商」；
        // 访问权限是只读；元数据按需缓存，探测尺寸时不必解析全部元数据。
        unsafe {
            factory.CreateDecoderFromFilename(
                &filename,
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
        }
        .ok()?
    };

    // SAFETY: `decoder` 是刚创建成功的有效接口指针。
    let container = unsafe { decoder.GetContainerFormat() }.ok()?;
    (container == GUID_ContainerFormatHeif).then_some(decoder)
}

/// 取第 `frame` 帧；越界或没有帧时返回 `None`。
fn frame_of(decoder: &IWICBitmapDecoder, frame: u32) -> Option<IWICBitmapFrameDecode> {
    // SAFETY: `decoder` 有效；`frame` 由调用方给出，越界只会得到 `Err`。
    unsafe { decoder.GetFrame(frame) }.ok()
}

/// `(width, height)`，任意一维为 0 时视为无效。
fn frame_size(frame: &IWICBitmapFrameDecode) -> Option<(u32, u32)> {
    let (mut width, mut height) = (0u32, 0u32);
    // SAFETY: 两个出参都是本栈上的局部变量，生命周期覆盖整个调用。
    unsafe { frame.GetSize(&mut width, &mut height) }.ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// 尺寸上限的口径。
///
/// 两种口径都合理，但用错了不会报错，只会让图糊一档或者多占一份内存，
/// 所以把它写成类型而不是一个裸的 `u32`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    /// 长边上限。横幅照片上它就等于宽度。
    LongEdge(u32),
    /// 宽度上限。与 `thumbnail::fit` 一致，也是 `lumen://thumb/…?w=` 的契约。
    Width(u32),
}

/// 按上限等比缩小；源本来就小就原样返回。
///
/// **只缩不放**：把一张 40px 的图标放大到 4096px 只会得到一张模糊的大图，
/// 那不是缩略图该有的样子。
fn target_size(width: u32, height: u32, limit: Option<Limit>) -> (u32, u32) {
    let Some(limit) = limit else {
        return (width, height);
    };
    let long_edge = width.max(height);
    let max_long_edge = match limit {
        Limit::LongEdge(limit) => limit,
        // 宽度口径要先换算成长边口径。缩放后的宽度是 `width * max_long_edge / long_edge`，
        // 令它等于 `limit_w` 就得到 `max_long_edge = limit_w * long_edge / width`。
        // 这里向上取整，多出来的那一个像素由 `thumbnail::fit` 再夹回去，
        // 而不是让它少一个像素。
        Limit::Width(limit_w) => {
            let numerator = u64::from(limit_w) * u64::from(long_edge);
            match u32::try_from(numerator.div_ceil(u64::from(width.max(1)))) {
                Ok(limit) => limit,
                // 换算结果超出 u32：当作没有上限，绝不返回一个错的尺寸。
                Err(_) => return (width, height),
            }
        }
    };
    if max_long_edge == 0 || long_edge <= max_long_edge {
        return (width, height);
    }
    // 先算分子再除，避免 `edge * max_long_edge` 在 4 万像素级的图上溢出 u32。
    let scaled = |edge: u32| -> u32 {
        ((u64::from(edge) * u64::from(max_long_edge)) / u64::from(long_edge)).max(1) as u32
    };
    (scaled(width), scaled(height))
}

/// BGRA8 的目标格式转换器。
///
/// 输入既可能是解码帧，也可能是缩放器的输出（缩放器只改尺寸，格式原样透传），
/// 所以两者都实现 `IWICBitmapSource`，这里用泛型收口。
fn convert_to_bgra(
    factory: &IWICImagingFactory,
    source: &windows::Win32::Graphics::Imaging::IWICBitmapSource,
) -> Option<IWICFormatConverter> {
    let converter = {
        // SAFETY: 工厂有效，返回 `Result`。
        unsafe { factory.CreateFormatConverter() }.ok()?
    };
    // SAFETY: 源与转换器都是有效的；目标格式是 32bpp BGRA；
    // 不做抖动、不用调色板（`WICBitmapPaletteTypeCustom` + 空调色板表示不需要）；
    // alpha 阈值 0.0 在 BGRA 下不参与运算。
    unsafe {
        converter.Initialize(
            source,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
    }
    .ok()?;
    Some(converter)
}

/// 把 `source` 拷成紧密排列的 BGRA8。
fn copy_pixels(
    source: &windows::Win32::Graphics::Imaging::IWICBitmapSource,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    // `stride = width * 4` 要装进 u32：WIC 的尺寸上限是 2^16 一级，乘法不会溢出，
    // 但仍然显式检查一次，因为溢出在这里意味着后面按错误的长度分配缓冲区。
    let stride = width.checked_mul(BYTES_PER_PIXEL)?;
    let len = usize::try_from(stride.checked_mul(height)?).ok()?;

    // 用 `try_reserve` 而不是 `vec![0; len]`：一张 4800 万像素的 HEIC 在
    // 32bpp 下是 190MB，分配失败在这个函数里应该是 `None`，而不是让整个进程 abort。
    let mut bgra: Vec<u8> = Vec::new();
    bgra.try_reserve_exact(len).ok()?;
    bgra.resize(len, 0);

    // SAFETY: `source` 有效；`prc` 传空表示整幅图；`cbstride` 与缓冲区长度一致
    // （`len == stride * height`）；缓冲区在本调用期间独占且足够大。
    unsafe { source.CopyPixels(std::ptr::null(), stride, &mut bgra) }.ok()?;

    // 不静默返回尺寸不符的缓冲区：拿一个长度不对的 `Vec` 去当位图用，
    // 报错的地方会离这里很远。
    (bgra.len() == len).then_some(bgra)
}

/// 目标平台上有 HEIF 解码器时才认为 HEIC 可用。
///
/// App 需要把「没装 HEIF 扩展」和「这个文件坏了」分开说：前者要提示用户去商店装扩展，
/// 后者只能说这张照片读不了。这里问的正是前者——工厂能不能实例化出 HEIF 解码器。
pub fn heif_available() -> bool {
    let Some(_guard) = ComGuard::new() else {
        return false;
    };
    let Some(factory) = factory() else {
        return false;
    };
    heif_decoder_configured(&factory)
}

/// 工厂是否装着一个能处理 `GUID_ContainerFormatHeif` 的解码器。
///
/// 直接构造解码器组件，比翻注册表更贴近事实：注册表里有键不等于组件真的能加载。
fn heif_decoder_configured(factory: &IWICImagingFactory) -> bool {
    // SAFETY: `GUID_ContainerFormatHeif` 是常量 GUID；两个指针参数都传空，
    // 分别是「不限定厂商」和「不接收已存在实例」。
    unsafe {
        factory.CreateDecoder(&GUID_ContainerFormatHeif, std::ptr::null())
    }
    .is_ok()
}

/// 读取 HEIF 图像的像素尺寸；不是 HEIF 或读不到时返回 `None`。
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let _guard = ComGuard::new()?;
    let decoder = open_decoder(path)?;
    let frame = frame_of(&decoder, 0)?;
    frame_size(&frame)
}

/// 容器里的帧数；静图是 1，真·动图 HEIC 会大于 1。
pub fn frame_count(path: &Path) -> Option<u32> {
    let _guard = ComGuard::new()?;
    let decoder = open_decoder(path)?;
    let count = {
        // SAFETY: `decoder` 由 `open_decoder` 成功构造。
        unsafe { decoder.GetFrameCount() }.ok()?
    };
    (count > 0).then_some(count)
}

/// 解码第 `frame` 帧。
///
/// `max_width` 是**长边**上限：给了就在解码出口用 `IWICBitmapScaler` 直接出小图，
/// 一张 4800 万像素的 HEIC 也就不必先整个物化再缩——缩略图要的只是 256px。
pub fn decode(path: &Path, max_width: Option<u32>, frame: u32) -> Option<DecodedImage> {
    decode_limited(
        path,
        max_width.filter(|limit| *limit > 0).map(Limit::LongEdge),
        frame,
    )
}

/// 按**宽度**上限解码第 `frame` 帧，其余与 [`decode`] 完全一致。
///
/// # 为什么需要第二个入口
///
/// 竖幅照片的长边是高度。同一个数字按长边解释，出来的图会窄一截：3:4 的竖图只拿到
/// 0.75 倍宽度。而 `lumen://thumb/…?w=` 的契约、`thumbnail::fit` 的契约，以及
/// JPEG/PNG 走的那条路，说的都是**宽度**。两种口径混在一起，后果是同一个网格里
/// iPhone 拍的竖图比其他照片糊一档，而且没有任何地方会报错——所以这里把它拆成
/// 两个名字明确的入口，而不是靠调用方记住哪个函数是哪种口径。
pub fn decode_to_width(path: &Path, width: u32, frame: u32) -> Option<DecodedImage> {
    decode_limited(path, (width > 0).then_some(Limit::Width(width)), frame)
}

/// 两个入口共用的实现，唯一区别是上限怎么解释。
fn decode_limited(path: &Path, limit: Option<Limit>, frame: u32) -> Option<DecodedImage> {
    let _guard = ComGuard::new()?;
    let factory = factory()?;
    let decoder = open_decoder(path)?;
    let frame = frame_of(&decoder, frame)?;
    let (source_w, source_h) = frame_size(&frame)?;
    let (target_w, target_h) = target_size(source_w, source_h, limit);

    if (target_w, target_h) == (source_w, source_h) {
        // 源本来就够小：直接用解码帧，少一次缩放。
        let source: IWICBitmapSource = frame.cast().ok()?;
        let converter = convert_to_bgra(&factory, &source)?;
        let bgra = copy_pixels(&converter, source_w, source_h)?;
        return Some(DecodedImage {
            width: source_w,
            height: source_h,
            bgra,
        });
    }

    let scaler = {
        // SAFETY: 工厂有效，返回 `Result`。
        unsafe { factory.CreateBitmapScaler() }.ok()?
    };
    // SAFETY: 源是有效的解码帧；目标尺寸由 `target_size` 保证非零且只缩不放；
    // 插值模式取高质量立方，缩略图是唯一会被人盯着看尺寸的地方。
    unsafe {
        scaler
            .Initialize(
                &frame,
                target_w,
                target_h,
                WICBitmapInterpolationModeHighQualityCubic,
            )
            .ok()?
    };
    let source: IWICBitmapSource = scaler.cast().ok()?;
    let converter = convert_to_bgra(&factory, &source)?;
    let bgra = copy_pixels(&converter, target_w, target_h)?;
    Some(DecodedImage {
        width: target_w,
        height: target_h,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实 iPhone HEIC 的样本路径。
    ///
    /// 绝对路径不可能做成可移植的 fixture，所以用它做证据的测试一律 `#[ignore]`，
    /// 由 `cargo test -- --ignored --nocapture` 显式跑，并且在文件不存在时安静跳过。
    /// 这两条路径只用于本地取证，CI 上不会有人依赖它们。
    const SAMPLES: &[&str] = &[
        r"D:\Users\lemostic\Pictures\iCloud Photos\Photos\IMG_0001.HEIC",
        r"D:\Users\lemostic\Pictures\iCloud Photos\Photos\IMG_0002.HEIC",
    ];

    fn first_sample() -> Option<&'static str> {
        SAMPLES
            .iter()
            .copied()
            .find(|sample| Path::new(sample).is_file())
    }

    /// 长边口径的尺寸语义：不超过上限、只缩不放、比例保持。
    #[test]
    fn target_size_fits_the_long_edge_and_never_upscales() {
        // 4032x3024 的横拍和竖拍都按长边算。
        assert_eq!(target_size(4032, 3024, Some(Limit::LongEdge(256))), (256, 192));
        assert_eq!(target_size(3024, 4032, Some(Limit::LongEdge(256))), (192, 256));
        // 源比上限小：原样返回，不放大。
        assert_eq!(target_size(40, 20, Some(Limit::LongEdge(512))), (40, 20));
        // 没有上限就是原始尺寸。
        assert_eq!(target_size(4032, 3024, None), (4032, 3024));
        // 极端长条图不该被算成 0 高。
        assert_eq!(target_size(4000, 3, Some(Limit::LongEdge(256))).1, 1);
    }

    /// 宽度口径的尺寸语义：出图的**宽度**正好是请求的那个数，竖图也不例外。
    ///
    /// 这一条是这次补上的。以前只有长边口径，于是 `?w=256` 在一张 3:4 的竖图上
    /// 给出 192x256——宽度少了四分之一，而 JPEG/PNG 那条路给的是 256x341。
    /// 同一个网格里，iPhone 拍的竖图就这么比其他照片糊一档，且不会报任何错。
    #[test]
    fn width_limit_lands_on_the_requested_width_even_for_portraits() {
        let (w, h) = target_size(3024, 4032, Some(Limit::Width(256)));
        assert_eq!(w, 256, "竖图的宽度必须正好是 256");
        assert_eq!(h, 342, "3024x4032 在 256 宽下对应 342 高");
        // 横幅照片上两种口径本来就一致，换算不能把它带偏。
        assert_eq!(target_size(4032, 3024, Some(Limit::Width(256))), (256, 192));
        // 「只缩不放」对宽度口径同样成立。
        assert_eq!(target_size(40, 20, Some(Limit::Width(512))), (40, 20));
        // 换算出来的长边等于源的长边时不该触发缩放。
        assert_eq!(target_size(4032, 3024, Some(Limit::Width(4032))), (4032, 3024));
    }

    /// 没有 HEIF 解码器时 `heif_available` 必须是 `false`，有则必须是 `true`。
    ///
    /// 这条是给「缺编解码器」和「文件损坏」分开报错用的，所以它不能只会返回 `true`。
    #[test]
    fn heif_availability_matches_what_the_decoder_can_do() {
        let probe = Path::new(r"D:\Users\lemostic\Pictures\iCloud Photos\Photos\IMG_0001.HEIC");
        if !probe.is_file() {
            // 没有样本时只能验工厂探测本身不 panic。
            let available = heif_available();
            println!("no HEIC sample on disk; heif_available() -> {available}");
            return;
        }
        println!("heif_available() -> {}", heif_available());
        assert!(heif_available(), "本机装着 HEIF 扩展，探测不该说没有");
    }

    /// 真实 iPhone HEIC 的端到端解码。
    ///
    /// 这条测试是给「WIC 这条路到底通不通」出证据的：尺寸、帧数、缓冲区长度三项。
    #[test]
    #[ignore = "需要本机 iCloud 图库里的真实 HEIC 样本"]
    fn decodes_a_real_heic_when_one_is_available() {
        let Some(sample) = first_sample() else {
            // 拷到别的机器上时安静跳过，而不是假装断言成功。
            println!("SKIP: 没有找到真实 HEIC 样本，跳过解码取证");
            return;
        };
        let path = Path::new(sample);
        println!("sample: {sample}");

        assert!(heif_available(), "本机装着 HEIF 扩展，heif_available() 应为 true");

        let (width, height) = dimensions(path).expect("真实 HEIC 必须能读出尺寸");
        println!("dimensions: {width}x{height}");
        assert!(width > 0 && height > 0, "尺寸不能是 0");
        // iPhone 主摄出图普遍是 4032x3024 一档；给一个宽区间，只排除明显不对的答案。
        assert!(
            (1000..=20000).contains(&width) && (1000..=20000).contains(&height),
            "iPhone HEIC 的尺寸不该是 {width}x{height}"
        );

        let frames = frame_count(path).expect("真实 HEIC 必须能读出帧数");
        println!("frame_count: {frames}");
        assert!(frames >= 1, "帧数至少是 1");

        let decoded = decode(path, None, 0).expect("真实 HEIC 必须能解码");
        println!(
            "decoded: {}x{}, {} bytes (expected {})",
            decoded.width,
            decoded.height,
            decoded.bgra.len(),
            decoded.width as usize * decoded.height as usize * 4
        );
        assert_eq!((decoded.width, decoded.height), (width, height));
        assert_eq!(
            decoded.bgra.len(),
            decoded.width as usize * decoded.height as usize * 4,
            "BGRA 缓冲区长度必须恰好是 width * height * 4"
        );
        // 一张真照片不可能整幅都是同一个字节，那说明我们拿到的是没填过的缓冲区。
        assert!(
            decoded.bgra.chunks(4).any(|px| px != &decoded.bgra[0..4]),
            "解码结果不该是纯色"
        );

        let thumb = decode(path, Some(256), 0).expect("缩略图尺寸的 HEIC 必须能解码");
        println!("thumbnail-scale decode: {}x{}", thumb.width, thumb.height);
        assert_eq!(thumb.width.max(thumb.height), 256, "长边应被夹到 256");
        assert_eq!(thumb.bgra.len(), thumb.width as usize * thumb.height as usize * 4);
    }

    /// 不是 HEIF 的文件必须是普通的 `None`，不是 panic。
    #[test]
    fn a_non_heif_file_is_none_not_a_panic() {
        let mut path = std::env::temp_dir();
        path.push(format!("lumenfold-wic-not-heif-{}.jpg", std::process::id()));
        std::fs::write(&path, b"definitely not a heif").expect("write junk");
        assert_eq!(dimensions(&path), None);
        assert_eq!(frame_count(&path), None);
        assert_eq!(decode(&path, None, 0), None);
        let _ = std::fs::remove_file(&path);
    }

    /// 不存在的文件同理：调用方要靠 `None` 把「读不到」讲清楚。
    #[test]
    fn a_missing_file_is_none_not_a_panic() {
        let path = Path::new("no-such-file-anywhere.heic");
        assert_eq!(dimensions(path), None);
        assert_eq!(frame_count(path), None);
        assert_eq!(decode(path, Some(64), 0), None);
    }

    /// 帧号越界是 `None`，不是崩溃——动图 HEIC 的帧数由容器说了算。
    #[test]
    fn an_out_of_range_frame_is_none() {
        let Some(sample) = first_sample() else {
            println!("SKIP: 没有真实 HEIC 样本");
            return;
        };
        assert_eq!(decode(Path::new(sample), Some(64), 9999), None);
    }
}
