use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime, State};

use crate::app_state::AppState;
use crate::settings::store::{SettingsStore, TauriStoreBackend};
use crate::settings::types::{AppSettings, AvailableModel};

const SETTINGS_FILE: &str = "smartcat-settings.json";

pub fn open_store<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<SettingsStore<TauriStoreBackend<R>>, String> {
    let store = tauri_plugin_store::StoreBuilder::new(app, SETTINGS_FILE)
        .disable_auto_save()
        .build()
        .map_err(|_| "settings_persistence_failed".to_owned())?;
    let path = settings_path(app)?;
    Ok(SettingsStore::new(TauriStoreBackend::new(store, path)))
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
    let _operation = state.lock_settings_operation().await;
    open_store(&app)?.load().await.map_err(settings_error_code)
}

#[tauri::command]
pub async fn save_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let _operation = state.lock_settings_operation().await;
    open_store(&app)?
        .save(&settings)
        .await
        .map_err(settings_error_code)?;
    Ok(settings)
}

#[tauri::command]
pub async fn list_available_models(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, String> {
    let service = state
        .model_catalog_service()
        .await
        .ok_or_else(|| "model_catalog_unavailable".to_owned())?;
    service
        .list()
        .await
        .map_err(|error| error.code().to_owned())
}

fn settings_error_code(error: crate::settings::types::SettingsError) -> String {
    error.code().to_owned()
}
