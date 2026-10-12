//! Copies indexed files to the system clipboard without changing the originals.
use tauri::{AppHandle, Manager, State};
use crate::{error::LumenError, library::SharedLibrary};

#[tauri::command]
pub async fn clipboard_files(
    app: AppHandle,
    state: State<'_, SharedLibrary>,
    asset_ids: Vec<String>,
    cut: bool,
) -> Result<usize, LumenError> {
    if asset_ids.is_empty() || asset_ids.len() > 1000 {
        return Err(LumenError::unsupported("请选择 1 至 1000 张照片"));
    }
    #[cfg(not(target_os = "windows"))]
    if cut { return Err(LumenError::unsupported("当前平台还不支持文件剪切")); }
    let paths = {
        let guard = state.lock().expect("library state poisoned");
        if guard.roots.is_empty() { return Err(LumenError::no_library("还没有打开图库")); }
        let mut paths = Vec::with_capacity(asset_ids.len());
        for id in &asset_ids {
            let record = guard.index.get(id).and_then(|index| guard.assets.get(*index))
                .ok_or_else(|| LumenError::io(format!("文件已从索引消失：{id}")))?;
            if !record.path.is_file() {
                return Err(LumenError::io(format!("文件已不存在：{}", record.path.display())));
            }
            paths.push(record.path.to_string_lossy().into_owned());
        }
        paths
    };
    app.state::<tauri_plugin_clipboard::Clipboard>()
        .write_files_uris(paths)
        .map_err(LumenError::io)?;
    #[cfg(target_os = "windows")]
    set_preferred_drop_effect(cut)?;
    Ok(asset_ids.len())
}

/// Explorer interprets CF_HDROP plus Preferred DropEffect=2 as Ctrl+X.
/// The clipboard plugin writes CF_HDROP first; this adds the second format without clearing it.
#[cfg(target_os = "windows")]
fn set_preferred_drop_effect(cut: bool) -> Result<(), LumenError> {
    use windows::core::w;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    // The global handle becomes clipboard-owned only after SetClipboardData succeeds.
    unsafe {
        OpenClipboard(None).map_err(|e| LumenError::io(e.to_string()))?;
        let result = (|| {
            let format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            if format == 0 { return Err(LumenError::io("无法注册剪贴板文件操作格式")); }
            let memory = GlobalAlloc(GMEM_MOVEABLE, 4).map_err(|e| LumenError::io(e.to_string()))?;
            let bytes = GlobalLock(memory) as *mut u8;
            if bytes.is_null() {
                let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
                return Err(LumenError::io("无法写入剪贴板内存"));
            }
            std::ptr::copy_nonoverlapping((if cut { 2u32 } else { 5u32 }).to_le_bytes().as_ptr(), bytes, 4);
            let _ = GlobalUnlock(memory);
            if let Err(error) = SetClipboardData(format, Some(HANDLE(memory.0))) {
                let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
                return Err(LumenError::io(error.to_string()));
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}
