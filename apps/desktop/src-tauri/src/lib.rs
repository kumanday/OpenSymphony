//! OpenSymphony Tauri desktop shell library.
//!
//! Exports Tauri commands that are explicitly scoped via capability files.
//! Each command uses narrow request and response types to limit attack surface.

use std::process;

use tauri::Manager;

mod actions;
pub mod commands;
pub mod daemon;
mod keychain;
mod settings;
pub mod types;

pub fn run() {
    let desktop_state = commands::DesktopState::new();

    if let Err(e) = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(desktop_state)
        .setup(|app| {
            if let Some(_window) = app.get_webview_window("main") {
                // Window exists; future setup hooks can attach here.
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_setting,
            settings::set_setting,
            keychain::get_credential,
            keychain::set_credential,
            keychain::delete_credential,
            keychain::credential_status,
            actions::open_file,
            actions::open_folder,
            actions::open_repository_folder,
            actions::reveal_workspace,
            actions::copy_to_clipboard,
            actions::open_linear_link,
            actions::notify,
            commands::daemon_status,
            commands::store_profile,
            commands::list_profiles,
            commands::set_active_profile,
            commands::probe_gateway,
            commands::discover_default_gateway,
            commands::start_daemon,
            commands::stop_daemon,
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("Tauri runtime error: {e}");
        process::exit(1);
    }
}
