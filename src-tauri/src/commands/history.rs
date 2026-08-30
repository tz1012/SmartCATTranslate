use crate::{
    commands::settings::open_store,
    storage::{
        CleanupService, HistoryPage, HistoryPolicy, HistoryStore, JobStore, NewHistoryRecord,
        PreparedDocumentRecovery, RecoverableJob,
    },
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, Manager, Runtime};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyStatus {
    pub cleanup_pending: bool,
    pub retention_pending: bool,
}

#[tauri::command]
pub fn save_history_record(
    app: tauri::AppHandle,
    record: NewHistoryRecord,
) -> Result<Option<String>, String> {
    let store = app
        .try_state::<Arc<HistoryStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?;
    store
        .save(record)
        .map_err(|_| "history_write_failed".to_owned())
}

#[tauri::command]
pub fn list_history(
    app: tauri::AppHandle,
    limit: u32,
    cursor: Option<String>,
) -> Result<HistoryPage, String> {
    let store = app
        .try_state::<Arc<HistoryStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?;
    store
        .list(limit, cursor.as_deref())
        .map_err(|_| "history_read_failed".to_owned())
}
#[tauri::command]
pub fn read_history(
    app: tauri::AppHandle,
    id: String,
) -> Result<Option<crate::storage::HistoryRecord>, String> {
    app.try_state::<Arc<HistoryStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .read(&id)
        .map_err(|_| "history_read_failed".to_owned())
}
#[tauri::command]
pub fn delete_history(app: tauri::AppHandle, id: String) -> Result<bool, String> {
    app.try_state::<Arc<HistoryStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .delete(&id)
        .map_err(|_| "history_delete_failed".to_owned())
}
#[tauri::command]
pub fn delete_all_history(app: tauri::AppHandle) -> Result<u64, String> {
    app.try_state::<Arc<HistoryStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .delete_all()
        .map_err(|_| "history_delete_failed".to_owned())
}
#[tauri::command]
pub async fn get_history_policy(app: tauri::AppHandle) -> Result<HistoryPolicy, String> {
    let settings = open_store(&app)?
        .load()
        .await
        .map_err(|e| e.code().to_owned())?;
    if let Some(history) = app.try_state::<Arc<HistoryStore>>() {
        history
            .configure_retention(settings.history_retention_days)
            .map_err(|_| "history_retention_invalid".to_owned())?;
    }
    Ok(HistoryPolicy {
        enabled: true,
        retention_days: settings.history_retention_days,
    })
}

#[tauri::command]
pub fn get_privacy_status(app: tauri::AppHandle) -> PrivacyStatus {
    privacy_status(&app)
}

pub(crate) fn emit_privacy_status<R: Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("privacy-status", privacy_status(app));
}

fn privacy_status<R: Runtime>(app: &tauri::AppHandle<R>) -> PrivacyStatus {
    PrivacyStatus {
        cleanup_pending: app
            .try_state::<CleanupService>()
            .is_some_and(|cleanup| cleanup.has_pending()),
        retention_pending: app
            .try_state::<Arc<HistoryStore>>()
            .is_none_or(|history| !history.retention_configured()),
    }
}
#[tauri::command]
pub async fn purge_history(app: tauri::AppHandle) -> Result<u64, String> {
    let settings = open_store(&app)?
        .load()
        .await
        .map_err(|e| e.code().to_owned())?;
    app.try_state::<Arc<HistoryStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .purge_expired(settings.history_retention_days)
        .map_err(|_| "history_purge_failed".to_owned())
}

#[tauri::command]
pub fn list_recoverable_jobs(app: tauri::AppHandle) -> Result<Vec<RecoverableJob>, String> {
    app.try_state::<Arc<JobStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .recoverable()
        .map_err(|_| "recovery_read_failed".to_owned())
}
#[tauri::command]
pub fn prepare_document_recovery(
    app: tauri::AppHandle,
    record_id: String,
) -> Result<PreparedDocumentRecovery, String> {
    app.try_state::<Arc<JobStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .prepare_document(&record_id)
        .map_err(|_| "recovery_mismatch".to_owned())
}
#[tauri::command]
pub fn delete_recovery_job(app: tauri::AppHandle, record_id: String) -> Result<bool, String> {
    app.try_state::<Arc<JobStore>>()
        .ok_or_else(|| "secure_storage_unavailable".to_owned())?
        .delete(&record_id)
        .map_err(|_| "recovery_delete_failed".to_owned())
}
