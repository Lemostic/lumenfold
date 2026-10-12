//! The `lumen://` custom URI scheme that serves thumbnails to the WebView.
//!
//! One shape, one handler:
//!
//! ```text
//! lumen://thumb/<assetId>?w=<pixels>
//! ```
//!
//! Every response is either a real encoded thumbnail or a 404 with a
//! `{"kind", "message"}` body matching the IPC error contract. There is no
//! placeholder image: a tile that cannot be decoded renders as a "无法解码"
//! tile in the UI, which is a truer answer than a grey box that looks like a
//! successfully loaded photo.
//!
//! Encoding is pure-Rust `image` (full decode, then `resize`), except for HEIC:
//! `image` has no HEIF decoder, so those go through the OS codec (WIC) and are
//! re-encoded with the same `image` encoders as everything else. This is the
//! no-native-dependency fallback — ADR-0001's libvips `shrink-on-load` pipeline
//! replaces it in M1.

use std::sync::Arc;

use tauri::http::{header, Request, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::LumenError;
use crate::library::{CachedThumb, SharedLibrary};

/// Scheme registered on the builder.
pub const SCHEME: &str = "lumen";

/// Default thumbnail width when the request omits `?w=`.
const DEFAULT_WIDTH: u32 = 256;

/// Bounds for `?w=`. Below 64 the UI is just upscaling; above 1024 the grid has
/// already left the screen.
const MIN_WIDTH: u32 = 64;
const MAX_WIDTH: u32 = 1024;

/// JPEG quality for opaque sources.
const JPEG_QUALITY: u8 = 80;

/// `lumen://full/...` 重新编码 HEIC 时的长边上限。
///
/// 看全图要的是屏幕上的清晰度，不是归档原件。iPhone 的 HEIC 是 3024×4032
/// （本次实测值），本身就在这个上限之内，所以这个数字的作用是给未来的
/// 4800 万像素 HEIC 封顶：再往上加像素，屏幕上多出来的细节肉眼看不出来，
/// 而 WebView2 那边要多留几十 MB 的位图解码缓冲——那正是 Lightbox 翻页卡顿的来源。
/// 另外这条路径只在容器确实比上限大时才会触发缩放，普通照片一个像素都不会动。
const FULL_IMAGE_MAX_LONG_EDGE: u32 = 4096;

/// Registers the `lumen` scheme. Must be called before `.setup`.
///
/// # 为什么响应要绕一圈后台线程
///
/// `register_asynchronous_uri_scheme_protocol` 这个名字只保证**响应**可以晚点给，
/// 它不保证**回调本身**在别的线程上跑。tauri 把这个闭包原样交给 wry，wry 在
/// WebView2 的 `WebResourceRequested` 回调里同步调用它，而这个回调跑在创建
/// WebView 的线程上——也就是 UI 线程。于是 `handle()` 里的一次完整解码、缩放、
/// 重编码（2400 万像素实测 100–200ms）会直接压在渲染线程上：一屏几十个格子就是
/// 肉眼可见的卡死，滚动、翻页、切视图全部停住。扫描进行中时磁盘和 CPU 都被扫描
/// 占着，每次解码更慢，卡顿也就更长——「图库扫描导致主界面卡顿」有一半是这里。
///
/// 所以真正的活交给 `worker_pool`：响应本身是 `Send` 的（tauri 侧的
/// `UriSchemeResponder` 就是 `Box<dyn FnOnce(_) + Send>`），从 worker 线程发回去
/// 完全合法，UI 线程只剩一次入队。
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol(SCHEME, move |ctx, request, responder| {
        // `ctx` 只借到 `&AppHandle`，而闭包要求 `'static`，所以先克隆一份句柄。
        // 克隆的是 `AppHandle`（内部是 Arc），不是整个应用状态。
        let app = ctx.app_handle().clone();
        crate::worker_pool::shared().dispatch(move || {
            responder.respond(handle(&app, &request));
        });
    })
}

