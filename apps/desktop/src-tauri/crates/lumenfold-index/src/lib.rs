//! Filesystem scanning and header probing for Lumenfold.
//!
//! # Scope, and why it is this way
//!
//! Everything in this crate is the **no-native-dependency fallback path**:
//!
//! * [`probe`] reads image headers with the pure-Rust `image` crate and pulls
//!   `DateTimeOriginal` / `DateTimeDigitized` out of EXIF with `kamadak-exif`.
//!   It is cheap (headers only) and needs no native library. What `image` has no
//!   decoder for is handed to the OS: HEIC dimensions come from WIC
//!   (`lumenfold-wic`, Windows only) and HEIC capture dates from `kamadak-exif`,
//!   which understands the ISOBMFF `ftyp` box that HEIF files carry. RAW
//!   (CR2/NEF/DNG) still probes as `0x0` with no capture date.
//! * Thumbnailing (see the app crate) is likewise full-decode-then-resize in
//!   pure Rust, with the same WIC hand-off for HEIC.
//!
//! ADR-0001 picked **libvips** (`rust-vips`, plus `libheif` and `libraw`) as
//! the decode and scale pipeline because `shrink-on-load` beats full decode by
//! about an order of magnitude on thumbnail throughput. That pipeline replaces
//! **both** of the fallbacks above in M1: libvips/libheif/libraw give RAW/HEIC
//! dimensions and dates from headers, and the same library generates the
//! thumbnails. Nothing here is intended to survive that swap unchanged; the
//! `AssetRecord` shape and the `id` derivation are what the rest of the app
//! depends on, and those stay.
//!
//! The WIC hand-off is deliberately the *same shape* as the future libvips one:
//! one call for dimensions, one for a decode. Swapping the implementation in M1
//! does not touch any caller.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Extensions treated as photos by the walker.
///
/// RAW variants are included on purpose: [`scan`] records them with whatever
/// [`probe`] managed to read, which today is often nothing.
const PHOTO_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "heic", "cr2", "nef", "dng",
];

/// Directory names that never contain user photos and are expensive to walk.
const SKIP_DIRS: &[&str] = &["@eaDir", ".thumbnails", "node_modules", ".git"];

/// What [`probe`] managed to read out of one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssetProbe {
    /// Pixel width, `0` when the format could not be probed.
    pub width: u32,
    /// Pixel height, `0` when the format could not be probed.
    pub height: u32,
    /// Capture time in Unix seconds, `None` when the file carries no EXIF date.
    pub taken_at: Option<i64>,
    /// 类别标记位，见 [`classify`]。
    ///
    /// 真实的 [`probe`] 结果**总是**带 [`TAG_CLASSIFIED`]；只有 `Default` 出来的
    /// 空探测是 0，那正表示「什么都没读到、因此也谈不上分类」。
    pub tags: u32,
}

/// One file discovered by [`scan`], with everything the indexer read.
///
/// Derives serde so the record can be written to the persisted index snapshot
/// and read back without the app crate hand-rolling a parallel DTO that would
/// drift from this struct on the first field someone adds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssetRecord {
    /// Stable id: first 16 hex chars of BLAKE3 over the lowercased absolute path.
    pub id: String,
    /// Absolute path on disk.
    pub path: PathBuf,
    /// File name including extension.
    pub file_name: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Last modified time as Unix seconds; `0` if the platform cannot report it.
    pub mtime_secs: i64,
    /// Pixel width, `0` when unknown.
    pub width: u32,
    /// Pixel height, `0` when unknown.
    pub height: u32,
    /// Capture time in Unix seconds, `None` when unknown.
    pub taken_at: Option<i64>,
    /// 类别标记位，见 [`classify`]。
    ///
    /// `#[serde(default)]` 是为了让**旧快照**仍能读进来：那些记录的 tags 是 0，
    /// 也就是「还没分过类」。增量扫描看到这一位没设会重新读一次容器头，
    /// 所以这次升级不需要清库重建，分类会在后台一轮一轮补齐。
    #[serde(default)]
    pub tags: u32,
}

/// Whether a lowercased (or uppercased) file extension is a photo.
pub fn is_photo_extension(ext: &str) -> bool {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    PHOTO_EXTENSIONS.contains(&ext.as_str())
}

/// Derives the asset id from an absolute path.
///
/// The id is a hash of the *lowercased* path, so the same file yields the same
/// id on Windows and on a case-sensitive filesystem.
pub fn asset_id_for(path: &Path) -> String {
    let mut key = path.to_string_lossy().to_lowercase().into_bytes();
    // Hash over the raw bytes plus their length: without the length prefix a
    // path list is still unambiguous here because we only ever hash one path,
    // but the length keeps the input description honest.
    key.extend_from_slice(&(path.as_os_str().len() as u64).to_le_bytes());
    blake3::hash(&key).to_hex()[..16].to_string()
}

/// Aspect-ratio bucket for a thumbnail tile.
pub fn orientation(width: u32, height: u32) -> &'static str {
    match (width, height) {
        (0, 0) => "unknown",
        (w, h) if w == h => "square",
        (w, h) if w > h => "landscape",
        _ => "portrait",
    }
}

/* ------------------------------------------------------------------ 分类标记 */

/// 动图：容器里真的有多帧（GIF / APNG / 动画 WebP / HEIF 图像序列）。
pub const TAG_ANIMATED: u32 = 1 << 0;
/// 截图：文件名像截图，或者是「没有拍摄时间 + PNG + 尺寸正好是某个常见屏幕分辨率」。
pub const TAG_SCREENSHOT: u32 = 1 << 1;
/// 相机 RAW（本版本扫描的 RAW 只有 cr2/nef/dng，见 `PHOTO_EXTENSIONS`）。
pub const TAG_RAW: u32 = 1 << 2;
/// 全景：长边至少是短边的 2.5 倍。
pub const TAG_PANORAMA: u32 = 1 << 3;

/// 「这条记录已经分过类」的标记位。
///
/// # 为什么要有这一位
///
/// 老快照里没有 `tags` 字段，`#[serde(default)]` 读回来是 0。而 0 在任何一条
/// 分类规则下都不是合法结果（自己的位总是会被设上），所以「没有这一位」可以
/// 干净地表示「这条记录还没分类过」。增量扫描据此重新读一次它的容器头，
/// 于是这次升级不需要清库重建：分类会在后台一轮一轮补齐。
///
/// 用一个高位是为了让它和将来新增的类别位互不打扰。
pub const TAG_CLASSIFIED: u32 = 1 << 31;

