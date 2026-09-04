use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::codex::auth::{AccountChangeReason, AccountEventSink, AccountState};
use crate::codex::bootstrap::bootstrap_account_service;
use crate::commands::account::TauriAccountEventSink;
use crate::storage::KeyStore;

pub mod app_state;
pub mod capture;
pub mod codex;
pub mod commands;
pub mod core;
pub mod documents;
pub mod hotkeys;
pub mod lifecycle;
pub mod platform;
pub mod settings;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let document_jobs = Arc::new(documents::DocumentJobStore::default());
    let document_jobs_for_setup = document_jobs.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(app_state::AppState::default())
        .manage(capture::CaptureCoordinator::default())
        .manage(capture::CaptureJobStore::default())
        .manage(document_jobs)
        .manage(commands::update::UpdateState::default())
        .setup(move |app| {
            lifecycle::setup(app)?;
            let app_data_root = app.path().app_local_data_dir()?;
            let key = match storage::OsKeyStore.load_or_create() {
                Ok(key) => key,
                Err(error) => {
                    let _ = app
                        .dialog()
                        .message("Secure local storage is unavailable. Enable Windows Credential Manager or unlock macOS Keychain, then restart the app.")
                        .title("BYOK Translator")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    crate::core::diagnostics::DiagnosticEvent::new(
                        crate::core::diagnostics::DiagnosticEventName::SecureStorage,
                        crate::core::diagnostics::DiagnosticOutcome::Failed,
                    )
                    .with_error_code("secure_storage_unavailable")
                    .emit();
                    return Err(error.into());
                }
            };
            let crypto = Arc::new(storage::CryptoBox::from_zeroizing(key));
            let database = storage::StorageDatabase::open(
                &app_data_root
                    .join("storage")
                    .join("smartcat-private.sqlite3"),
            )?;
            let history_store =
                Arc::new(storage::HistoryStore::new(database.clone(), crypto.clone()));
            let job_store = Arc::new(storage::JobStore::new(database, crypto));
            let _ = job_store.purge_expired();
            document_jobs_for_setup.install_resume_backend(Arc::new(
                storage::EncryptedDocumentResumeBackend::new(
                    job_store.clone(),
                    document_jobs_for_setup.clone(),
                ),
            ));
            let cleanup = storage::CleanupService::new(app_data_root.join("private-temp"))?;
            commands::documents::set_private_permissions(cleanup.root(), true);
            app.manage(history_store);
            app.manage(job_store);
            app.manage(cleanup.clone());
            let _ = cleanup.on_start();
            let resource_root = app.path().resource_dir()?;
            let executable_path = std::env::current_exe()?;
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<app_state::AppState>();
                let event_sink = Arc::new(TauriAccountEventSink(app_handle.clone()));
                let bootstrap = bootstrap_account_service(
                    &state,
                    app_data_root,
                    resource_root,
                    executable_path,
                    event_sink.clone(),
                )
                .await;
                if bootstrap.is_ok() {
                    event_sink.account_state_changed(AccountChangeReason::AccountUpdated);
                    let account_ready = match state.account_service().await {
                        Some(service) => {
                            !matches!(service.read().await, Ok(AccountState::SignedOut))
                        }
                        None => false,
                    };
                    app_handle
                        .state::<lifecycle::LifecycleState>()
                        .set_account_ready(account_ready);
                } else {
                    state.mark_account_runtime_failed();
                }
                let _ = commands::windows::restart_quick_hotkeys(app_handle.clone()).await;
                match commands::settings::open_store(&app_handle) {
                    Ok(store) => match store.load().await {
                        Ok(settings) => {
                        if let Some(history) = app_handle.try_state::<Arc<storage::HistoryStore>>() {
                                if history
                                    .purge_expired(settings.history_retention_days)
                                    .is_err()
                                {
                                    crate::core::diagnostics::DiagnosticEvent::new(
                                        crate::core::diagnostics::DiagnosticEventName::HistoryMaintenance,
                                        crate::core::diagnostics::DiagnosticOutcome::Failed,
                                    )
                                    .with_error_code("history_retention_invalid")
                                    .emit();
                                }
                        }
                        app_handle
                            .state::<lifecycle::LifecycleState>()
                            .set_locale(settings.locale, state.hotkeys_suspended());
                        }
                        Err(_) => crate::core::diagnostics::DiagnosticEvent::new(
                            crate::core::diagnostics::DiagnosticEventName::HistoryMaintenance,
                            crate::core::diagnostics::DiagnosticOutcome::Failed,
                        )
                        .with_error_code("history_retention_pending")
                        .emit(),
                    },
                    Err(_) => crate::core::diagnostics::DiagnosticEvent::new(
                        crate::core::diagnostics::DiagnosticEventName::HistoryMaintenance,
                        crate::core::diagnostics::DiagnosticOutcome::Failed,
                    )
                    .with_error_code("history_retention_pending")
                    .emit(),
                }
                commands::history::emit_privacy_status(&app_handle);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::account::get_account,
            commands::account::get_rate_limits,
            commands::account::start_chatgpt_login,
            commands::account::cancel_chatgpt_login,
            commands::hotkeys::analyze_hotkey,
            commands::hotkeys::save_hotkey,
            commands::hotkeys::list_hotkeys,
            commands::hotkeys::list_blocked_apps,
            commands::hotkeys::save_blocked_apps,
            commands::hotkeys::suspend_hotkeys,
            commands::lifecycle::get_lifecycle_status,
            commands::lifecycle::set_launch_at_login,
            commands::lifecycle::set_close_behavior,
            commands::lifecycle::set_quick_access_position,
            commands::lifecycle::set_hotkeys_paused,
            commands::lifecycle::quit_application,
            commands::translation::translate_text,
            commands::translation::cancel_translation,
            commands::translation_save::save_translation_text,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::list_available_models,
            commands::history::save_history_record,
            commands::history::list_history,
            commands::history::read_history,
            commands::history::delete_history,
            commands::history::delete_all_history,
            commands::history::get_history_policy,
            commands::history::get_privacy_status,
            commands::history::purge_history,
            commands::history::list_recoverable_jobs,
            commands::history::prepare_document_recovery,
            commands::history::delete_recovery_job,
            commands::windows::show_quick_popup,
            commands::windows::close_quick_popup,
            commands::windows::open_main_window,
            commands::capture::start_screen_capture,
            commands::capture::get_capture_overlay,
            commands::capture::update_screen_selection,
            commands::capture::complete_screen_capture,
            commands::capture::cancel_screen_capture,
            commands::capture::choose_image,
            commands::capture::translate_image,
            commands::capture::cancel_image_translation,
            commands::capture::update_capture_block,
            commands::capture::export_translated_image,
            commands::documents::choose_document,
            commands::documents::inspect_document_path,
            commands::documents::translate_document,
            commands::documents::cancel_document_translation,
            commands::documents::open_document_result,
            commands::documents::open_document_folder,
            commands::documents::get_document_result_preview,
            commands::documents::choose_document_output_directory,
            commands::update::check_for_update,
            commands::update::prepare_update,
            commands::update::install_update,
            commands::update::authorize_update_restart,
            commands::update::mark_app_healthy,
            commands::update::get_update_recovery_instructions,
            commands::update::open_previous_installer,
            commands::update::open_update_release,
        ])
        .build(tauri::generate_context!())
        .expect("failed to run BYOK Translator");
    app.run(|app_handle, event| {
        if let tauri::RunEvent::WindowEvent { label, event, .. } = &event {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if commands::capture::is_capture_overlay_window(label) {
                    api.prevent_close();
                    let _ = commands::capture::handle_capture_overlay_close(app_handle, label);
                } else if app_handle
                    .state::<lifecycle::LifecycleState>()
                    .should_intercept_close()
                    && lifecycle::should_intercept_window_close(label)
                {
                    api.prevent_close();
                    lifecycle::handle_window_close(app_handle, label);
                }
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let state = app_handle.state::<app_state::AppState>();
                let job_ids = state.tombstone_window_translation_jobs(label);
                let app_handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_handle.state::<app_state::AppState>();
                    state.cancel_tombstoned_translation_jobs(&job_ids).await;
                });
            }
        }
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(jobs) = app_handle.try_state::<Arc<storage::JobStore>>() {
                jobs.clear_secret();
            }
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