/// Resolves one `lumen://` request into a full HTTP response.
fn handle<R: Runtime>(app: &AppHandle<R>, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    // The full URI is used rather than just the path: the resource kind sits in
    // the authority position, so `path` alone would lose it.
    match route(app, &request.uri().to_string()) {
        Ok(thumb) => image_response(thumb),
        Err(error) => error_response(&error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceKind {
    Thumb,
    Full,
}

/// Parses both thumbnail and original-image requests. The browser-facing
/// source is `http://lumen.localhost/...`; Tauri hands the restored
/// `lumen://localhost/...` URI to this handler.
fn parse_resource_route(uri: &str) -> Result<(ResourceKind, String, Option<u32>), LumenError> {
    let (path, query) = match uri.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (uri, None),
    };

    let rest = path
        .strip_prefix(&format!("{SCHEME}://"))
        .or_else(|| path.strip_prefix(&format!("{SCHEME}:")))
        .unwrap_or(path)
        .trim_start_matches('/');

    let mut segments = rest.split('/');
    if segments.clone().next() == Some("localhost") {
        segments.next();
    }

    let kind = match segments.next() {
        Some("thumb") => ResourceKind::Thumb,
        Some("full") => ResourceKind::Full,
        Some(other) => {
            return Err(LumenError::unsupported(format!(
                "unsupported {SCHEME}:// resource {other:?}; expected /thumb or /full"
            )))
        }
        None => {
            return Err(LumenError::unsupported(format!(
                "unsupported {SCHEME}:// path; expected /thumb/<assetId> or /full/<assetId>"
            )))
        }
    };

    let Some(asset_id) = segments.next().filter(|value| !value.is_empty()) else {
        return Err(LumenError::unsupported("missing asset id in image request"));
    };
    let width_from_path = match segments.next() {
        Some(segment) => Some(segment.parse::<u32>().map_err(|_| {
            LumenError::unsupported(format!("malformed {SCHEME}:// image width"))
        })?),
        None => None,
    };
    if segments.next().is_some() {
        return Err(LumenError::unsupported(format!("malformed {SCHEME}:// image path")));
    }
    let width_from_query = query.and_then(|query| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix("w="))
            .and_then(|value| value.parse::<u32>().ok())
    });
    Ok((kind, asset_id.to_string(), width_from_path.or(width_from_query)))
}
/// Parses `lumen://localhost/thumb/<assetId>?w=<pixels>` into the asset id and a
/// clamped width.
///
/// # 这个 handler 收到的 URL 长什么样
///
/// WebView2 不认识自定义协议，wry 把它们映射到 `http://<协议名>.localhost/`，
/// 按 `http://lumen.*` 这个过滤器用 `WebResourceRequested` 拦下来，
/// 再把地址还原成 `lumen://localhost/<path>` 交给本函数。
///
/// 注意那个 `localhost` 主机名是**被还原回去的固定前缀**，前端并不需要写它。
/// 前端真正要写的是 http 源 `http://lumen.localhost/thumb/<id>`：
/// wry 只对顶层导航做协议改写，子资源（也就是 `<img>`）不经过那一步，
/// 写成 `lumen://...` 的话 WebView2 判定协议未知，请求根本到不了这里，
/// 屏幕上只会留下一个破图。之前全库缩略图都是坏图标，根因就在这里。
///
/// 两种形状这里都接受：多一层兼容的代价只是一行判断，
/// 而少一层的代价是整库缩略图不可用。
fn parse_thumb_route(uri: &str) -> Result<(String, u32), LumenError> {
    let (kind, asset_id, requested_width) = parse_resource_route(uri)?;
    if kind != ResourceKind::Thumb {
        return Err(LumenError::unsupported("only /thumb routes accept a width"));
    }
    let width = requested_width
        .unwrap_or(DEFAULT_WIDTH)
        .clamp(MIN_WIDTH, MAX_WIDTH);
    Ok((asset_id, width))
}

/// Looks up and encodes the thumbnail, or explains why there is none.
///
/// # 两层缓存
///
/// 内存里是 `LibraryState::thumbs`（LRU，几千条），磁盘上是
/// [`crate::thumbstore::ThumbStore`]（目录，字节预算内无限）。顺序是
/// 内存 → 磁盘 → 解码，两层都命中不了才真的去开照片。
///
/// 磁盘那层是「打开图库不用重新解码一遍」的答案：内存 LRU 随进程一起没了，
/// 而用户每天都要重看昨天看过的那些照片。没有它，启动后的第一屏要把整屏缩略图
/// 重新解码一次，几万张的库就是几分钟的等待。
fn route<R: Runtime>(app: &AppHandle<R>, uri: &str) -> Result<CachedThumb, LumenError> {
    let (kind, asset_id, _requested_width) = parse_resource_route(uri)?;
    if kind == ResourceKind::Full {
        return full_image(app, &asset_id);
    }
    // Reuse the thumbnail parser for the clamped width. Keeping that parser as
    // the single width contract also preserves its route tests.
    let width = parse_thumb_route(uri)?.1;
    let cache_key = format!("{asset_id}@{width}");

    // Encode outside the lock: decoding a 24 MP JPEG takes long enough that
    // holding the library mutex would stall `library_stats` for every other
    // command.
    let state = app.state::<SharedLibrary>();
    if let Some(cached) = state.lock().expect("library state poisoned").thumbs.get(&cache_key) {
        return Ok(cached.clone());
    }

    // 磁盘命中：回填内存层，这样反复上下滚动时不用每次都读文件。
    // 文件读取很便宜，但缩略图网格会同时要几十张，而这些都是同步阻塞协议线程的。
    let store = app.state::<crate::thumbstore::ThumbStore>();
    if let Some(hit) = store.get(&cache_key) {
        let thumb = CachedThumb {
            bytes: hit.bytes,
            content_type: hit.content_type,
        };
        state
            .lock()
            .expect("library state poisoned")
            .put_thumb(cache_key, thumb.clone());
        return Ok(thumb);
    }

    let path = {
        let guard = state.lock().expect("library state poisoned");
        let position = *guard
            .index
            .get(&asset_id)
            .ok_or_else(|| LumenError::io(format!("unknown asset id {asset_id:?}")))?;
        guard.assets[position].path.clone()
    };

    // 同一个格子被并发请求多次时只允许一次解码，其余的等结果。
    // 界面重排（改密度、切视图、窗口缩放）时同一个 URL 会被同时要好几次，
    // 没有这道闸就是并排做三四次一模一样的全尺寸解码。
    let in_flight = state
        .lock()
        .expect("library state poisoned")
        .decoding
        .entry(cache_key.clone())
        .or_default()
        .clone();

    let outcome = in_flight.get_or_init(|| encode(&path, width)).clone();

    // 无论成功失败都要清掉这条在途记录：留着会让一次偶发的失败变成永久失败，
    // 而这个格子之后可能还会被滚回来。
    let mut guard = state.lock().expect("library state poisoned");
    guard.decoding.remove(&cache_key);
    if let Ok(thumb) = &outcome {
        guard.put_thumb(cache_key.clone(), thumb.clone());
        drop(guard);
        // 存盘放在释放图库锁之后：磁盘 IO 不该把 library_stats 一起拖住。
        store.put(&cache_key, &thumb.bytes, thumb.content_type);
        return Ok(thumb.clone());
    }
    outcome
}