/// 用户可以在界面上按这些类别筛选（不含 `TAG_CLASSIFIED`）。
pub const FILTERABLE_TAGS: &[(&str, u32)] = &[
    ("animated", TAG_ANIMATED),
    ("screenshot", TAG_SCREENSHOT),
    ("raw", TAG_RAW),
    ("panorama", TAG_PANORAMA),
];

/// 把界面传来的类别 id 翻成标记位。未知 id 返回 `None`，由调用方如实报错。
pub fn tag_bit(name: &str) -> Option<u32> {
    FILTERABLE_TAGS
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, bit)| *bit)
}

/// RAW 扩展名。只列扫描器真的会登记的那几个，避免界面说「能筛 RAW」而其实没有。
fn is_raw_extension(ext: &str) -> bool {
    matches!(ext, "cr2" | "nef" | "dng")
}

/// 长边至少是短边的 2.5 倍算全景。
///
/// 用整数比较而不是浮点：`宽 * 2 >= 高 * 5` 就是 `宽 / 高 >= 2.5`，
/// 不会因为浮点误差把一张刚好 2.5 的照片判进或判出。
fn is_panorama(width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let (long, short) = if width >= height { (width, height) } else { (height, width) };
    u64::from(long) * 2 >= u64::from(short) * 5
}

/// 常见屏幕分辨率，用来认那些文件名完全没有线索的截图。
///
/// 这一条单独用会误判（16:9 的照片也是 1920x1080），所以它必须和
/// 「PNG + 没有拍摄时间」一起用，见 [`looks_like_screenshot`]。
const SCREEN_SIZES: &[(u32, u32)] = &[
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
    (1366, 768),
    (1600, 900),
    (1680, 1050),
    (1280, 720),
    (1440, 900),
    (2880, 1800),
    (2560, 1600),
    (1920, 1200),
    (3440, 1440),
    (1024, 768),
    (1280, 1024),
    (1170, 2532),
    (1179, 2556),
    (1284, 2778),
    (1290, 2796),
    (1320, 2868),
    (1080, 1920),
    (1080, 2160),
    (1080, 2340),
    (1080, 2400),
    (1440, 2560),
    (1440, 3040),
    (720, 1280),
    (750, 1334),
    (1125, 2436),
    (828, 1792),
    (1242, 2688),
    (1536, 2048),
    (1668, 2388),
    (2048, 2732),
];

/// 文件名里出现这些片段就当成截图。中英文都列，因为这两边的命名习惯都在用。
const SCREENSHOT_MARKERS: &[&str] = &[
    "screenshot",
    "screen shot",
    "screen_shot",
    "screen-shot",
    "screencap",
    "screen capture",
    "snipaste",
    "截图",
    "截屏",
    "屏幕快照",
    "屏幕截图",
];

/// 截图判定：两条信号，任一条命中就算。
///
/// 1. **文件名**里有明确的截图标记。这一条精度高、召回一般。
/// 2. **PNG + 没有拍摄时间 + 尺寸正好等于某个常见屏幕分辨率**。这一条是为了
///    兜住手机截图——iPhone 的截图叫 `IMG_xxxx.PNG`，光看名字和相机照片没区别。
///
/// 两条都是启发式，会漏也会误。界面上必须把这条规则说清楚，不能让用户以为
/// 这是文件里写着的事实。
fn looks_like_screenshot(
    file_name: &str,
    ext: &str,
    width: u32,
    height: u32,
    taken_at: Option<i64>,
) -> bool {
    let name = file_name.to_lowercase();
    if SCREENSHOT_MARKERS.iter().any(|marker| name.contains(marker)) {
        return true;
    }
    if ext != "png" || taken_at.is_some() || width < 600 || height < 600 {
        return false;
    }
    SCREEN_SIZES
        .iter()
        .any(|(w, h)| (*w == width && *h == height) || (*w == height && *h == width))
}

/// 读文件开头的一小段。分类只需要容器头，不必把整张图读进来。
fn read_prefix(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::new();
    file.take(limit).read_to_end(&mut buffer).ok()?;
    Some(buffer)
}

/// 跳过一个以 0 字节结尾的 GIF 子块序列，返回下一个块的偏移。
fn skip_gif_sub_blocks(data: &[u8], mut offset: usize) -> usize {
    while offset < data.len() {
        let length = data[offset] as usize;
        offset += 1;
        if length == 0 {
            break;
        }
        offset += length;
    }
    offset
}

/// GIF 是不是动图：走一遍块结构，数到第二个图像描述符就收工。
///
/// 不能在字节流里直接搜 `0x2C`——那个值会出现在像素数据里。
/// 必须按块长度跳过去，所以这里做一遍最小解析。
fn gif_is_animated(data: &[u8]) -> bool {
    if data.len() < 13 || &data[0..3] != b"GIF" {
        return false;
    }
    // 逻辑屏幕描述符占 6..13，其中第 10 字节的 bit7 表示有没有全局颜色表。
    let packed = data[10];
    let mut offset = 13;
    if packed & 0x80 != 0 {
        offset += 3 * (1usize << ((packed & 0x07) + 1));
    }

    let mut images = 0usize;
    while offset < data.len() {
        match data[offset] {
            // trailer：到此为止，没有第二帧。
            0x3B => return false,
            0x2C => {
                images += 1;
                if images > 1 {
                    return true;
                }
                if offset + 10 > data.len() {
                    return false;
                }
                let local = data[offset + 9];
                offset += 10;
                if local & 0x80 != 0 {
                    offset += 3 * (1usize << ((local & 0x07) + 1));
                }
                // LZW 最小码长占 1 字节，后面是图像数据的子块。
                offset = skip_gif_sub_blocks(data, offset + 1);
            }
            // 扩展块：0x21 + 标签，后面是子块。
            0x21 => {
                if offset + 2 > data.len() {
                    return false;
                }
                offset = skip_gif_sub_blocks(data, offset + 2);
            }
            _ => return false,
        }
    }
    false
}

