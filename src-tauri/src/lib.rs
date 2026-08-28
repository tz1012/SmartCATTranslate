use std::sync::Arc;

use tauri::Manager;

use crate::codex::auth::{AccountChangeReason, AccountEventSink};
use crate::codex::bootstrap::bootstrap_account_service;
use crate::commands::account::TauriAccountEventSink;

pub mod app_state;
pub mod codex;
pub mod commands;
pub mod core;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state::AppState::default())
        .setup(|app| {
            let app_data_root = app.path().app_local_data_dir()?;
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<app_state::AppState>();
                let event_sink = Arc::new(TauriAccountEventSink(app_handle.clone()));
                if bootstrap_account_service(&state, app_data_root, event_sink.clone())
                    .await
                    .is_ok()
                {
                    event_sink.account_state_changed(AccountChangeReason::AccountUpdated);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::account::get_account,
            commands::account::get_rate_limits,
            commands::account::start_chatgpt_login,
            commands::account::cancel_chatgpt_login,
        ])
        .build(tauri::generate_context!())
        .expect("failed to run SmartCAT Translate");
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let state = app_handle.state::<app_state::AppState>();
            let _ = tauri::async_runtime::block_on(state.shutdown());
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_is_stable() {
        assert_eq!(env!("CARGO_PKG_NAME"), "smartcat-translate");
    }
}
