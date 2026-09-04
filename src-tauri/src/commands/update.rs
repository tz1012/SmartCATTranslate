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
const PUBLIC_RELEASE_API: &str =
    "https://api.github.com/repos/tz1012/SmartCATTranslate/releases?per_page=100";
const PUBLIC_RELEASE_PATH_PREFIX: &str = "/tz1012/SmartCATTranslate/releases/tag/app-v";
const SIGNED_RELEASE_METADATA_URL: &str =
    "https://github.com/tz1012/SmartCATTranslate/releases/latest/download/latest.json";

#[derive(Default)]
pub struct UpdateState {
    checks: Mutex<HashMap<String, CheckedUpdate>>,
    prepared: Mutex<HashMap<String, PreparedUpdate>>,
    restart_consents: Mutex<HashMap<String, RestartConsent>>,
    installing: AtomicBool,
    healthy_marked: AtomicBool,
}

#[derive(Debug)]
struct InstallGuard<'a>(&'a AtomicBool);

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn acquire_install_guard(installing: &AtomicBool) -> Result<InstallGuard<'_>, String> {
    installing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| InstallGuard(installing))
        .map_err(|_| "update_install_in_progress".to_owned())
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
    manual_only: bool,
    release_url: Option<String>,
}

#[derive(Deserialize)]
struct PublicRelease {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
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
    if require_configured().is_err() {
        return check_public_release(&app).await;
    }
    let update = match app
        .updater()
        .map_err(|_| "updater_not_configured".to_owned())?
        .check()
        .await
    {
        Ok(update) => update,
        Err(tauri_plugin_updater::Error::ReleaseNotFound)
            if signed_feed_is_missing_or_unreachable().await =>
        {
            return check_public_release(&app).await;
        }
        Err(error) if should_fallback_to_public_release(&error) => {
            return check_public_release(&app).await;
        }
        Err(error) => return Err(map_check_error(error)),
    };
    let Some(update) = update else {
        return Ok(UpdateCheckResult {
            available: false,
            version: None,
            release_notes: None,
            published_at: None,
            size_bytes: None,
            consent_token: None,
            manual_only: false,
            release_url: None,
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
        manual_only: false,
        release_url: None,
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
    let _install_guard = acquire_install_guard(&state.installing)?;
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

fn should_fallback_to_public_release(error: &tauri_plugin_updater::Error) -> bool {
    use tauri_plugin_updater::Error;
    match error {
        Error::Reqwest(error) => error.is_connect() || error.is_timeout(),
        Error::Network(_) => true,
        _ => false,
    }
}

async fn signed_feed_is_missing_or_unreachable() -> bool {
    signed_feed_is_missing_or_unreachable_at(SIGNED_RELEASE_METADATA_URL).await
}

async fn signed_feed_is_missing_or_unreachable_at(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(3))
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return true;
    };
    match client
        .get(url)
        .header(reqwest::header::USER_AGENT, "SmartCAT-Translate")
        .send()
        .await
    {
        Ok(response) => signed_feed_status_allows_public_fallback(response.status().as_u16()),
        Err(error) => error.is_connect() || error.is_timeout(),
    }
}

fn signed_feed_status_allows_public_fallback(status: u16) -> bool {
    status == 404
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

async fn check_public_release(app: &AppHandle) -> Result<UpdateCheckResult, String> {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "update_network_error".to_owned())?
        .get(PUBLIC_RELEASE_API)
        .header(reqwest::header::USER_AGENT, "SmartCAT-Translate")
        .send()
        .await
        .map_err(|_| "update_network_error".to_owned())?;
    if response.status().as_u16() == 404 {
        return Ok(no_public_update());
    }
    if !response.status().is_success() {
        return Err("update_network_error".to_owned());
    }
    let releases: Vec<PublicRelease> = response
        .json()
        .await
        .map_err(|_| "update_network_error".to_owned())?;
    let Some((target, release, release_url)) =
        select_public_release(releases, &app.package_info().version)
    else {
        return Ok(no_public_update());
    };
    Ok(UpdateCheckResult {
        available: true,
        version: Some(target.to_string()),
        release_notes: release.body,
        published_at: release.published_at,
        size_bytes: None,
        consent_token: None,
        manual_only: true,
        release_url: Some(release_url),
    })
}

fn select_public_release(
    releases: Vec<PublicRelease>,
    current: &semver::Version,
) -> Option<(semver::Version, PublicRelease, String)> {
    releases
        .into_iter()
        .filter_map(|release| {
            if release.draft || release.prerelease {
                return None;
            }
            let version = semver::Version::parse(release.tag_name.strip_prefix("app-v")?).ok()?;
            if &version <= current {
                return None;
            }
            let url =
                format!("https://github.com/tz1012/SmartCATTranslate/releases/tag/app-v{version}");
            Some((version, release, url))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
}

fn no_public_update() -> UpdateCheckResult {
    UpdateCheckResult {
        available: false,
        version: None,
        release_notes: None,
        published_at: None,
        size_bytes: None,
        consent_token: None,
        manual_only: true,
        release_url: None,
    }
}

fn is_allowed_release_url(candidate: &str) -> bool {
    let Ok(url) = url::Url::parse(candidate) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .path()
            .strip_prefix(PUBLIC_RELEASE_PATH_PREFIX)
            .is_some_and(|version| semver::Version::parse(version).is_ok())
}

#[tauri::command]
pub fn open_update_release(app: AppHandle, url: String) -> Result<(), String> {
    if !is_allowed_release_url(&url) {
        return Err("update_release_url_invalid".to_owned());
    }
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|_| "update_release_unavailable".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_install_guard, is_allowed_release_url, select_public_release,
        should_fallback_to_public_release, signed_feed_status_allows_public_fallback,
        PublicRelease,
    };
    use std::sync::atomic::AtomicBool;

    fn redirecting_feed(final_status: u16, reason: &'static str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().take(2).enumerate() {
                let mut stream = stream.unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                let response = if index == 0 {
                    "HTTP/1.1 302 Found\r\nLocation: /tagged/latest.json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                } else {
                    format!(
                        "HTTP/1.1 {final_status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        format!("http://{address}/latest.json")
    }

    fn looping_feed() -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(5) {
                let mut stream = stream.unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                stream
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        });
        format!("http://{address}/latest.json")
    }

    #[test]
    fn only_one_installer_can_run_and_the_guard_releases_afterward() {
        let installing = AtomicBool::new(false);
        let first =
            acquire_install_guard(&installing).expect("first installer should acquire guard");
        assert_eq!(
            acquire_install_guard(&installing).unwrap_err(),
            "update_install_in_progress"
        );
        drop(first);
        assert!(acquire_install_guard(&installing).is_ok());
    }

    #[test]
    fn manual_release_url_is_restricted_to_the_public_repository() {
        assert!(is_allowed_release_url(
            "https://github.com/tz1012/SmartCATTranslate/releases/tag/app-v0.1.4"
        ));
        assert!(!is_allowed_release_url(
            "https://github.com/tz1012/SmartCATTranslate.evil.example/releases/tag/app-v0.1.4"
        ));
        assert!(!is_allowed_release_url(
            "https://user@github.com/tz1012/SmartCATTranslate/releases/tag/app-v0.1.4"
        ));
    }

    #[test]
    fn only_signed_feed_transport_failures_fall_back_directly() {
        use tauri_plugin_updater::Error;

        assert!(should_fallback_to_public_release(&Error::Network(
            "signed feed unavailable".to_owned()
        )));
        assert!(!should_fallback_to_public_release(&Error::ReleaseNotFound));
        assert!(!should_fallback_to_public_release(
            &Error::AuthenticationFailed
        ));
        assert!(!should_fallback_to_public_release(&Error::SignatureUtf8(
            "invalid signature".to_owned()
        )));
        assert!(!should_fallback_to_public_release(&Error::Serialization(
            serde_json::from_str::<serde_json::Value>("{").unwrap_err()
        )));
        assert!(!should_fallback_to_public_release(&Error::UnsupportedArch));
    }

    #[test]
    fn only_confirmed_missing_signed_feed_status_falls_back() {
        assert!(signed_feed_status_allows_public_fallback(404));
        for status in [200, 302, 401, 403, 429, 500] {
            assert!(!signed_feed_status_allows_public_fallback(status));
        }
    }

    #[tokio::test]
    async fn signed_feed_probe_follows_redirect_before_classifying_status() {
        let missing = redirecting_feed(404, "Not Found");
        assert!(super::signed_feed_is_missing_or_unreachable_at(&missing).await);

        let forbidden = redirecting_feed(403, "Forbidden");
        assert!(!super::signed_feed_is_missing_or_unreachable_at(&forbidden).await);
    }

    #[tokio::test]
    async fn signed_feed_probe_does_not_treat_redirect_loop_as_unavailable() {
        assert!(!super::signed_feed_is_missing_or_unreachable_at(&looping_feed()).await);
    }

    #[tokio::test]
    async fn updater_redirect_error_does_not_fall_back_directly() {
        let error = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(1))
            .build()
            .unwrap()
            .get(looping_feed())
            .send()
            .await
            .unwrap_err();
        assert!(error.is_redirect());
        assert!(!should_fallback_to_public_release(
            &tauri_plugin_updater::Error::Reqwest(error)
        ));
    }

    #[test]
    fn latest_app_release_ignores_newer_unrelated_draft_and_prerelease_entries() {
        let releases = vec![
            release("runtime-v9.0.0", false, false),
            release("app-v0.1.7", true, false),
            release("app-v0.1.6", false, true),
            release("app-v0.1.5", false, false),
            release("app-v0.1.4", false, false),
        ];

        let (version, _, url) =
            select_public_release(releases, &semver::Version::parse("0.1.3").unwrap()).unwrap();

        assert_eq!(version, semver::Version::parse("0.1.5").unwrap());
        assert_eq!(
            url,
            "https://github.com/tz1012/SmartCATTranslate/releases/tag/app-v0.1.5"
        );
    }

    fn release(tag_name: &str, draft: bool, prerelease: bool) -> PublicRelease {
        PublicRelease {
            tag_name: tag_name.to_owned(),
            body: None,
            published_at: None,
            draft,
            prerelease,
        }
    }
}
