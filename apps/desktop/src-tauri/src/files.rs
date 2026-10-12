//! Filesystem side of the batch actions exposed to the photo wall.
//!
//! 批量操作直接落在原文件上，所以这一层有三条硬规则：
//!
//! 1. **绝不静默覆盖。** 目标目录里已经有同名文件时追加 ` (1)`、` (2)`……
//!    覆盖等于在没有告知用户的情况下销毁另一张照片，而照片删掉就再也换不回来。
//! 2. **逐条报告成败。** 目标盘满了、某一张正被别的程序占用，用户要能看出是哪几张没成，
//!    所以结果是 Vec<BatchItemResult>，不是一个总数。
//! 3. **删除走可恢复的路径。** Lumenfold 自己的空状态就写着「只读取你指定的目录，
//!    不会搬动或复制原图」——删除按钮不该背过身去把原图烧掉。
//!    Windows 上走回收站（资源管理器里能「还原」），其他平台移进图库根目录下的废纸篓。

use std::path::{Path, PathBuf};

use crate::error::LumenError;
use crate::types::{BatchAction, BatchItemResult, BatchReport};

/// Trash folder created inside the library root on platforms without a system
/// trash. The leading dot keeps it out of the way in Finder.
#[cfg(not(target_os = "windows"))]
const TRASH_DIR: &str = ".Lumenfold 废纸篓";

/// One asset resolved to its real location on disk.
///
/// Asset ids are a hash of the path, so they cannot be turned back into a path
/// without the index. The command layer owns the index and does that lookup;
/// this module never sees an id it could not resolve.
#[derive(Debug, Clone)]
pub struct BatchEntry {
    pub asset_id: String,
    pub path: PathBuf,
    pub file_name: String,
}

/// Runs one batch action and reports every file's outcome.
///
/// Never fails as a whole: a batch where 40 of 50 copies succeed is a success
/// with 10 failures, and the UI has to be able to say exactly that.
/// `Err` is reserved for requests that make no sense at all.
pub fn operate(
    entries: &[BatchEntry],
    action: BatchAction,
    destination: Option<&Path>,
    roots: &[PathBuf],
) -> Result<BatchReport, LumenError> {
    if entries.is_empty() {
        return Err(LumenError::unsupported("没有选中任何照片"));
    }
    if action.needs_destination() && destination.is_none() {
        return Err(LumenError::unsupported("复制与剪切都需要先选一个目标文件夹"));
    }
    if roots.is_empty() {
        return Err(LumenError::no_library("还没有打开图库"));
    }

    let destination = destination.map(|dir| dir.to_path_buf());
    let items = entries
        .iter()
        .map(|entry| {
            // 图库现在可以挂多个目录，所以「根目录」是**每条记录各自**的那个：
            // 复制/剪切的默认落点、以及废纸篓放在哪，都该跟着文件走。
            // 用一个全局根会让 B 盘上的照片被丢进 A 盘的废纸篓。
            let root = root_for(&entry.path, roots);
            apply_one(entry, action, destination.as_deref(), root)
        })
        .collect();

    Ok(BatchReport::from_items(
        items,
        destination.map(|dir| dir.to_string_lossy().into_owned()),
    ))
}

/// 包含 `path` 的那个图库目录。都不包含时退回第一个——至少有个确定的落点，
/// 而不是让整个批量操作因为一个越界的路径全盘失败。
fn root_for<'a>(path: &Path, roots: &'a [PathBuf]) -> &'a Path {
    roots
        .iter()
        .find(|root| path.starts_with(root))
        .map(PathBuf::as_path)
        .unwrap_or_else(|| roots[0].as_path())
}

/// One asset, one outcome. A failure must not abort the rest of the batch.
fn apply_one(
    entry: &BatchEntry,
    action: BatchAction,
    destination: Option<&Path>,
    root: &Path,
) -> BatchItemResult {
    let result = match action {
        BatchAction::Copy | BatchAction::Move => {
            let target_dir = destination.unwrap_or(root);
            transfer(&entry.path, target_dir, action).map(Some)
        }
        BatchAction::Delete => trash(&entry.path, root).map(|_| None),
    };

    match result {
        Ok(path) => BatchItemResult {
            asset_id: entry.asset_id.clone(),
            file_name: entry.file_name.clone(),
            ok: true,
            message: String::new(),
            path: path.map(|p| p.to_string_lossy().into_owned()),
        },
        Err(message) => BatchItemResult {
            asset_id: entry.asset_id.clone(),
            file_name: entry.file_name.clone(),
            ok: false,
            message,
            path: None,
        },
    }
}