/// Serves original bytes at their native dimensions for the photo wall and
/// Lightbox. The browser handles the supported image format directly.
///
/// # HEIC 是唯一的例外，而且是必须的例外
///
/// 这个函数以前对所有格式一视同仁：原样读出来，配上 `content_type_for` 猜的 MIME。
/// 对 HEIC 来说最后这一步是坏的：**WebView2 是 Chromium，它渲染不了 `image/heic`**。
/// 字节会完整地送到 `<img>` 手里，然后什么也不显示——用户看到的就是一张点不开的照片，
/// 而请求本身是 200，日志里一点异常都没有。
///
/// 所以 HEIC/HEIF 在这里解码之后重新编码成 JPEG，`content_type` 报 `image/jpeg`。
/// 这不是「顺便压一下」，是因为前端根本没有能显示 HEIC 的解码器。
///
/// 其他格式一个字节都不变地流过去：那条路上浏览器是能解码的一方，
/// 动它只会白白多一次转码和一个更差的 JPEG。
fn full_image<R: Runtime>(app: &AppHandle<R>, asset_id: &str) -> Result<CachedThumb, LumenError> {
    let path = {
        let state = app.state::<SharedLibrary>();
        let guard = state.lock().expect("library state poisoned");
        let position = *guard
            .index
            .get(asset_id)
            .ok_or_else(|| LumenError::io(format!("unknown asset id {asset_id:?}")))?;
        guard.assets[position].path.clone()
    };

    if is_heif(&path) {
        return encode_heic(&path, FULL_IMAGE_MAX_LONG_EDGE).ok_or_else(|| {
            LumenError::unsupported(format!(
                "cannot decode {}: 系统没有可用的 HEIF 解码器，或者文件已损坏",
                path.display()
            ))
        });
    }

    let bytes = std::fs::read(&path)
        .map_err(|err| LumenError::io(format!("cannot read {}: {err}", path.display())))?;
    Ok(CachedThumb {
        bytes: Arc::new(bytes),
        content_type: content_type_for(&path),
    })
}

/// 按扩展名判断是不是 HEIF 家族的容器。
///
/// 用扩展名而不是嗅探魔数，是因为这个判断只用来决定「要不要走那条更贵的 WIC 路径」：
/// 判错一次的代价是一张图多花几十毫秒，而不是给出错误的答案。真正的正确性由
/// `lumenfold-wic` 负责——它认的 `ftyp` 品牌不对就返回 `None`，然后我们如实报错。
fn is_heif(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("heic") | Some("heif")
    )
}

fn content_type_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("heic") => "image/heic",
        Some("avif") => "image/avif",
        _ => "image/jpeg",
    }
}

/// Decodes, resizes to `width` preserving aspect, and re-encodes.
///
/// # 三条路，按代价从低到高排
///
/// 1. JPEG 的缩放解码（[`encode_jpeg_shrunk`]），绝大多数的照片走这条。
/// 2. `image` 的通用路径（[`encode_generic`]），PNG/WebP/TIFF/BMP/GIF 走这条。
/// 3. 系统编解码层 WIC（[`encode_heic`]），只有前两条都失败时才走。
///
/// 第 3 条是给 HEIC 补的：`image` 0.25 没有 HEIF 解码器，所以 iPhone 照片以前在
/// 第 2 条上必定失败，格子里就是「无法解码」。走到这里才回头去问系统，
/// 是因为每一步的代价递增——WIC 要先初始化 COM 再实例化解码器，
/// 不能让每一张 JPEG 都白付一次。
///
/// 原始错误会一路带到最后：HEIC 也读不出来的时候，报的是第 2 条那句
/// 「cannot decode（真实原因）」，而不是一句没有信息量的「WIC 失败了」。
pub fn encode(path: &std::path::Path, width: u32) -> Result<CachedThumb, LumenError> {
    match encode_jpeg_shrunk(path, width) {
        Ok(Some(thumb)) => return Ok(thumb),
        // 不是 JPEG，或者这个 JPEG 解不动。两种情况都退回通用路径，
        // 表现和以前一样，而不是把一张照片变成一个破图。
        Ok(None) => {}
        Err(_) => {}
    }
    let original = match decode_with_image(path) {
        Ok(image) => image,
        Err(err) => {
            return encode_heic(path, width)
                // 没有 WIC 编解码器时把原始错误原样报出去：那句话里带着真实原因。
                .ok_or(err)
        }
    };
    Ok(encode_decoded(&original, width))
}

