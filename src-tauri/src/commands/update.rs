use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;
use uuid::Uuid;

const CONSENT_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct UpdateState {
    checks: Mutex<HashMap<String, CheckedUpdate>>,
    prepared: Mutex<HashMap<String, PreparedUpdate>>,
    restart_consents: Mutex<HashMap<String, RestartConsent>>,
    healthy_marked: AtomicBool,
}

struct CheckedUpdate {
    version: String,
    expires_at: Instant,
    update: Update,
}

struct PreparedUpdate {
    version: String,
    expires_at: Instant,
    update: Update,
    bytes: Vec<u8>,
}

struct RestartConsent {
    version: String,
    install_token: String,
    expires_at: Instant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    available: bool,
    version: Option<String>,
    release_notes: Option<String>,
    published_at: Option<String>,
    size_bytes: Option<u64>,
    consent_token: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedUpdateResult {
    install_token: String,
    size_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartConsentResult {
    restart_consent_token: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    version: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingUpdate {
    from_version: String,
    target_version: String,
    previous_installer_url: String,
    target_installer_url: String,
    installed_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LastKnownGood {
    version: String,
    reached_main_window_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInstructions {
    previous_version: String,
    previous_installer_url: String,
    message: String,
}

fn require_configured() -> Result<(), String> {
    if option_env!("SMARTCAT_UPDATER_CONFIGURED") == Some("1")
        && option_env!("SMARTCAT_RELEASE_REPOSITORY").is_some()
    {
        Ok(())
    } else {
        Err("updater_not_configured".to_owned())
    }
}

#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateCheckResult, String> {
    require_configured()?;
    let update = app
        .updater()
        .map_err(|_| "updater_not_configured".to_owned())?
        .check()
        .await
        .map_err(map_check_error)?;
    let Some(update) = update else {
        return Ok(UpdateCheckResult {
            available: false,
            version: None,
            release_notes: None,
            published_at: None,
            size_bytes: None,
            consent_token: None,
        });
    };

    let size_bytes = update
        .raw_json
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            update
                .raw_json
                .get("platforms")
                .and_then(|platforms| platforms.get(&update.target))
                .and_then(|platform| platform.get("size"))
                .and_then(serde_json::Value::as_u64)
        });
    let token = Uuid::new_v4().to_string();
    let version = update.version.clone();
    let result = UpdateCheckResult {
        available: true,
        version: Some(version.clone()),
        release_notes: update.body.clone(),
        published_at: update.date.map(|date| date.to_string()),
        size_bytes,
        consent_token: Some(token.clone()),
    };
    let mut checks = state.checks.lock().await;
    checks.retain(|_, checked| checked.expires_at > Instant::now());
    checks.insert(
        token,
        CheckedUpdate {
            version,
            expires_at: Instant::now() + CONSENT_TTL,
            update,
        },
    );
    Ok(result)
}

#[tauri::command]
pub async fn prepare_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
    version: String,
    consent_token: String,
) -> Result<PreparedUpdateResult, String> {
    require_configured()?;
    let checked = state
        .checks
        .lock()
        .await
        .remove(&consent_token)
        .ok_or_else(|| "update_consent_invalid".to_owned())?;
    if checked.expires_at <= Instant::now() {
        return Err("update_consent_expired".to_owned());
    }
    if checked.version != version {
        return Err("update_version_mismatch".to_owned());
    }

    let progress_app = app.clone();
    let progress_version = version.clone();
    let mut downloaded = 0_u64;
    let bytes = checked
        .update
        .download(
            move |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ = progress_app.emit(
                    "update-progress",
                    UpdateProgress {
                        version: progress_version.clone(),
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(map_download_error)?;
    let size_bytes = bytes.len() as u64;
    let install_token = Uuid::new_v4().to_string();
    let mut prepared = state.prepared.lock().await;
    prepared.retain(|_, update| update.expires_at > Instant::now());
    prepared.insert(
        install_token.clone(),
        PreparedUpdate {
            version,
            expires_at: Instant::now() + CONSENT_TTL,
            update: checked.update,
            bytes,
        },
    );
    Ok(PreparedUpdateResult {
        install_token,
        size_bytes,
    })
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
    version: String,
    install_token: String,
    restart_consent_token: String,
) -> Result<(), String> {
    require_configured()?;
    let restart_consent = state
        .restart_consents
        .lock()
        .await
        .remove(&restart_consent_token)
        .ok_or_else(|| "update_restart_consent_invalid".to_owned())?;
    if restart_consent.expires_at <= Instant::now() {
        return Err("update_restart_consent_expired".to_owned());
    }
    if restart_consent.version != version || restart_consent.install_token != install_token {
        return Err("update_restart_consent_mismatch".to_owned());
    }
    let prepared = state
        .prepared
        .lock()
        .await
        .remove(&install_token)
        .ok_or_else(|| "update_consent_invalid".to_owned())?;
    if prepared.expires_at <= Instant::now() {
        return Err("update_consent_expired".to_owned());
    }
    if prepared.version != version {
        return Err("update_version_mismatch".to_owned());
    }
    write_pending_update(&app, &version)?;
    prepared
        .update
        .install(&prepared.bytes)
        .map_err(map_install_error)?;
    std::thread::spawn(move || app.restart());
    Ok(())
}

#[tauri::command]
pub async fn authorize_update_restart(
    state: State<'_, UpdateState>,
    version: String,
    install_token: String,
) -> Result<RestartConsentResult, String> {
    require_configured()?;
    let prepared = state.prepared.lock().await;
    let update = prepared
        .get(&install_token)
        .ok_or_else(|| "update_consent_invalid".to_owned())?;
    if update.expires_at <= Instant::now() || update.version != version {
        return Err("update_consent_invalid".to_owned());
    }
    drop(prepared);
    let token = Uuid::new_v4().to_string();
    let mut consents = state.restart_consents.lock().await;
    consents.retain(|_, consent| consent.expires_at > Instant::now());
    consents.insert(
        token.clone(),
        RestartConsent {
            version,
            install_token,
            expires_at: Instant::now() + Duration::from_secs(2 * 60),
        },
    );
    Ok(RestartConsentResult {
        restart_consent_token: token,
    })
}

#[tauri::command]
pub fn get_update_recovery_instructions(
    app: AppHandle,
) -> Result<Option<RecoveryInstructions>, String> {
    let root = update_state_dir(&app)?;
    let path = root.join("pending-update.json");
    let record: PendingUpdate = match std::fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "rollback_record_invalid".to_owned())?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("rollback_record_unavailable".to_owned()),
    };
    let from = semver::Version::parse(&record.from_version)
        .map_err(|_| "rollback_record_invalid".to_owned())?;
    let target = semver::Version::parse(&record.target_version)
        .map_err(|_| "rollback_record_invalid".to_owned())?;
    let current = app.package_info().version.clone();
    if target <= from || (current != from && current != target) {
        clear_pending_records(&root)?;
        return Ok(None);
    }
    if current == target {
        return Ok(None);
    }
    Ok(Some(RecoveryInstructions {
        previous_version: record.from_version,
        previous_installer_url: record.previous_installer_url,
        message: "새 버전이 시작되지 않으면 이전 설치 관리자를 직접 다운로드해 실행하세요. 앱은 자동으로 롤백하지 않습니다.".to_owned(),
    }))
}

#[tauri::command]
pub fn open_previous_installer(app: AppHandle) -> Result<(), String> {
    let path = update_state_dir(&app)?.join("pending-update.json");
    let bytes = std::fs::read(path).map_err(|_| "rollback_record_unavailable".to_owned())?;
    let record: PendingUpdate =
        serde_json::from_slice(&bytes).map_err(|_| "rollback_record_invalid".to_owned())?;
    if !record
        .previous_installer_url
        .starts_with("https://github.com/")
    {
        return Err("rollback_url_invalid".to_owned());
    }
    app.opener()
        .open_url(record.previous_installer_url, None::<String>)
        .map_err(|_| "rollback_installer_unavailable".to_owned())
}

#[tauri::command]
pub fn mark_app_healthy(app: AppHandle, state: State<'_, UpdateState>) -> Result<bool, String> {
    if state.healthy_marked.load(Ordering::Acquire) {
        return Ok(false);
    }
    let root = update_state_dir(&app)?;
    let pending_path = root.join("pending-update.json");
    let pending: PendingUpdate = match std::fs::read(&pending_path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "rollback_record_invalid".to_owned())?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            state.healthy_marked.store(true, Ordering::Release);
            return Ok(false);
        }
        Err(_) => return Err("rollback_record_unavailable".to_owned()),
    };
    let target = semver::Version::parse(&pending.target_version)
        .map_err(|_| "rollback_record_invalid".to_owned())?;
    if app.package_info().version != target {
        state.healthy_marked.store(true, Ordering::Release);
        return Ok(false);
    }
    std::fs::create_dir_all(&root).map_err(|_| "update_state_unavailable".to_owned())?;
    let record = LastKnownGood {
        version: app.package_info().version.to_string(),
        reached_main_window_at: chrono::Utc::now().to_rfc3339(),
    };
    write_private_json(root.join("last-known-good.json"), &record)?;
    clear_pending_records(&root)?;
    state.healthy_marked.store(true, Ordering::Release);
    Ok(true)
}