/// WebP 是不是动图。
///
/// 只有扩展格式（`VP8X`）能装动画；裸 `VP8 `/`VP8L` 一定是静态的。
/// `VP8X` 的 flags 在块头（8 字节）之后，animation 是 bit 1。
fn webp_is_animated(data: &[u8]) -> bool {
    data.len() >= 21 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" && &data[12..16] == b"VP8X"
        && data[20] & 0x02 != 0
}

/// PNG 是不是动图（APNG）：第一段图像数据之前有没有 `acTL` 块。
fn png_is_animated(data: &[u8]) -> bool {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 8 || &data[0..8] != SIGNATURE {
        return false;
    }
    let mut offset = 8;
    while offset + 8 <= data.len() {
        let length = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let kind = &data[offset + 4..offset + 8];
        if kind == b"acTL" {
            return true;
        }
        // 图像数据开始之后就不会再有 acTL 了。
        if kind == b"IDAT" || kind == b"IEND" {
            return false;
        }
        // 4 长度 + 4 类型 + 数据 + 4 CRC。
        offset += 12 + length;
    }
    false
}

/// HEIF 是不是图像序列（动图）：`ftyp` 的品牌里有没有 `msf1`。
///
/// 只看品牌，不调用 WIC 的帧数：品牌是纯字节判断，而 WIC 那条路要为每个文件
/// 初始化一次 COM——它在缩略图热路径上已经够贵了。
fn heif_is_animated(data: &[u8]) -> bool {
    if data.len() < 16 || &data[4..8] != b"ftyp" {
        return false;
    }
    // major brand 在 8..12，兼容品牌从 16 开始，都是 4 字节一组。
    let mut offset = 8;
    while offset + 4 <= data.len() && offset < 64 {
        if &data[offset..offset + 4] == b"msf1" {
            return true;
        }
        offset += 4;
    }
    false
}

/// 给一张照片打上类别标记。返回的位里**总是**带上 [`TAG_CLASSIFIED`]。
///
/// 这是一个纯函数加几次文件头读取；读不到头只影响「动图」这一位，
/// 其余几位仍然如实给出，不会把整张照片判成未知。
pub fn classify(
    path: &Path,
    file_name: &str,
    ext: &str,
    width: u32,
    height: u32,
    taken_at: Option<i64>,
) -> u32 {
    let mut tags = TAG_CLASSIFIED;

    if is_raw_extension(ext) {
        tags |= TAG_RAW;
    }
    if is_panorama(width, height) {
        tags |= TAG_PANORAMA;
    }
    if looks_like_screenshot(file_name, ext, width, height, taken_at) {
        tags |= TAG_SCREENSHOT;
    }

    // 只有这几种容器装得下多帧，其余格式连头都不用读。
    let animated = match ext {
        "gif" => read_prefix(path, 64 * 1024).is_some_and(|data| gif_is_animated(&data)),
        "webp" => read_prefix(path, 4096).is_some_and(|data| webp_is_animated(&data)),
        "png" => read_prefix(path, 4096).is_some_and(|data| png_is_animated(&data)),
        "heic" | "heif" => read_prefix(path, 64).is_some_and(|data| heif_is_animated(&data)),
        _ => false,
    };
    if animated {
        tags |= TAG_ANIMATED;
    }

    tags
}

/// Reads dimensions and, for JPEG/TIFF/HEIF, the capture date, from `path`.
///
/// Never fails: a file that cannot be opened, is not an image, or uses a format
/// nothing here can decode (RAW) yields `AssetProbe::default()` — `0x0`, no date
/// — because one unreadable file must not abort a scan of a folder.
///
/// # HEIC：`image` 读不出来，但这不代表读不出来
///
/// `image` 0.25 没有 HEIF 解码器，所以 `into_dimensions()` 对 `.HEIC` 永远返回
/// `Err`——iPhone 照片在图库里就是一片 0x0 的格子。这里退回系统自带的 WIC 编解码层
/// （见 `lumenfold-wic`）：尺寸能如实读出来，且不需要引入任何要编译的原生依赖。
///
/// 顺序是刻意这样排的：先试纯 Rust 路径，它对 JPEG/PNG 只需要读头部、代价最低；
/// WIC 那条路要初始化 COM 并实例化解码器，是更贵的一步，只在必要时付。
///
/// 没有装 HEIF 扩展的机器上 `lumenfold_wic::dimensions` 返回 `None`，
/// 于是一张读不了的 HEIC 仍然落回 `0x0`——和以前一样，只是现在这个 `0x0`
/// 真的只表示「读不到」，不再表示「这个格式不支持」。
pub fn probe(path: &Path) -> AssetProbe {
    let dimensions = image::ImageReader::open(path)
        .ok()
        .and_then(|reader| {
            // `with_guessed_format` sniffs the magic bytes, so the extension
            // does not have to be right.
            reader.with_guessed_format().ok()
        })
        .and_then(|reader| reader.into_dimensions().ok())
        .map(|(width, height)| (width, height))
        .filter(|(width, height)| *width > 0 && *height > 0)
        // 纯 Rust 路径失败时才问系统。顺序反过来会让每一张 JPEG 都白付一次 COM 初始化。
        .or_else(|| lumenfold_wic::dimensions(path))
        .filter(|(width, height)| *width > 0 && *height > 0);

    let taken_at = exif_taken_at(path);

    let (width, height) = dimensions.unwrap_or((0, 0));
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let tags = classify(path, file_name, &ext, width, height, taken_at);

    AssetProbe {
        width,
        height,
        taken_at,
        tags,
    }
}

