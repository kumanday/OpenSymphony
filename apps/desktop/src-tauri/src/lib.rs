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
pub mod opensymphony_gateway_schema;
mod settings;
pub mod types;

pub fn run() {
    let desktop_state = commands::DesktopState::new();
    let subscription_state = commands::SubscriptionState::default();
    let gateway_connection = tokio::sync::RwLock::new(commands::GatewayConnection::default());

    if let Err(e) = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(desktop_state)
        .manage(subscription_state)
        .manage(gateway_connection)
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
            commands::remove_profile,
            commands::probe_gateway,
            commands::discover_default_gateway,
            commands::start_daemon,
            commands::stop_daemon,
            // COE-410: Gateway local stream transport commands
            commands::attach_gateway,
            commands::dashboard_snapshot,
            commands::task_graph,
            commands::run_detail,
            commands::run_files,
            commands::run_diffs,
            commands::run_validation,
            commands::run_approvals,
            commands::run_events,
            commands::terminal_snapshot,
            commands::get_connection_profiles,
            commands::gateway_capabilities,
            commands::gateway_connection_info,
            commands::subscribe_events,
            commands::subscribe_terminal,
            commands::unsubscribe_events,
            commands::unsubscribe_terminal,
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("Tauri runtime error: {e}");
        process::exit(1);
    }
}
