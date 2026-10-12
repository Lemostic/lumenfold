//! 缩略图吞吐实测：拿真实照片对比「全尺寸解码」与「DCT 缩放解码」。
//! 用法：cargo run --release --example thumbbench -- <目录>

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lumenfold_desktop_lib::thumbnail;

fn main() {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: thumbbench <dir>");
        std::process::exit(2);
    };
    let files = collect(Path::new(&root), 0);
    if files.is_empty() {
        eprintln!("no photos under {root}");
        return;
    }

    // 第一轮用来把文件读进系统页缓存，否则测的是磁盘而不是解码。
    for file in &files {
        let _ = std::fs::read(file);
    }

    println!("{} 张照片，先读一遍预热文件缓存：", files.len());
    for width in [256u32, 384] {
        let full = time(&files, width, thumbnail::encode_generic);
        let shrunk = time(&files, width, thumbnail::encode);
        println!(
            "w={width:<4} 全尺寸解码 {:>6.1} ms/张   缩放解码 {:>6.1} ms/张   快 {:.1}x",
            full.0,
            shrunk.0,
            full.0 / shrunk.0,
        );
        println!(
            "          单张 {:>5.1} KiB      单张 {:>5.1} KiB      整屏 30 张 {:?} -> {:?}",
            full.1 as f64 / 1024.0 / files.len() as f64,
            shrunk.1 as f64 / 1024.0 / files.len() as f64,
            full.2,
            shrunk.2,
        );
    }
}

fn time(
    files: &[PathBuf],
    width: u32,
    encode: fn(&Path, u32) -> Result<lumenfold_desktop_lib::library::CachedThumb, lumenfold_desktop_lib::error::LumenError>,
) -> (f64, usize, Duration) {
    let mut total = Duration::ZERO;
    let mut bytes = 0usize;
    let wall = Instant::now();
    for file in files {
        let t0 = Instant::now();
        match encode(file, width) {
            Ok(thumb) => {
                total += t0.elapsed();
                bytes += thumb.bytes.len();
            }
            Err(err) => eprintln!("skip {}: {err:?}", file.display()),
        }
    }
    (
        total.as_secs_f64() * 1000.0 / files.len() as f64,
        bytes,
        wall.elapsed(),
    )
}

fn collect(root: &Path, depth: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
            out.push(path);
            if out.len() >= 40 {
                return out;
            }
        } else if depth < 2 && path.is_dir() {
            out.extend(collect(&path, depth + 1));
        }
    }
    out
}