/// JPEG 专用的缩放解码。返回 `Ok(None)` 表示这个文件不该走这条路。
///
/// # 为什么 JPEG 要单独走一条路
///
/// `image` 0.25 的 JPEG 解码走 zune-jpeg，而它没有「按比例缩小解码」的入口，
/// 只能把 1200 万像素全部解出来再缩。对缩略图来说这意味着每张图都要付一次
/// 全尺寸解码的钱：一张 4160×2768 的照片约 100–200ms，一屏 30 多个格子就是好几秒，
/// 而且这些解码挤在一起会把界面卡住——正是「感觉卡顿」的来源。
///
/// `jpeg-decoder` 可以在 DCT 域上按 1/2、1/4、1/8 直接出图：解码 1/8 的像素量只有
/// 全量的 1/64，之后再补一次精确 resize。这条路对 JPEG（也就是绝大多数照片）
/// 快一个数量级，其他格式仍然走 `image`，行为不变。
///
/// `Decoder::scale` 要的是**目标尺寸**而不是倍率，它自己挑最小的 IDCT 档位：
/// 要 320px 宽就拿 1/4 或 1/8 直接出图，解码的像素量只有全量的 1/16 或 1/64。
fn encode_jpeg_shrunk(
    path: &std::path::Path,
    width: u32,
) -> Result<Option<CachedThumb>, LumenError> {
    use std::io::BufReader;

    let file = std::fs::File::open(path)
        .map_err(|err| LumenError::io(format!("cannot open {}: {err}", path.display())))?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));

    let fail = |err: jpeg_decoder::Error| {
        LumenError::unsupported(format!("cannot decode {}: {err}", path.display()))
    };

    decoder.read_info().map_err(fail)?;
    let info = decoder.info().ok_or_else(|| fail(
        jpeg_decoder::Error::Io(std::io::Error::other("no JPEG frame header")),
    ))?;
    // 灰度、16 位和 CMYK 的字节布局不一样，交给通用路径处理，
    // 这里只走最常见也最值得优化的一条：RGB24。
    if info.pixel_format != jpeg_decoder::PixelFormat::RGB24 {
        return Ok(None);
    }

    let target = fit(info.width as u32, info.height as u32, width);
    let (out_w, out_h) = decoder
        .scale(target.0.min(u32::from(u16::MAX)) as u16, target.1.min(u32::from(u16::MAX)) as u16)
        .map_err(fail)?;
    let pixels = decoder.decode().map_err(fail)?;

    let decoded =
        image::RgbImage::from_raw(out_w as u32, out_h as u32, pixels)
            .ok_or_else(|| fail(jpeg_decoder::Error::Io(std::io::Error::other(
                "decoded size does not match the buffer",
            ))))?;

    Ok(Some(encode_rgb(&decoded, width)))
}

/// The general path: full decode through `image`, then resize.
///
/// PNG in, PNG out for anything with an alpha channel; JPEG everywhere else.
/// Decoding is done through the concrete `DynamicImage`, because the typed
/// decoders cannot answer the alpha question.
///
/// 这里保留原来的签名和错误处理，是因为它要能被单独调用：它不知道自己为什么被调用，
/// 所以「走 WIC 再试一次」这件事放在 [`encode`] 里，而不是塞进这个函数。
pub fn encode_generic(path: &std::path::Path, width: u32) -> Result<CachedThumb, LumenError> {
    Ok(encode_decoded(&decode_with_image(path)?, width))
}

/// 用 `image` 把文件解成一张位图；解不动就如实报错。
fn decode_with_image(path: &std::path::Path) -> Result<image::DynamicImage, LumenError> {
    image::ImageReader::open(path)
        .map_err(|err| LumenError::io(format!("cannot open {}: {err}", path.display())))?
        .with_guessed_format()
        .map_err(|err| LumenError::unsupported(format!("cannot guess format: {err}")))?
        .decode()
        .map_err(|err| LumenError::unsupported(format!("cannot decode {}: {err}", path.display())))
}

/// 已经解好的一张图：按 `width` 缩放后编码。
///
/// 缩略图和 HEIC 那条路共用它，所以两条路的输出在视觉上是一致的——
/// 同一个 `Triangle` 滤波器、同一个 JPEG 质量、同一套「有 alpha 就出 PNG」的判断。
fn encode_decoded(image: &image::DynamicImage, width: u32) -> CachedThumb {
    let (source_w, source_h) = (image.width(), image.height());
    let target = fit(source_w, source_h, width);
    let resized = if target == (source_w, source_h) {
        image.clone()
    } else {
        image.resize_exact(target.0, target.1, image::imageops::FilterType::Triangle)
    };

    if resized.color().has_alpha() {
        return encode_png(&resized);
    }
    encode_jpeg(&resized)
}