/// Copies or moves one file, never overwriting an existing one.
fn transfer(source: &Path, target_dir: &Path, action: BatchAction) -> Result<PathBuf, String> {
    // 移到自己所在的目录没有任何意义。放着不管的话 `unique_path` 会给它改个名，
    // 于是「移动」凭空多出一张照片——这比报错更难解释。
    if action == BatchAction::Move && source.parent() == Some(target_dir) {
        return Err("照片已经在目标文件夹里了".to_string());
    }
    let file_name = source
        .file_name()
        .ok_or_else(|| format!("{} 没有文件名", source.display()))?;
    let target = unique_path(target_dir, file_name);

    match action {
        BatchAction::Copy => {
            std::fs::copy(source, &target).map_err(|err| err.to_string())?;
        }
        BatchAction::Move => move_file(source, &target)?,
        // `operate` rejects this before any file is touched.
        BatchAction::Delete => return Err("删除不走传输路径".to_string()),
    }
    Ok(target)
}

/// `rename` when both sides share a volume, copy-then-delete when they do not.
///
/// A cross-volume move through `rename` fails outright on Windows; falling back
/// to copy + remove is what makes "移动到另一块硬盘" work at all.
fn move_file(source: &Path, target: &Path) -> Result<(), String> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(source, target).map_err(|err| err.to_string())?;
            // If the delete fails the file now exists in both places, which is
            // better than losing it — so the copy counts and the report says why.
            std::fs::remove_file(source).map_err(|err| format!("已复制到目标目录，但删除源文件失败：{err}"))
        }
    }
}