fn write_pending_update(app: &AppHandle, target_version: &str) -> Result<(), String> {
    let repository = option_env!("SMARTCAT_RELEASE_REPOSITORY")
        .ok_or_else(|| "updater_not_configured".to_owned())?;
    let from_version = app.package_info().version.to_string();
    let record = PendingUpdate {
        previous_installer_url: format!(
            "https://github.com/{repository}/releases/tag/app-v{from_version}"
        ),
        target_installer_url: format!(
            "https://github.com/{repository}/releases/tag/app-v{target_version}"
        ),
        from_version,
        target_version: target_version.to_owned(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    let root = update_state_dir(app)?;
    std::fs::create_dir_all(&root).map_err(|_| "update_state_unavailable".to_owned())?;
    write_private_json(root.join("pending-update.json"), &record)?;
    write_private_json(root.join("rollback.json"), &record)
}

fn clear_pending_records(root: &std::path::Path) -> Result<(), String> {
    // Pending is the recovery authority and is removed last so a partial cleanup remains retryable.
    for name in ["rollback.json", "pending-update.json"] {
        match std::fs::remove_file(root.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("update_state_unavailable".to_owned()),
        }
    }
    Ok(())
}

fn update_state_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|root| root.join("updates"))
        .map_err(|_| "update_state_unavailable".to_owned())
}

fn write_private_json(path: PathBuf, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "update_state_invalid".to_owned())?;
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "update_state_unavailable".to_owned())?;
    use std::io::Write;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "update_state_unavailable".to_owned())?;
    if atomic_replace(&temporary, &path).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err("update_state_unavailable".to_owned());
    }
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn atomic_replace(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

fn map_check_error(error: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match error {
        Error::Minisign(_) | Error::SignatureUtf8(_) | Error::AuthenticationFailed => {
            "update_signature_invalid".to_owned()
        }
        Error::EmptyEndpoints | Error::InsecureTransportProtocol => {
            "updater_not_configured".to_owned()
        }
        _ => "update_network_error".to_owned(),
    }
}

fn map_download_error(error: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match error {
        Error::Minisign(_) | Error::SignatureUtf8(_) | Error::AuthenticationFailed => {
            "update_signature_invalid".to_owned()
        }
        _ => "update_network_error".to_owned(),
    }
}

fn map_install_error(_: tauri_plugin_updater::Error) -> String {
    "update_install_failed".to_owned()
}
