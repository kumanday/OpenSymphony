//! OpenSymphony Tauri desktop entry point.
//!
//! Registers the native command surface and boots the Tauri v2 runtime.
//! Custom commands are gated by capability files in `src-tauri/capabilities/`.

use std::process;
use tauri::Manager;

mod commands;

fn main() {
    if let Err(e) = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            if let Some(_window) = app.get_webview_window("main") {
                // Window exists; future setup hooks can attach here.
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::open_folder,
            commands::notify,
            commands::get_setting,
            commands::set_setting,
            commands::daemon_status,
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("Tauri runtime error: {e}");
        process::exit(1);
    }
}