/// EXIF `DateTimeOriginal` (0x9003), falling back to `DateTimeDigitized` (0x9004).
///
/// # 为什么 HEIC 也在这里
///
/// 这个白名单以前是「只读 JPEG 和 TIFF 容器」。它现在不成立了：
/// `kamadak-exif` 0.6.1 有 `isobmff.rs`，里面有 `HEIF_BRANDS = [b"mif1", b"msf1"]`
/// 和 `is_heif()`，`read_from_container` 因此认得 HEIF 的 `ftyp` 盒子并去里面找
/// EXIF。抽样过的用户文件每一个的 major brand 都是 `heic`，兼容品牌里有 `mif1`，
/// 所以 `is_heif()` 全部命中。
///
/// 换句话说，HEIC 的拍摄日期从来都在文件里，缺的只是这行白名单。
/// 少了它，iPhone 照片即使尺寸读对了，也会因为没有日期而掉进「未知时间」分组。
fn exif_taken_at(path: &Path) -> Option<i64> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "tif" | "tiff" | "heic" | "heif"
    ) {
        return None;
    }

    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    // `read_from_container` locates the EXIF block for us, whatever the
    // container is: it handles the JPEG APP1 segment, a bare TIFF header, and
    // the ISOBMFF `meta`/`iinf` walk HEIF needs.
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;

    for field in exif.fields() {
        let tag = field.tag.number();
        if tag != 0x9003 && tag != 0x9004 {
            continue;
        }
        if let exif::Value::Ascii(raw) = &field.value {
            // A malformed tag can hold several NUL-separated strings; the
            // first parseable one wins.
            if let Some(seconds) = raw.iter().find_map(|chunk| parse_exif_datetime(chunk)) {
                return Some(seconds);
            }
        }
    }
    None
}

/// Parses `"YYYY:MM:DD HH:MM:SS"` (as written by EXIF) into Unix seconds, UTC.
///
/// Returns `None` for anything that does not parse; the field is advisory.
fn parse_exif_datetime(raw: &[u8]) -> Option<i64> {
    // EXIF writes `"YYYY:MM:DD HH:MM:SS"`. The date and the time are separated
    // by whitespace, not by a colon, so splitting on `:` alone would glue
    // `"04 05"` together as the day. Split in two steps.
    let text = std::str::from_utf8(raw).ok()?.trim_end_matches('\0').trim();
    let mut fields = text.split_whitespace();
    let mut date = fields.next()?.split(':');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // A date with no time component is written midnight UTC.
    let (hour, minute, second) = match fields.next() {
        None => (0, 0, 0),
        Some(hms) => {
            let mut hms = hms.split(':');
            let hour: i64 = hms.next()?.parse().ok()?;
            let minute: i64 = hms.next().unwrap_or("0").parse().ok()?;
            let second: i64 = hms.next().unwrap_or("0").parse().ok()?;
            (hour, minute, second)
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Walks `root` and returns one [`AssetRecord`] per photo file, sorted by path.
///
/// `progress` is called with the running record count after each file is
/// probed. Symlinks are not followed, and dotfiles plus the directories in
/// `SKIP_DIRS` are skipped.
pub fn scan(root: &Path, progress: &mut impl FnMut(usize)) -> io::Result<Vec<AssetRecord>> {
    let mut records = Vec::new();
    scan_each(root, &mut |record| records.push(record), progress)?;
    records.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(records)
}

/// Like [`scan`], but hands each record to `on_record` as it is discovered.
///
/// This is what the app uses: it publishes records into shared state while the
/// walk is still running, so the UI can fill the grid before the scan ends.
/// Records arrive in walk order, not sorted; sort the collected set yourself if
/// order matters.
pub fn scan_each(
    root: &Path,
    on_record: &mut dyn FnMut(AssetRecord),
    progress: &mut dyn FnMut(usize),
) -> io::Result<()> {
    scan_each_while(root, on_record, progress, &|| false)
}

/// [`scan_each`] with a cooperative stop flag.
///
/// 取消走的是协作式：walker 每发现一个文件就问一次 should_stop，
/// 而不是让 UI 线程去 kill 扫描线程。一次 stop 的代价是再多读一个文件的头，
/// 对几万张的图库来说是毫秒级；换来的是 on_record 绝不会在取消之后再往状态里塞记录，
/// 那正是「点了取消，照片数量还在涨」的那种竞态。
pub fn scan_each_while(
    root: &Path,
    on_record: &mut dyn FnMut(AssetRecord),
    progress: &mut dyn FnMut(usize),
    should_stop: &dyn Fn() -> bool,
) -> io::Result<()> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", root.display()),
        ));
    }

    let mut count = 0usize;
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_skipped(entry.file_name()));

    for entry in walker {
        // An unreadable directory or dangling symlink is skipped, not fatal.
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !is_photo_extension(ext) {
            continue;
        }

        if should_stop() {
            return Ok(());
        }

        let Ok(metadata) = entry.metadata() else { continue };
        let probe = probe(path);
        let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let file_name = entry.file_name().to_string_lossy().into_owned();

        on_record(AssetRecord {
            id: asset_id_for(&absolute),
            path: absolute,
            file_name,
            bytes: metadata.len(),
            mtime_secs: modified_secs(&metadata),
            width: probe.width,
            height: probe.height,
            taken_at: probe.taken_at,
            tags: probe.tags,
        });
        count += 1;
        progress(count);
    }
    Ok(())
}

/// What one incremental walk actually had to do.
///
/// 界面要拿它说人话：「复用 12,340 张 / 重新读取 12 张」和「复用 0 张 /
/// 重新读取 12,340 张」是两件完全不同的事，前者是秒开，后者是重扫全库。
/// 不把这个区别统计出来，增量扫描就只是一句没人能验证的口号。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// Photo files the walk saw.
    pub visited: usize,
    /// Files whose size and mtime were unchanged, so the cached record was reused verbatim.
    pub reused: usize,
    /// Files that had to be opened and probed.
    pub probed: usize,
    /// The walk stopped early because `should_stop` flipped.
    pub cancelled: bool,
}

