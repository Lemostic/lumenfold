//! HEIF/HEIC decoding through the Windows Imaging Component.
//!
//! # 为什么是 WIC，而不是 libheif
//!
//! ADR-0001 规划的图片流水线是 libvips + libheif。今天它落不了地：这台机器上没有
//! `cmake`/`nasm`/`clang`/`ninja`，libheif 只能从源码构建，仓库里也没有预编译的
//! libvips。也就是说「按 ADR 办」的实际代价是先给每个开发者和每个 CI 装一套 C 工具链，
//! 再解决 LGPL 的动态链接分发问题——为了看一眼 iPhone 照片，这个代价不成比例。
//!
//! WIC 是 Windows 自带的编解码 API。HEIC 的能力不是我们链接进来的，而是用户机器上
//! 已经装好的 `Microsoft.HEIFImageExtension` 提供的（本机版本 1.2.48.0，
//! 配套 `Microsoft.HEVCVideoExtension` 2.5.33.0）。于是：
//!
//! * **零新增构建依赖**：不编译任何 C/C++，不需要 cmake/nasm/ninja。
//! * **零再分发**：不含 libheif、不含 libde265，AGPL-3.0-or-later 的授权问题不存在。
//! * **零安装步骤**：Windows 10 1809 以后系统自带 HEIF 扩展；没有扩展的机器上
//!   [`heif_available`] 会如实说「没有」，而不是让上层把「缺编解码器」显示成「文件损坏」。
//!
//! ADR-0001 的 libvips 流水线上线后，本模块就是被替换掉的那一块：它的公开接口只有
//! 尺寸、帧数和解码三个动作，替换它不需要动任何调用方。
//!
//! # 失败一律是 `None`
//!
//! 与 `lumenfold-index` 的 `probe()` 同一套哲学：一个读不了的文件不能让整个文件夹的
//! 扫描中断。不是 HEIF 的文件、没有装 HEIF 扩展的机器、拷了一半的损坏文件，在这里
//! 全都是普通的 `None`，不会 panic，也不会 `.unwrap()` 一个 COM 结果。

/// 一张解码完成的位图。
///
/// 像素布局固定为 BGRA8、紧密排列（行跨距 == `width * 4`），因为 WIC 的
/// `GUID_WICPixelFormat32bppBGRA` 就是这个布局，中间再做一次转换只是多一次拷贝。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    /// 像素宽度。
    pub width: u32,
    /// 像素高度。
    pub height: u32,
    /// BGRA8 像素，长度恰好是 `width * height * 4`。
    pub bgra: Vec<u8>,
}

/// 目标平台上有 HEIF 解码器时才编译真实现。
#[cfg(target_os = "windows")]
mod imp;

#[cfg(target_os = "windows")]
pub use imp::{decode, decode_to_width, dimensions, frame_count, heif_available};

/// 非 Windows 目标上的空实现。
///
/// 签名不随平台变化——调用方（索引探测、缩略图）因此不需要任何 `cfg` 分支，
/// 在 Linux/macOS 上编译出来的东西只是「这里永远没有 HEIC 支持」。
#[cfg(not(target_os = "windows"))]
mod stub {
    use super::DecodedImage;
    use std::path::Path;

    /// 非 Windows 平台上没有 WIC，也就没有 HEIF 解码器。
    pub fn heif_available() -> bool {
        false
    }

    /// 非 Windows 平台上探测不到 HEIC 尺寸。
    pub fn dimensions(_path: &Path) -> Option<(u32, u32)> {
        None
    }

    /// 非 Windows 平台上数不出 HEIC 帧数。
    pub fn frame_count(_path: &Path) -> Option<u32> {
        None
    }

    /// 非 Windows 平台上解码不了 HEIC。
    pub fn decode(_path: &Path, _max_width: Option<u32>, _frame: u32) -> Option<DecodedImage> {
        None
    }

    /// 非 Windows 平台上解码不了 HEIC（宽度口径的入口，语义与 Windows 版一致）。
    pub fn decode_to_width(_path: &Path, _width: u32, _frame: u32) -> Option<DecodedImage> {
        None
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub::{decode, decode_to_width, dimensions, frame_count, heif_available};