/// `name.ext`, then `name (1).ext`, `name (2).ext`, ...
///
/// The counter goes before the extension so Explorer still groups and sorts the
/// results next to the originals instead of scattering them alphabetically.
fn unique_path(dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let text = file_name.to_string_lossy();
    let (base, extension) = match text.rsplit_once('.') {
        Some((base, extension)) if !base.is_empty() => {
            (base.to_string(), format!(".{extension}"))
        }
        _ => (text.to_string(), String::new()),
    };

    let direct = dir.join(file_name);
    if !direct.exists() {
        return direct;
    }
    for n in 1..10_000u32 {
        let candidate = dir.join(format!("{base} ({n}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // 10000 个同名文件几乎不会真出现；真出现了就带上进程号而不是覆盖。
    dir.join(format!("{base} ({}){extension}", std::process::id()))
}

/// Sends one file somewhere the user can get it back from.
fn trash(source: &Path, root: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // Windows 走系统回收站，图库根目录派不上用场。
        let _ = root;
        recycle_bin(source)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let bin = root.join(TRASH_DIR).join(stamp.to_string());
        std::fs::create_dir_all(&bin).map_err(|err| err.to_string())?;
        let target = unique_path(
            &bin,
            source.file_name().unwrap_or_else(|| std::ffi::OsStr::new("")),
        );
        move_file(source, &target)
    }
}

/// Windows Recycle Bin through `SHFileOperationW`.
///
/// 这是唯一能让资源管理器给出「还原」的做法：`FOF_ALLOWUNDO` 打开撤销支持，
/// `FOFX_RECYCLEONDELETE` 明确要求进回收站而不是永久删除，
/// 两者缺一个，文件就只是从目录里消失了。
#[cfg(target_os = "windows")]
fn recycle_bin(source: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt as _;

    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FO_DELETE, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION,
        FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    };

    // `pFrom` 要的是一份双 NUL 结尾的路径列表，所以指向整个缓冲区，
    // 而不是某个 CStr——单条路径也必须带两个结尾 NUL。
    let from: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0).chain(std::iter::once(0)))
        .collect();

    // 这组标志位是「进回收站」的充分条件，缺一个就变成永久删除：
    // FOF_ALLOWUNDO 打开撤销支持，FOFX_RECYCLEONDELETE 明确要求进回收站。
    let flags = (FOF_ALLOWUNDO
        | FOFX_RECYCLEONDELETE
        | FOF_NOCONFIRMATION
        | FOF_NOERRORUI
        | FOF_SILENT)
        .0 as u16;

    let mut op = SHFILEOPSTRUCTW {
        hwnd: Default::default(),
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        pTo: PCWSTR::null(),
        fFlags: flags,
        fAnyOperationsAborted: BOOL(0),
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: PCWSTR::null(),
    };

    // SAFETY: `op` is a live, fully initialised struct, and `from` is an owned
    // NUL-terminated UTF-16 buffer that outlives the call. `SHFileOperationW`
    // is documented to leave `op` valid and to report the "aborted" flag in
    // place, which is why the struct is passed by pointer and read afterwards.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        return Err(format!("移入回收站失败（SHFileOperationW 返回 {code}）"));
    }
    if op.fAnyOperationsAborted.as_bool() {
        return Err("移入回收站的操作被取消".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "lumenfold-files-{}-{}-{}",
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

    fn entry(dir: &Path, name: &str) -> BatchEntry {
        let path = dir.join(name);
        std::fs::write(&path, b"pixels").expect("write source");
        BatchEntry {
            asset_id: format!("id-{name}"),
            path,
            file_name: name.to_string(),
        }
    }


    #[test]
    fn copy_never_overwrites_an_existing_file() {
        let dir = TempDir::new("copy");
        let out = TempDir::new("copy-out");
        std::fs::write(out.0.join("a.jpg"), b"do not touch").expect("seed destination");

        let report = operate(
            &[entry(&dir.0, "a.jpg")],
            BatchAction::Copy,
            Some(&out.0),
            &[dir.0.clone()],
        )
        .expect("copy");

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(
            std::fs::read(out.0.join("a.jpg")).expect("read original"),
            b"do not touch",
            "the file already in the destination must be left alone"
        );
        assert!(out.0.join("a (1).jpg").is_file(), "the copy gets a new name");
        assert_eq!(
            report.items[0].path.as_deref(),
            Some(out.0.join("a (1).jpg").to_string_lossy().as_ref()),
        );
    }

    #[test]
    fn move_relocates_the_file_and_leaves_nothing_behind() {
        let dir = TempDir::new("move");
        let out = TempDir::new("move-out");
        let source = entry(&dir.0, "a.jpg");

        let report =
            operate(&[source], BatchAction::Move, Some(&out.0), &[dir.0.clone()]).expect("move");

        assert_eq!(report.succeeded, 1);
        assert!(out.0.join("a.jpg").is_file());
        assert!(!dir.0.join("a.jpg").exists(), "the source is gone after a move");
    }

    #[test]
    fn one_failure_does_not_stop_the_rest_of_the_batch() {
        let dir = TempDir::new("partial");
        let out = TempDir::new("partial-out");
        let good = entry(&dir.0, "good.jpg");
        let missing = BatchEntry {
            asset_id: "id-gone".into(),
            path: dir.0.join("gone.jpg"),
            file_name: "gone.jpg".into(),
        };

        let report = operate(
            &[missing, good],
            BatchAction::Copy,
            Some(&out.0),
            &[dir.0.clone()],
        )
        .expect("batch runs");

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 1);
        let failed_item = report.items.iter().find(|i| !i.ok).expect("one failure");
        assert_eq!(failed_item.asset_id, "id-gone");
        assert!(!failed_item.message.is_empty(), "a failure must say why");
        assert!(out.0.join("good.jpg").is_file());
    }

    #[test]
    fn delete_is_recoverable_not_a_permanent_unlink() {
        let dir = TempDir::new("delete");
        let source = entry(&dir.0, "a.jpg");

        let report = operate(&[source], BatchAction::Delete, None, &[dir.0.clone()]).expect("delete");

        assert_eq!(report.succeeded, 1);
        assert!(!dir.0.join("a.jpg").exists(), "the file leaves the library");

        // 找不回来就是永久删除。这里断言文件确实还在某个地方——
        // Windows 上是回收站，其余平台是图库根目录下的废纸篓。
        #[cfg(target_os = "windows")]
        {
            // 回收站里的原路径已经不存在了，能证明的只有这次调用没有报错。
            // 真正的「还原」由资源管理器负责，这里不越界去读它的内部目录。
        }
        #[cfg(not(target_os = "windows"))]
        {
            let trash_root = dir.0.join(TRASH_DIR);
            assert!(trash_root.is_dir(), "废纸篓目录应当被创建");
            let restored = walkdir_files(&trash_root);
            assert_eq!(restored.len(), 1, "文件应当被移进废纸篓而不是消失");
            assert!(restored[0].ends_with("a.jpg"));
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn walkdir_files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir_files(&path));
            } else {
                out.push(path.to_string_lossy().into_owned());
            }
        }
        out
    }

    #[test]
    fn an_empty_selection_is_rejected_rather_than_silently_succeeding() {
        let dir = TempDir::new("empty");
        let err = operate(&[], BatchAction::Copy, Some(&dir.0), &[dir.0.clone()])
            .expect_err("empty selection must be an error");
        assert!(matches!(err, LumenError::Unsupported(_)));
    }

    #[test]
    fn copy_without_a_destination_is_rejected() {
        let dir = TempDir::new("nodest");
        let entry = entry(&dir.0, "a.jpg");
        let err = operate(&[entry], BatchAction::Copy, None, &[dir.0.clone()])
            .expect_err("copy needs a destination");
        assert!(matches!(err, LumenError::Unsupported(_)));
    }

    #[test]
    fn unique_path_counts_up_instead_of_overwriting() {
        let dir = TempDir::new("unique");
        std::fs::write(dir.0.join("photo.jpg"), b"1").expect("seed");
        assert_eq!(
            unique_path(&dir.0, std::ffi::OsStr::new("photo.jpg")),
            dir.0.join("photo (1).jpg")
        );
        std::fs::write(dir.0.join("photo (1).jpg"), b"2").expect("seed");
        assert_eq!(
            unique_path(&dir.0, std::ffi::OsStr::new("photo.jpg")),
            dir.0.join("photo (2).jpg")
        );
    }

    #[test]
    fn unique_path_leaves_extensionless_names_alone() {
        let dir = TempDir::new("noext");
        std::fs::write(dir.0.join("README"), b"1").expect("seed");
        assert_eq!(
            unique_path(&dir.0, std::ffi::OsStr::new("README")),
            dir.0.join("README (1)")
        );
    }
}