/// [`scan_each_while`] against a previously saved index, reusing what it can.
///
/// This is the difference between reopening a library and re-indexing it. The
/// expensive part of a scan is not the directory walk — that is a `stat` per
/// file — it is [`probe`], which opens the file, decodes the header and parses
/// EXIF, once per photo. So the walker still visits every file, but when
/// [`asset_id_for`] resolves to a cached record whose size and mtime are both
/// unchanged, it hands that record straight back and never opens the photo.
///
/// `on_record` receives `(record, from_cache)`. The `false` case is the only one
/// that needs new work in the caller: a reused record is by definition already
/// in the persisted index, so upserting it again would be a no-op at best.
/// Both cases are still delivered so the caller can track *which ids exist on
/// disk* — that set is what lets externally deleted photos fall out of the index
/// without a full rebuild.
///
/// A cache miss is not an error and a cache hit is not a guarantee: a file can
/// change without changing its size or mtime if something rewrites it in place
/// with a restored timestamp. That case degrades to showing the previous probe
/// of that file, which is the same answer a rescan would have given a moment
/// earlier — not a wrong answer, just a stale one.
pub fn scan_each_while_cached(
    root: &Path,
    cache: &HashMap<String, AssetRecord>,
    on_record: &mut dyn FnMut(AssetRecord, bool),
    progress: &mut dyn FnMut(usize),
    should_stop: &dyn Fn() -> bool,
) -> io::Result<ScanOutcome> {
    let mut outcome = ScanOutcome::default();
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", root.display()),
        ));
    }

    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_skipped(entry.file_name()));

    for entry in walker {
        // An unreadable directory or dangling symlink is skipped, not fatal.
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !is_photo_extension(ext) {
            continue;
        }

        if should_stop() {
            outcome.cancelled = true;
            return Ok(outcome);
        }

        let Ok(metadata) = entry.metadata() else { continue };
        let bytes = metadata.len();
        let mtime_secs = modified_secs(&metadata);
        let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let id = asset_id_for(&absolute);

        // size、mtime 都得对上，而且这条记录必须**已经分过类**。
        // 少一个都不能复用：改 EXIF 不会改字节数（很多导出工具会把时间戳原样
        // 搬过去），只看 size 会把改过的照片永远钉在旧记录上；而老快照里的
        // 记录 tags 是 0，直接复用等于把「未知类别」当成结论。
        if let Some(cached) = cache.get(&id) {
            if cached.bytes == bytes
                && cached.mtime_secs == mtime_secs
                && cached.tags & TAG_CLASSIFIED != 0
            {
                on_record(cached.clone(), true);
                outcome.reused += 1;
                outcome.visited += 1;
                progress(outcome.visited);
                continue;
            }
        }

        let probe = probe(path);
        on_record(
            AssetRecord {
                id,
                path: absolute,
                file_name: entry.file_name().to_string_lossy().into_owned(),
                bytes,
                mtime_secs,
                width: probe.width,
                height: probe.height,
                taken_at: probe.taken_at,
                tags: probe.tags,
            },
            false,
        );
        outcome.probed += 1;
        outcome.visited += 1;
        progress(outcome.visited);
    }
    Ok(outcome)
}

/// Skips dotfiles and the known non-photo directory names.
///
/// 公开出去是给「自动发现图库目录」用的：发现逻辑必须和扫描逻辑用**同一份**
/// 跳过名单，否则它会把 `.git` 之类的目录推荐成图库，而扫描进去又什么都找不到。
pub fn is_skipped(name: &std::ffi::OsStr) -> bool {
    let Some(text) = name.to_str() else {
        // Non-UTF-8 names cannot match a skip list; keep them.
        return false;
    };
    text.starts_with('.') || SKIP_DIRS.contains(&text)
}