/// HEIC/HEIF 的解码路径：交给操作系统的 WIC 编解码层。
///
/// # 为什么不能让 WebView2 自己看图
///
/// 缩略图要能被 `<img>` 直接渲染，而 WebView2 是 Chromium：它**不认识
/// `image/heic`**。把原始字节交给它，`<img>` 只会是一片空白，然后渲染成坏图。
/// 所以这里必须自己解出来再重新编码成 JPEG——不是为了省字节，是因为
/// 前端根本没有能显示 HEIC 的解码器。
///
/// # 为什么复用 `encode_rgb` / `encode_decoded`
///
/// 再写一套编码器意味着 HEIC 缩略图的色调、尺寸、字节大小都和别的格式不一样，
/// 而这种差异没有人会去测。这里把 WIC 的 BGRA 转成 RGB，然后喂给和 PNG/JPEG
/// 完全相同的那条编码路径。
///
/// 返回 `None` 表示这条路走不通（不是 HEIF、机器上没装 HEIF 扩展、文件损坏），
/// 调用方会退回原始错误。不会 panic：`lumenfold-wic` 的每一个失败路径都是 `None`。
fn encode_heic(path: &std::path::Path, width: u32) -> Option<CachedThumb> {
    // 这里用宽度口径的入口，而不是 `decode(path, Some(width), 0)`：后者的
    // `Some(width)` 是**长边**上限，而竖幅照片的长边是高度，于是 `?w=256` 在
    // 一张 3:4 的竖图上只给出 192×256，同时同一张图走 JPEG 那条路给出 256×341。
    // 结果就是同一个网格里 iPhone 拍的竖图比别的照片糊一档，且不会报任何错。
    let decoded = lumenfold_wic::decode_to_width(path, width, 0)?;
    let rgb = bgra_to_rgb(&decoded.bgra, decoded.width, decoded.height)?;
    // `encode_rgb` 自己会按 `fit` 夹一次尺寸：WIC 那边已经缩过一次，
    // 这里是补掉取整误差的第二遍，和 JPEG 缩放解码那条路完全同一个套路。
    Some(encode_rgb(&rgb, width))
}

/// WIC 的 BGRA8 缓冲区转成 `image` 的 RGB8。
///
/// 丢掉的 alpha 通道对照片没有意义：HEIC 的 4 个通道里 alpha 通常是常量 255，
/// 而 `encode_decoded` 见到 alpha 就会去出 PNG——给一张不透明的照片出 PNG
/// 会让缩略图体积翻好几倍，还拖慢网格加载。
fn bgra_to_rgb(bgra: &[u8], width: u32, height: u32) -> Option<image::RgbImage> {
    // WIC 的输出是紧密排列的 BGRA，长度必须正好对得上；对不上就当作拿不到，
    // 而不是按错误的跨距去读一段越界的像素。
    let expected = width as usize * height as usize * 4;
    if bgra.len() != expected {
        return None;
    }
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in bgra.chunks_exact(4) {
        // 每个像素恰好 4 字节（`chunks_exact`），索引不会越界。
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    image::RgbImage::from_raw(width, height, rgb)
}

/// 缩放解码之后把剩下的那点误差补掉。
///
/// 只向下：一张 40px 的图标放大到 512px 只会得到一张模糊的大图，
/// 那不是缩略图该有的样子，所以目标宽度被源图宽度夹住。
pub(crate) fn fit(source_w: u32, source_h: u32, width: u32) -> (u32, u32) {
    let target_w = width.min(source_w).max(1);
    let target_h =
        (u64::from(source_h) * u64::from(target_w) / u64::from(source_w).max(1)).max(1) as u32;
    (target_w, target_h)
}

fn encode_rgb(source: &image::RgbImage, width: u32) -> CachedThumb {
    let target = fit(source.width(), source.height(), width);
    let resized = if target == (source.width(), source.height()) {
        source.clone()
    } else {
        image::imageops::resize(
            source,
            target.0,
            target.1,
            image::imageops::FilterType::Triangle,
        )
    };
    encode_jpeg(&image::DynamicImage::ImageRgb8(resized))
}

fn encode_jpeg(image: &image::DynamicImage) -> CachedThumb {
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        std::io::Cursor::new(&mut out),
        JPEG_QUALITY,
    );
    // 往一个 `Vec<u8>` 里写 JPEG 只会因为内存不足失败，那不是能在这里处理的情况。
    encoder
        .encode_image(image)
        .expect("writing a JPEG into a Vec<u8> cannot fail");
    CachedThumb {
        bytes: Arc::new(out),
        content_type: "image/jpeg",
    }
}

fn encode_png(image: &image::DynamicImage) -> CachedThumb {
    let mut out = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("writing a PNG into a Vec<u8> cannot fail");
    CachedThumb {
        bytes: Arc::new(out),
        content_type: "image/png",
    }
}
fn image_response(thumb: CachedThumb) -> Response<Vec<u8>> {
    let length = thumb.bytes.len();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, thumb.content_type)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .header(header::CONTENT_LENGTH, length)
        .body(thumb.bytes.as_ref().clone())
        .expect("thumbnail response is well formed")
}

