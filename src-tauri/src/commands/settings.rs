use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime, State};

use crate::app_state::AppState;
use crate::codex::auth::AccountState;
use crate::settings::store::{FileSettingsBackend, SettingsStore};
use crate::settings::types::{AppSettings, AvailableModel};

const SETTINGS_FILE: &str = "smartcat-settings.json";

pub fn open_store<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<SettingsStore<FileSettingsBackend>, String> {
    Ok(SettingsStore::new(FileSettingsBackend::new(settings_path(
        app,
    )?)))
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|_| "settings_persistence_failed".to_owned())
}

#[tauri::command]
pub async fn get_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    open_store(&app)?.load().await.map_err(settings_error_code)
}

#[tauri::command]
pub async fn save_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    open_store(&app)?
        .save(&settings)
        .await
        .map_err(settings_error_code)?;
    app.state::<crate::lifecycle::LifecycleState>()
        .set_locale(settings.locale, state.hotkeys_suspended());
    Ok(settings)
}

#[tauri::command]
pub async fn list_available_models(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, String> {
    read_authoritative_models(&state).await
}

pub(crate) async fn read_authoritative_models(
    state: &AppState,
) -> Result<Vec<AvailableModel>, String> {
    let account = state
        .account_service()
        .await
        .ok_or_else(|| "model_catalog_unavailable".to_owned())?
        .read()
        .await
        .map_err(|_| "model_catalog_unavailable".to_owned())?;
    if matches!(account, AccountState::SignedOut) {
        return Err("model_catalog_signed_out".to_owned());
    }
    state
        .model_catalog_service()
        .await
        .ok_or_else(|| "model_catalog_unavailable".to_owned())?
        .list()
        .await
        .map_err(|error| error.code().to_owned())
}

fn settings_error_code(error: crate::settings::types::SettingsError) -> String {
    error.code().to_owned()
}