fn modified_secs(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Creates a unique temp directory and deletes it on drop, so a failing
    /// assertion never leaves anything behind in the repo or in `%TEMP%`.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "lumenfold-index-{}-{}-{}",
                std::process::id(),
                tag,
                n
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn photo_extensions_are_recognized() {
        for ext in [
            "jpg", "jpeg", "png", "webp", "tif", "tiff", "heic", "cr2", "nef", "dng",
        ] {
            assert!(is_photo_extension(ext), "{ext} should be a photo extension");
            assert!(
                is_photo_extension(&ext.to_ascii_uppercase()),
                "{ext} should match case-insensitively"
            );
        }
        for ext in ["txt", "mp4", "zip"] {
            assert!(!is_photo_extension(ext), "{ext} is not a photo");
        }
    }

    #[test]
    fn orientation_covers_all_cases() {
        assert_eq!(orientation(0, 0), "unknown");
        assert_eq!(orientation(800, 800), "square");
        assert_eq!(orientation(1920, 1080), "landscape");
        assert_eq!(orientation(1080, 1920), "portrait");
    }

    #[test]
    fn scan_finds_only_photos_and_ids_are_stable() {
        let dir = TempDir::new("scan");

        // A real PNG: 7x5, so a non-zero probe is provable.
        let png = image::RgbImage::from_pixel(7, 5, image::Rgb([10, 20, 30]));
        png.save_with_format(dir.0.join("photo.png"), image::ImageFormat::Png)
            .expect("write png");
        std::fs::write(dir.0.join("notes.txt"), b"not a photo").expect("write txt");

        let mut seen: Vec<usize> = Vec::new();
        let first = scan(&dir.0, &mut |n| seen.push(n)).expect("scan");
        assert_eq!(seen, vec![1], "progress should fire once per photo");
        assert_eq!(first.len(), 1, "only the PNG should be indexed");
        assert_eq!(first[0].file_name, "photo.png");
        assert_eq!(first[0].width, 7);
        assert_eq!(first[0].height, 5);
        assert!(first[0].bytes > 0);
        assert_eq!(first[0].id.len(), 16);

        let second = scan(&dir.0, &mut |_| {}).expect("rescan");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, first[0].id, "ids must be stable across scans");
        assert_eq!(second[0].path, first[0].path);
    }

    #[test]
    fn scan_each_while_stops_as_soon_as_should_stop_flips() {
        let dir = TempDir::new("cancel");
        for i in 0..5 {
            image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
                .save_with_format(dir.0.join(format!("p{i}.png")), image::ImageFormat::Png)
                .expect("write png");
        }

        let mut seen = Vec::new();
        // 停在一个已经会立刻返回 true 的闭包上，等价于「还没开始就取消」：
        // 一条记录都不能发出去，这正是取消后计数不该继续增长的那条不变量。
        scan_each_while(
            &dir.0,
            &mut |record| seen.push(record.file_name),
            &mut |_| {},
            &|| true,
        )
        .expect("canceled walk is not an error");
        assert!(seen.is_empty(), "canceled walk must publish nothing");

        let mut all = Vec::new();
        scan_each_while(&dir.0, &mut |r| all.push(r.file_name), &mut |_| {}, &|| false)
            .expect("uncanceled walk");
        assert_eq!(all.len(), 5);

        // 走到第 3 条时取消：剩下两条不能出现。
        // `Cell` 而不是裸变量：两个闭包同时借用这个计数，裸 `usize` 借不动。
        let counter = std::cell::Cell::new(0usize);
        let mut partial = Vec::new();
        scan_each_while(
            &dir.0,
            &mut |r| {
                counter.set(counter.get() + 1);
                partial.push(r.file_name);
            },
            &mut |_| {},
            &|| counter.get() >= 3,
        )
        .expect("partial walk");
        assert_eq!(partial.len(), 3, "stop must take effect within one file");
    }

    #[test]
    fn scan_skips_dotfiles_and_ignored_directories() {
        let dir = TempDir::new("skips");
        let nested = dir.0.join("@eaDir").join("thumb");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
            .save_with_format(nested.join("ignored.png"), image::ImageFormat::Png)
            .expect("write ignored png");

        let root = image::RgbImage::from_pixel(3, 3, image::Rgb([4, 5, 6]));
        root.save_with_format(dir.0.join("kept.png"), image::ImageFormat::Png)
            .expect("write kept png");
        std::fs::write(dir.0.join(".hidden.png"), b"dotfile").expect("write dotfile");

        let records = scan(&dir.0, &mut |_| {}).expect("scan");
        assert_eq!(records.len(), 1, "only kept.png should be indexed");
        assert_eq!(records[0].file_name, "kept.png");
    }

    #[test]
    fn probe_of_unreadable_file_is_zeroed_not_fatal() {
        let dir = TempDir::new("probe");
        let path = dir.0.join("broken.png");
        std::fs::write(&path, b"this is not a png").expect("write junk");
        // 「读不到」和「没分类」是两件事，这条测试要同时守住两个都成立。
        let probed = probe(&path);
        assert_eq!((probed.width, probed.height, probed.taken_at), (0, 0, None));
        // 这一段垃圾既不可能是动图也不是全景，但它是**分过类的**——
        // 没有 `TAG_CLASSIFIED` 的话增量扫描会每一轮都重读它。
        assert_eq!(probed.tags, TAG_CLASSIFIED);
        // The junk file still carries a photo extension, so it lands in the
        // index with unknown dimensions rather than being dropped.
        let records = scan(&dir.0, &mut |_| {}).expect("scan");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].width, 0);
        assert_eq!(records[0].height, 0);
        assert_eq!(records[0].taken_at, None);
    }

    /* ------------------------------------------------------------ 分类判定 */

    /// 组装一个最小的 GIF：`images` 个图像描述符，每个带一段 2 字节子块。
    fn gif_bytes(images: usize) -> Vec<u8> {
        let mut gif = Vec::new();
        gif.extend_from_slice(b"GIF89a");
        gif.extend_from_slice(&[1, 0, 1, 0]); // 逻辑屏幕 1x1
        gif.push(0x00); // 打包字段：没有全局颜色表
        gif.push(0x00); // 背景色索引
        gif.push(0x00); // 像素宽高比
        for _ in 0..images {
            gif.push(0x2C); // 图像描述符
            gif.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]); // left/top/width/height
            gif.push(0x00); // 打包字段：没有局部颜色表
            gif.push(0x02); // LZW 最小码长
            gif.push(0x02); // 子块长度
            gif.extend_from_slice(&[0x4C, 0x01]);
            gif.push(0x00); // 子块结束
        }
        gif.push(0x3B); // trailer
        gif
    }

    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC 不参与判定
        out
    }

    /// 动图要**走块结构**数出来。
    ///
    /// 在字节流里直接搜 `0x2C` 是错的：那个值会出现在像素数据里，
    /// 一张静态图只要碰巧含有它就判成动图。这条测试用一个单帧 GIF 后面
    /// 接三个 `0x2C` 来钉住这件事。
    #[test]
    fn gif_animation_is_counted_by_walking_blocks() {
        assert!(!gif_is_animated(&gif_bytes(1)), "单帧 GIF 不是动图");
        assert!(gif_is_animated(&gif_bytes(2)), "两帧 GIF 是动图");

        let mut decoy = gif_bytes(1);
        decoy.extend_from_slice(&[0x2C, 0x2C, 0x2C]);
        assert!(!gif_is_animated(&decoy), "trailer 之后的字节不该被当成第二帧");
    }

    #[test]
    fn webp_animation_needs_the_vp8x_flag() {
        let mut animated = Vec::new();
        animated.extend_from_slice(b"RIFF");
        animated.extend_from_slice(&[0, 0, 0, 0]);
        animated.extend_from_slice(b"WEBP");
        animated.extend_from_slice(b"VP8X");
        animated.extend_from_slice(&[0, 0, 0, 0]);
        animated.push(0x02); // flags：animation
        assert!(webp_is_animated(&animated));

        // 同样的容器，flag 没置位就不是动图。
        let mut still = animated.clone();
        still[20] = 0x00;
        assert!(!webp_is_animated(&still));

        // 裸 VP8L 装不下动画，连 flags 都不用看。
        let mut lossless = Vec::new();
        lossless.extend_from_slice(b"RIFF");
        lossless.extend_from_slice(&[0, 0, 0, 0]);
        lossless.extend_from_slice(b"WEBP");
        lossless.extend_from_slice(b"VP8L");
        assert!(!webp_is_animated(&lossless));
    }

    #[test]
    fn png_animation_needs_an_actl_chunk_before_the_pixels() {
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(&png_chunk(b"IHDR", &[0u8; 13]));
        apng.extend_from_slice(&png_chunk(b"acTL", &[0u8; 8]));
        apng.extend_from_slice(&png_chunk(b"IDAT", &[0u8; 4]));
        assert!(png_is_animated(&apng), "acTL 在像素之前 = APNG");

        let mut still = b"\x89PNG\r\n\x1a\n".to_vec();
        still.extend_from_slice(&png_chunk(b"IHDR", &[0u8; 13]));
        still.extend_from_slice(&png_chunk(b"IDAT", &[0u8; 4]));
        assert!(!png_is_animated(&still));

        // IDAT 之后才出现的 acTL 是坏文件，不该被算成多帧。
        let mut late = b"\x89PNG\r\n\x1a\n".to_vec();
        late.extend_from_slice(&png_chunk(b"IHDR", &[0u8; 13]));
        late.extend_from_slice(&png_chunk(b"IDAT", &[0u8; 4]));
        late.extend_from_slice(&png_chunk(b"acTL", &[0u8; 8]));
        assert!(!png_is_animated(&late));
    }

    #[test]
    fn heif_animation_needs_the_msf1_brand() {
        let mut sequence = Vec::new();
        sequence.extend_from_slice(&[0, 0, 0, 0x18]);
        sequence.extend_from_slice(b"ftyp");
        sequence.extend_from_slice(b"msf1"); // major brand：图像序列
        sequence.extend_from_slice(&[0, 0, 0, 0]);
        sequence.extend_from_slice(b"mif1");
        assert!(heif_is_animated(&sequence));

        // 用户图库里抽样的 300 个 HEIC 全是 heic + mif1——静态照片。
        let mut still = Vec::new();
        still.extend_from_slice(&[0, 0, 0, 0x18]);
        still.extend_from_slice(b"ftyp");
        still.extend_from_slice(b"heic");
        still.extend_from_slice(&[0, 0, 0, 0]);
        still.extend_from_slice(b"mif1");
        assert!(!heif_is_animated(&still));
    }

    /// 截图判定是启发式，这条测试把两条信号和它们的边界都写下来。
    #[test]
    fn screenshot_heuristic_needs_a_name_marker_or_a_screen_sized_png() {
        // 文件名命中：尺寸和时间都不参与。
        assert!(looks_like_screenshot("Screenshot 2024-05-01.png", "png", 800, 600, Some(1)));
        assert!(looks_like_screenshot("屏幕截图 2024-05-01.png", "png", 800, 600, Some(1)));

        // 名字没线索，但「PNG + 没有拍摄时间 + 尺寸正好是某块屏幕」命中——
        // iPhone 的截图就叫 IMG_xxxx.PNG，光看名字和相机照片没区别。
        assert!(looks_like_screenshot("IMG_1234.PNG", "png", 1170, 2532, None));

        // 同样尺寸但有拍摄时间：那是照片，不是截图。
        assert!(!looks_like_screenshot("IMG_1234.PNG", "png", 1170, 2532, Some(1)));
        // 是 PNG、没有时间，但尺寸不匹配任何一块屏幕。
        assert!(!looks_like_screenshot("IMG_1234.PNG", "png", 1234, 5678, None));
        // 尺寸像屏幕，但不是 PNG。
        assert!(!looks_like_screenshot("DSC_0001.jpg", "jpg", 1920, 1080, None));
    }

    #[test]
    fn panorama_threshold_is_two_and_a_half_to_one() {
        assert!(is_panorama(1000, 400), "正好 2.5:1 算全景");
        assert!(!is_panorama(1000, 401), "差一点点就不算");
        assert!(is_panorama(400, 1000), "竖幅全景同样算");
        assert!(!is_panorama(0, 0));
        assert!(!is_panorama(1000, 0), "读不到尺寸时不能靠 0 除出个全景");
    }

    /// `classify` 必须**总是**带上 `TAG_CLASSIFIED`。
    ///
    /// 增量扫描就是靠这一位区分「这张图没有这些类别」和「这张图还没被分类过」；
    /// 少了它，老索引里的每一条都会在每一轮扫描里被重新打开一次。
    #[test]
    fn classify_always_marks_the_record_so_incremental_scan_can_tell() {
        let dir = TempDir::new("classify");
        // 路径不存在：容器头读不到，但「分过类了」这件事仍然成立。
        let tags = classify(&dir.0.join("gone.jpg"), "gone.jpg", "jpg", 100, 50, Some(1));
        assert_eq!(tags, TAG_CLASSIFIED, "没命中任何类别时只剩下分类标记");

        // RAW 与全景都只看扩展名和尺寸，不需要真的读到文件。
        let tags = classify(&dir.0.join("gone.dng"), "gone.dng", "dng", 8000, 2000, None);
        assert_eq!(tags & TAG_RAW, TAG_RAW);
        assert_eq!(tags & TAG_PANORAMA, TAG_PANORAMA);
        assert_eq!(tags & TAG_ANIMATED, 0);
        assert_eq!(tags & TAG_CLASSIFIED, TAG_CLASSIFIED);
    }

    #[test]
    fn tag_bit_only_knows_the_filterable_names() {
        assert_eq!(tag_bit("animated"), Some(TAG_ANIMATED));
        assert_eq!(tag_bit("panorama"), Some(TAG_PANORAMA));
        // 分类标记位不是可以拿来筛的类别，两个名字都不该被认出来。
        assert_eq!(tag_bit("classified"), None);
        assert_eq!(tag_bit(""), None);
        assert_eq!(tag_bit("nonsense"), None);
    }

    /// 真实 iPhone HEIC 的探测取证。
    ///
    /// 绝对路径不可能做成可移植的 fixture，所以这条测试 `#[ignore]`，
    /// 由 `cargo test -- --ignored --nocapture` 显式跑，样本不在时安静跳过。
    /// 它守的是这次修复的两半：尺寸来自 WIC，日期来自 `kamadak-exif` 的 ISOBMFF 解析。
    #[test]
    #[ignore = "需要本机 iCloud 图库里的真实 HEIC 样本"]
    fn probes_a_real_heic_when_one_is_available() {
        const SAMPLE: &str = r"D:\Users\lemostic\Pictures\iCloud Photos\Photos\IMG_0001.HEIC";
        let path = Path::new(SAMPLE);
        if !path.is_file() {
            println!("SKIP: 没有找到真实 HEIC 样本");
            return;
        }

        let probed = probe(path);
        println!(
            "probe({SAMPLE}) -> {}x{}, taken_at={:?}",
            probed.width, probed.height, probed.taken_at
        );

        // 修复前这两项都是 0：`image` 没有 HEIF 解码器，尺寸永远读不出来。
        assert!(probed.width > 0, "HEIC 的宽度必须是真值，不能是 0");
        assert!(probed.height > 0, "HEIC 的高度必须是真值，不能是 0");
        // 修复前这里也是 None：白名单只放行 JPEG/TIFF。
        assert!(
            probed.taken_at.is_some(),
            "iPhone HEIC 里带着 EXIF 拍摄时间，不该读不出来"
        );
        // 2010 年之后、2100 年之前：只排除明显解析错的答案。
        let seconds = probed.taken_at.expect("checked above");
        assert!(
            (1_262_304_000..4_102_444_800).contains(&seconds),
            "拍摄时间 {seconds} 不在合理区间"
        );

        let tag = orientation(probed.width, probed.height);
        println!("orientation -> {tag}");
    }

    #[test]
    fn an_unchanged_photo_is_reused_without_reopening_it() {        let dir = TempDir::new("cached");
        image::RgbImage::from_pixel(7, 5, image::Rgb([10, 20, 30]))
            .save_with_format(dir.0.join("photo.png"), image::ImageFormat::Png)
            .expect("write png");

        let first = scan(&dir.0, &mut |_| {}).expect("first scan");
        let cache: HashMap<String, AssetRecord> =
            first.iter().map(|r| (r.id.clone(), r.clone())).collect();

        let mut delivered = Vec::new();
        let outcome = scan_each_while_cached(
            &dir.0,
            &cache,
            &mut |record, from_cache| delivered.push((record, from_cache)),
            &mut |_| {},
            &|| false,
        )
        .expect("cached walk");

        assert_eq!(outcome.visited, 1);
        assert_eq!(outcome.reused, 1, "nothing changed, so nothing should be reopened");
        assert_eq!(outcome.probed, 0, "reprobing is the whole cost this avoids");
        assert!(!outcome.cancelled);

        assert_eq!(delivered.len(), 1, "both cases are delivered so ids stay countable");
        let (record, from_cache) = delivered.remove(0);
        assert!(from_cache, "the caller must be able to tell reused from fresh");
        assert_eq!(record, first[0], "a reused record must be handed back verbatim");
    }

    /// 改过 EXIF 的照片字节数常常一模一样（导出工具会把时间戳原样搬过去）。
    ///
    /// 只看 mtime 会漏掉原地改写，只看 size 会漏掉「恢复过时间戳的改写」；
    /// 两个都对不上才允许复用——少复用一次只是慢一次，多复用一次就一直显示
    /// 过期的尺寸和拍摄时间，而后者是用户看得见的错。
    #[test]
    fn bytes_are_checked_too_because_a_rewrite_can_restore_the_timestamp() {
        let dir = TempDir::new("rewrite");
        let path = dir.0.join("photo.png");
        image::RgbImage::from_pixel(7, 5, image::Rgb([10, 20, 30]))
            .save_with_format(&path, image::ImageFormat::Png)
            .expect("write png");

        let first = scan(&dir.0, &mut |_| {}).expect("first scan");
        let cache: HashMap<String, AssetRecord> =
            first.iter().map(|r| (r.id.clone(), r.clone())).collect();
        let original_mtime = std::fs::metadata(&path).expect("stat").modified().expect("mtime");

        // 换成一张字节数不同的图，然后把修改时间**按回原来的值**。
        image::RgbImage::from_pixel(9, 5, image::Rgb([10, 20, 30]))
            .save_with_format(&path, image::ImageFormat::Png)
            .expect("rewrite png");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open for touch")
            .set_modified(original_mtime)
            .expect("restore mtime");

        assert_ne!(
            std::fs::metadata(&path).expect("stat").len(),
            first[0].bytes,
            "the rewrite must change the byte count for this test to mean anything",
        );
        assert_eq!(
            std::fs::metadata(&path).expect("stat").modified().expect("mtime"),
            original_mtime,
            "the timestamp was deliberately restored",
        );

        let mut latest = Vec::new();
        let outcome = scan_each_while_cached(
            &dir.0,
            &cache,
            &mut |record, from_cache| latest.push((record, from_cache)),
            &mut |_| {},
            &|| false,
        )
        .expect("cached walk");

        assert_eq!(outcome.reused, 0, "mtime alone must not be trusted");
        assert_eq!(outcome.probed, 1);
        assert_eq!(latest[0].1, false, "a fresh probe must be reported as fresh");
        assert_eq!(latest[0].0.width, 9, "the new dimensions have to be picked up");
    }

    /// 被删掉的文件不会出现在结果里——这是调用方能把外部删除踢出索引的依据。
    #[test]
    fn a_cached_walk_reports_only_the_files_that_still_exist() {
        let dir = TempDir::new("prune");
        for name in ["a.png", "b.png"] {
            image::RgbImage::from_pixel(3, 3, image::Rgb([4, 5, 6]))
                .save_with_format(dir.0.join(name), image::ImageFormat::Png)
                .expect("write png");
        }
        let first = scan(&dir.0, &mut |_| {}).expect("first scan");
        let cache: HashMap<String, AssetRecord> =
            first.iter().map(|r| (r.id.clone(), r.clone())).collect();

        std::fs::remove_file(dir.0.join("b.png")).expect("delete b");

        let mut names = Vec::new();
        let outcome = scan_each_while_cached(
            &dir.0,
            &cache,
            &mut |record, _| names.push(record.file_name),
            &mut |_| {},
            &|| false,
        )
        .expect("cached walk");

        assert_eq!(outcome.reused, 1, "the surviving file was unchanged");
        assert_eq!(names, vec!["a.png"], "the deleted file must not be reported");
    }

    #[test]
    fn a_cached_walk_still_stops_on_cancellation() {
        let dir = TempDir::new("cached-cancel");
        for i in 0..4 {
            image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
                .save_with_format(dir.0.join(format!("p{i}.png")), image::ImageFormat::Png)
                .expect("write png");
        }
        let cache: HashMap<String, AssetRecord> = scan(&dir.0, &mut |_| {})
            .expect("first scan")
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();

        let outcome = scan_each_while_cached(
            &dir.0,
            &cache,
            &mut |_, _| {},
            &mut |_| {},
            &|| true,
        )
        .expect("canceled walk is not an error");

        assert!(outcome.cancelled, "取消不是错误，但必须被说出来");
        assert_eq!(outcome.visited, 0);
    }

    #[test]
    fn parses_exif_datetimes() {
        // 2021-03-04T05:06:07Z == 1614834367
        assert_eq!(
            parse_exif_datetime(b"2021:03:04 05:06:07\0"),
            Some(1_614_834_367)
        );
        // Date only: midnight UTC, 2021-03-04.
        assert_eq!(parse_exif_datetime(b"2021:03:04"), Some(1_614_816_000));
        // Truncated to HH:MM: seconds default to zero.
        assert_eq!(parse_exif_datetime(b"2021:03:04 05:06"), Some(1_614_834_360));
        assert_eq!(parse_exif_datetime(b"not a date"), None);
        assert_eq!(parse_exif_datetime(b"2021:13:04 00:00:00"), None);
        assert_eq!(parse_exif_datetime(b""), None);
    }
}