/// A 404 carrying the same `{kind, message}` shape the IPC errors use, so the
/// frontend can render one message for both.
fn error_response(error: &LumenError) -> Response<Vec<u8>> {
    let body = serde_json::to_vec(&error)
        .unwrap_or_else(|_| br#"{"kind":"io","message":"thumbnail request failed"}"#.to_vec());
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, body.len())
        .body(body)
        .expect("error response is well formed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(tag: &str) -> TempFile {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "lumenfold-thumb-{}-{}-{}",
                std::process::id(),
                tag,
                n
            ));
            TempFile(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn encodes_opaque_sources_as_jpeg_at_the_requested_width() {
        let file = TempFile::new("jpeg");
        image::RgbImage::from_pixel(800, 400, image::Rgb([120, 30, 200]))
            .save_with_format(&file.0, image::ImageFormat::Png)
            .expect("write rgb png");

        let thumb = encode(&file.0, 256).expect("encode");
        assert_eq!(thumb.content_type, "image/jpeg");
        let decoded = image::load_from_memory(&thumb.bytes).expect("decode thumb");
        assert_eq!(decoded.width(), 256);
        // Aspect ratio is preserved: 800x400 halves to 256x128.
        assert_eq!(decoded.height(), 128);
    }

    #[test]
    fn encodes_sources_with_alpha_as_png() {
        let file = TempFile::new("alpha");
        image::RgbaImage::from_pixel(100, 100, image::Rgba([1, 2, 3, 128]))
            .save_with_format(&file.0, image::ImageFormat::Png)
            .expect("write rgba png");

        let thumb = encode(&file.0, 64).expect("encode");
        assert_eq!(thumb.content_type, "image/png");
        let decoded = image::load_from_memory(&thumb.bytes).expect("decode thumb");
        assert_eq!((decoded.width(), decoded.height()), (64, 64));
    }

    /// 缩放解码是缩略图速度的来源，所以它必须真的走通并给出正确的尺寸。
    #[test]
    fn large_jpegs_take_the_shrunk_decode_path() {
        let file = TempFile::new("big-jpeg");
        // 2000x1000：请求 250px 宽时 IDCT 只能挑 1/8（250x125），
        // 剩下的误差由精确 resize 补掉。走通用路径也能得到 250x125，
        // 但那要先解完 200 万像素——测试守的是尺寸，不是耗时。
        image::RgbImage::from_pixel(2000, 1000, image::Rgb([40, 90, 160]))
            .write_to(
                &mut std::fs::File::create(&file.0).expect("create"),
                image::ImageFormat::Jpeg,
            )
            .expect("write jpeg");

        let thumb = encode(&file.0, 250).expect("encode");
        assert_eq!(thumb.content_type, "image/jpeg");
        let decoded = image::load_from_memory(&thumb.bytes).expect("decode thumb");
        assert_eq!((decoded.width(), decoded.height()), (250, 125));

        // 缩略图必须比原图小得多，否则说明根本没有缩。
        let bytes = std::fs::metadata(&file.0).expect("stat").len();
        assert!(
            (thumb.bytes.len() as u64) < bytes / 4,
            "a 250px thumbnail should be far smaller than the source: {} vs {bytes}",
            thumb.bytes.len()
        );
    }

    /// 目标尺寸接近原图时不能退回放大。
    #[test]
    fn shrunk_decode_still_clamps_to_the_source_size() {
        let file = TempFile::new("clamp-jpeg");
        image::RgbImage::from_pixel(120, 60, image::Rgb([7, 7, 7]))
            .write_to(
                &mut std::fs::File::create(&file.0).expect("create"),
                image::ImageFormat::Jpeg,
            )
            .expect("write jpeg");

        let decoded = image::load_from_memory(&encode(&file.0, 512).expect("encode").bytes)
            .expect("decode thumb");
        assert_eq!((decoded.width(), decoded.height()), (120, 60));
    }

    #[test]
    fn fit_only_ever_scales_down() {
        assert_eq!(fit(4000, 2000, 300), (300, 150));
        assert_eq!(fit(200, 100, 512), (200, 100));
        // 四舍五入不该把高度算成 0。
        assert_eq!(fit(4000, 10, 320).1, 1);
    }

    #[test]
    fn small_sources_are_not_upscaled() {
        let file = TempFile::new("tiny");
        image::RgbImage::from_pixel(40, 20, image::Rgb([9, 9, 9]))
            .save_with_format(&file.0, image::ImageFormat::Png)
            .expect("write tiny png");
        let thumb = encode(&file.0, 512).expect("encode");
        let decoded = image::load_from_memory(&thumb.bytes).expect("decode thumb");
        assert_eq!((decoded.width(), decoded.height()), (40, 20));
    }

    #[test]
    fn undecodable_file_is_an_error_not_a_placeholder() {
        let file = TempFile::new("junk");
        std::fs::File::create(&file.0)
            .expect("create")
            .write_all(b"definitely not an image")
            .expect("write junk");
        assert!(matches!(encode(&file.0, 256), Err(LumenError::Unsupported(_))));

        let response = error_response(&LumenError::unsupported("cannot decode"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value =
            serde_json::from_slice(response.body()).expect("json body");
        assert_eq!(body["kind"], "unsupported");
        assert_eq!(body["message"], "cannot decode");
    }

    /// 真实 iPhone HEIC 的缩略图取证。
    ///
    /// 绝对路径不可能做成可移植的 fixture，所以 `#[ignore]`，样本不在时安静跳过。
    /// 它守的是这次修复在下游的这一半：`image` 解不动的文件必须能经 WIC 出图，
    /// 而且要出成 `<img>` 能渲染的 JPEG，而不是原样把 HEIC 字节丢给 WebView2。
    #[test]
    #[ignore = "需要本机 iCloud 图库里的真实 HEIC 样本"]
    fn encodes_a_real_heic_thumbnail_when_one_is_available() {
        const SAMPLE: &str = r"D:\Users\lemostic\Pictures\iCloud Photos\Photos\IMG_0001.HEIC";
        let path = std::path::Path::new(SAMPLE);
        if !path.is_file() {
            println!("SKIP: 没有找到真实 HEIC 样本");
            return;
        }

        // 先确认这条路不是「反正也走不到」：`image` 对 HEIC 必须真的失败。
        assert!(
            decode_with_image(path).is_err(),
            "`image` 不该能解 HEIC —— 能解就说明这条回退路径根本没被走到"
        );

        let thumb = encode(path, 256).expect("HEIC 必须能出缩略图");
        println!(
            "thumbnail: content_type={}, {} bytes",
            thumb.content_type,
            thumb.bytes.len()
        );
        // WebView2 认这个 MIME，认不得 image/heic。
        assert_eq!(thumb.content_type, "image/jpeg");

        let decoded = image::load_from_memory(&thumb.bytes).expect("缩略图必须能被 JPEG 解码");
        println!("thumbnail pixels: {}x{}", decoded.width(), decoded.height());
        // 源是竖拍 3024x4032，而 `?w=` 的全站契约是**宽度**——
        // `fit()` 也是按宽度算的（`target_w = width.min(source_w)`）。
        // 所以哪怕长边是高度，256 的请求也必须给出一张宽 256 的图：
        // 3024x4032 按宽 256 换算得到高 342。
        //
        // 这条断言是这一步新加的。以前 WIC 那边把 `?w=` 当**长边**上限，
        // 竖图只出 192x256，而同一张图走 JPEG 那条路出的是 256x341——
        // 同一个网格里两种格式糊得不一样，且不会报任何错。
        assert_eq!(decoded.width(), 256, "竖图的宽度也要正好是请求的那个数");
        assert_eq!((decoded.width(), decoded.height()), (256, 342));
        assert!(
            (decoded.width() as f64 / decoded.height() as f64 - 0.75).abs() < 0.01,
            "缩略图比例必须跟源一致"
        );

        // 缩略图必须真的比原图小：一张 12MP 的 HEIC 通常是 1–3MB。
        let source_bytes = std::fs::metadata(path).expect("stat").len();
        println!("source: {source_bytes} bytes");
        assert!(
            (thumb.bytes.len() as u64) < source_bytes,
            "缩略图不该比原图还大：{} vs {source_bytes}",
            thumb.bytes.len()
        );
    }

    /// HEIC 的全图路径必须交出 JPEG，因为 WebView2 没有 HEIF 解码器。
    #[test]
    #[ignore = "需要本机 iCloud 图库里的真实 HEIC 样本"]
    fn full_size_heic_becomes_a_jpeg_when_one_is_available() {
        const SAMPLE: &str = r"D:\Users\lemostic\Pictures\iCloud Photos\Photos\IMG_0001.HEIC";
        let path = std::path::Path::new(SAMPLE);
        if !path.is_file() {
            println!("SKIP: 没有找到真实 HEIC 样本");
            return;
        }

        let thumb = encode_heic(path, FULL_IMAGE_MAX_LONG_EDGE).expect("HEIC 必须能出全图");
        println!(
            "full size: content_type={}, {} bytes",
            thumb.content_type,
            thumb.bytes.len()
        );
        assert_eq!(thumb.content_type, "image/jpeg");

        let decoded = image::load_from_memory(&thumb.bytes).expect("全图必须能被 JPEG 解码");
        println!("full pixels: {}x{}", decoded.width(), decoded.height());
        // 源是 3024x4032，本身就在 4096 的上限之内，所以尺寸原样保留。
        assert_eq!((decoded.width(), decoded.height()), (3024, 4032));
    }

    /// 全图只对 HEIC 转码；别的格式必须原字节穿过，不能被顺手转一遍。
    ///
    /// `full_image` 本身要 `AppHandle`，测不到，所以这里守的是它唯一的分支条件：
    /// 非 HEIF 的路径不经过 `encode_heic`，原字节直接配上 `content_type_for` 的 MIME。
    #[test]
    fn non_heic_full_images_keep_their_original_bytes() {
        // 这个文件必须带 `.png` 后缀：`is_heif` 和 `content_type_for` 都看扩展名，
        // 而 `TempFile` 生成的名字是没有后缀的。
        let mut path = std::env::temp_dir().join(format!(
            "lumenfold-thumb-full-passthrough-{}",
            std::process::id()
        ));
        path.set_extension("png");
        let file = TempFile(path);
        image::RgbImage::from_pixel(12, 8, image::Rgb([3, 4, 5]))
            .save_with_format(&file.0, image::ImageFormat::Png)
            .expect("write png");

        assert!(!is_heif(&file.0), "PNG 不该被当成 HEIF");
        assert_eq!(content_type_for(&file.0), "image/png");
        // 走的是 `std::fs::read` 那条路，读出来就是磁盘上的原字节。
        let on_disk = std::fs::read(&file.0).expect("read png");
        assert_eq!(&on_disk[..8], b"\x89PNG\r\n\x1a\n", "原字节应保持 PNG 头");
    }

    #[test]
    fn heif_extensions_are_recognized_case_insensitively() {
        for name in ["a.heic", "a.HEIC", "a.heif", "a.HEIF", "a.HeIc"] {
            assert!(is_heif(std::path::Path::new(name)), "{name} 是 HEIF");
        }
        for name in ["a.jpg", "a.jpeg", "a.png", "a.webp", "a", "a.heic.jpg"] {
            assert!(!is_heif(std::path::Path::new(name)), "{name} 不是 HEIF");
        }
    }

    /// BGRA→RGB 的字节序：WIC 给的是 B,G,R,A，写反了就是红蓝互换。
    #[test]
    fn bgra_is_reordered_into_rgb() {
        // 两个像素：(10,20,30,255) 和 (40,50,60,128) 的 BGRA 写法。
        let bgra = [30u8, 20, 10, 255, 60, 50, 40, 128];
        let rgb = bgra_to_rgb(&bgra, 2, 1).expect("2x1 的缓冲区是合法的");
        assert_eq!(rgb.width(), 2);
        assert_eq!(rgb.height(), 1);
        assert_eq!(rgb.as_raw().as_slice(), &[10u8, 20, 30, 40, 50, 60]);
    }

    /// 长度对不上时必须拒绝，不能按错误的跨距去读像素。
    #[test]
    fn a_wrongly_sized_bgra_buffer_is_rejected() {
        assert!(bgra_to_rgb(&[0u8; 7], 2, 1).is_none());
        assert!(bgra_to_rgb(&[], 0, 0).is_some());
    }

    #[test]
    fn thumb_route_reads_id_and_width_from_the_query() {
        let (id, width) = parse_thumb_route("lumen://thumb/abc123?w=320").expect("valid route");
        assert_eq!(id, "abc123");
        assert_eq!(width, 320);
    }

    #[test]
    fn thumb_route_defaults_the_width_when_the_query_is_absent() {
        let (id, width) = parse_thumb_route("lumen://thumb/abc123").expect("valid route");
        assert_eq!(id, "abc123");
        assert_eq!(width, DEFAULT_WIDTH);
    }

    #[test]
    fn thumb_route_clamps_the_requested_width() {
        assert_eq!(
            parse_thumb_route("lumen://thumb/a?w=8").expect("clamped").1,
            MIN_WIDTH
        );
        assert_eq!(
            parse_thumb_route("lumen://thumb/a?w=99999").expect("clamped").1,
            MAX_WIDTH
        );
    }

    #[test]
    fn thumb_route_also_accepts_the_width_as_a_path_segment() {
        let (id, width) = parse_thumb_route("lumen://thumb/abc123/512").expect("valid route");
        assert_eq!(id, "abc123");
        assert_eq!(width, 512);
    }

    /// 这是修复「全库缩略图都是坏图标」的那条用例。
    ///
    /// wry 的 `custom_protocol_workaround` 把 `lumen://localhost/thumb/x`
    /// 改写成 `http://lumen.localhost/thumb/x` 交给 WebView2，再原样改回来。
    /// 所以 handler 收到的一定是这个形状，而不是前端 `<img src>` 里写的那个形状。
    #[test]
    fn thumb_route_accepts_the_url_shape_the_webview_actually_sends() {
        for uri in [
            "lumen://localhost/thumb/abc123?w=320",
            "lumen://localhost/thumb/abc123/320",
        ] {
            let (id, width) = parse_thumb_route(uri)
                .unwrap_or_else(|err| panic!("{uri} must parse: {err:?}"));
            assert_eq!(id, "abc123", "{uri}");
            assert_eq!(width, 320, "{uri}");
        }
    }

    /// 老写法把资源类型放在主机位上（`thumb` 变成 authority）。
    /// 它不会走到这个 handler，但解析器接受它，代价只是一行判断。
    #[test]
    fn thumb_route_still_accepts_the_legacy_authority_spelling() {
        assert_eq!(
            parse_thumb_route("lumen://thumb/abc123?w=320").expect("legacy form"),
            ("abc123".to_string(), 320)
        );
    }

    /// `localhost` 只是主机名，不该被当成资源类型放行。
    #[test]
    fn thumb_route_still_rejects_unknown_resources_with_a_localhost_authority() {
        for uri in [
            "lumen://localhost/nope/abc123",
            "lumen://localhost/",
            "lumen://localhost",
        ] {
            assert!(parse_thumb_route(uri).is_err(), "{uri} must be rejected");
        }
    }

    #[test]
    fn thumb_route_rejects_malformed_urls() {
        for uri in [
            "lumen://thumb",
            "lumen://thumb/",
            "lumen://other/abc",
            "lumen://thumb/abc/256/extra",
        ] {
            assert!(
                parse_thumb_route(uri).is_err(),
                "{uri} must be rejected"
            );
        }
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let error = encode(std::path::Path::new("no-such-file-anywhere.png"), 256)
            .expect_err("missing file must fail");
        assert_eq!(error.kind(), crate::error::ErrorKind::Io);
    }
}
