//! Lumenfold desktop backend.

use std::sync::Mutex;

use tauri::Manager;

pub mod clipboard;
pub mod commands;
pub mod discover;
pub mod enhance;
pub mod enhance_tasks;
pub mod error;
pub mod files;
pub mod library;
pub mod library_index;
pub mod models;
pub mod thumbnail;
pub mod thumbstore;
pub mod types;
pub mod worker_pool;

/// Starts the Tauri application and blocks until the main window closes.
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard::init())
        .manage(Mutex::new(library::LibraryState::empty()))
        // 模型状态和图库状态一样是单锁结构，下载线程只往里写字节数。
        .manage(Mutex::new(models::ModelState::empty()))
        // 缩略图磁盘缓存。缓存目录要 `AppHandle` 才拿得到，而 `manage` 只能挂在
        // builder 上，所以这里先注册空壳，目录和预算在 `setup` 里补上。
        .manage(thumbstore::ThumbStore::default());

    // The protocol has to be registered before `setup` so the WebView can
    // resolve `lumen://` URLs from its very first paint.
    let app = thumbnail::register(builder)
        .setup(|app| {
            // 画质增强的运行时位置必须在**任何** ort 调用之前告诉它。
            // `load-dynamic` 是第一次建会话时才去加载动态库的，之后再设环境变量
            // 已经晚了。失败不拦启动：随包的 DLL 丢了只会让增强不可用，
            // 图库、搜索、缩略图都不该跟着一起打不开。
            if let Err(err) = enhance::install_runtime_path(app.handle()) {
                eprintln!("enhancement runtime unavailable: {err}");
            }
            // 开机时那个「不扫目录」的字节计数要尽快变成真值，否则状态栏会在
            // 接下来的一整轮扫描里一直说谎。这活要读整个缓存目录，交给后台
            // 线程，别挡住窗口出现。
            let store = app.state::<thumbstore::ThumbStore>();
            store.init(app.handle());
            store.reconcile_detached();
            // Reopen the folder from the previous run, if it still exists. Any
            // failure here is silent by design: a stale `library.json` is not
            // worth refusing to start over.
            library::restore(app.handle());
            // 上次跑完的权重要先对一遍摘要再对外显示为 ready。
            models::install_crypto_provider();
            models::reconcile(app.handle());
            enhance_tasks::restore(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::library_stats,
            commands::open_library,
            commands::set_library_roots,
            commands::discover_libraries,
            commands::rescan,
            commands::cancel_scan,
            commands::set_thumb_cache_limit,
            commands::clear_thumb_cache,
            commands::batch_operate,
            clipboard::clipboard_files,
            commands::query_assets,
            commands::asset_tag_counts,
            commands::asset_metadata,
            commands::edit_image,
            commands::enhance_progress,
            commands::cancel_enhance,
                         enhance_tasks::start_enhancement,
             enhance_tasks::enhancement_tasks,
             commands::list_people,
            commands::search,
            models::list_models,
            models::start_model_download,
            models::cancel_model_download,
        ]);

    app.run(tauri::generate_context!())
        .expect("failed to start the Lumenfold desktop application");
}

#[cfg(test)]
mod tests {
    //! 一个用来防住一类可以静默发生的故障的用例。

    /// 前端调用的命令名必须在 Rust 这边真的存在。
    ///
    /// Tauri 的命令名是从**函数名**推出来的，不是路径。把
    /// `list_models` 改成 `list` 会把 wire 契约静默改掉，
    /// 而 Rust 单测调的是函数本身、不经过 IPC，所以测试全绿也看不出来。
    /// 这条用例把前端的 `invoke(\"...\")` 和注册表对上，名字对不上就构建失败。
    #[test]
    fn every_command_the_frontend_invokes_is_registered() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let ipc = repo.join("../src/lib/ipc.ts");
        let handlers = std::fs::read_to_string(&ipc)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", ipc.display()));

        let registered = std::fs::read_to_string("src/lib.rs").expect("read src/lib.rs");

        // 一条命令都没扫到的时候验过是空转，会让这条用例永远绿着。
        let mut checked = 0usize;
        let mut missing = Vec::new();
        for line in handlers.lines() {
            let Some(start) = line.find("invoke<") else { continue };
            let rest = &line[start..];
            let Some(open) = rest.find('"') else { continue };
            let rest = &rest[open + 1..];
            let Some(end) = rest.find('"') else { continue };
            let name = &rest[..end];
            checked += 1;

            // `generate_handler!` 里的每一行都是命令的最后一段路径。
            let registered_line = registered
                .lines()
                .any(|candidate| candidate.trim().trim_end_matches(',').ends_with(name));
            if !registered_line {
                missing.push(name);
            }
        }

        assert!(checked >= 13, "只扫到 {checked} 条命令，解析可能已经失效");
        assert!(
            missing.is_empty(),
            "前端调用了没有注册的命令：{}\n\
             Tauri 的命令名取自函数名，改名就等于改了 wire 契约。",
            missing.join(", ")
        );
    }
}
