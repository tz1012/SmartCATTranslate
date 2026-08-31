use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_opener::OpenerExt;

use crate::app_state::AppState;
use crate::codex::auth::{
    start_login_and_open, AccountChangeReason, AccountEventSink, AccountSnapshot, BrowserOpenError,
    BrowserOpener, RateLimitState,
};

const SERVICE_UNAVAILABLE: &str = "account_service_unavailable";

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StartLoginResult {
    BrowserOpened,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CancelLoginResult {
    Cancelled,
    NotPending,
}

#[tauri::command]
pub async fn get_account(state: State<'_, AppState>) -> Result<AccountSnapshot, String> {
    let service = state.account_service().await.ok_or(SERVICE_UNAVAILABLE)?;
    service
        .read_snapshot()
        .await
        .map_err(|error| error_code(&error))
}

#[tauri::command]
pub async fn get_rate_limits(state: State<'_, AppState>) -> Result<RateLimitState, String> {
    let service = state.account_service().await.ok_or(SERVICE_UNAVAILABLE)?;
    service
        .read_rate_limits()
        .await
        .map_err(|error| error_code(&error))
}

#[tauri::command]
pub async fn start_chatgpt_login<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<StartLoginResult, String> {
    let service = state.account_service().await.ok_or(SERVICE_UNAVAILABLE)?;
    start_login_and_open(service.as_ref(), &TauriBrowserOpener { app })
        .await
        .map_err(|error| error_code(&error))?;
    Ok(StartLoginResult::BrowserOpened)
}

#[tauri::command]
pub async fn cancel_chatgpt_login(state: State<'_, AppState>) -> Result<CancelLoginResult, String> {
    let service = state.account_service().await.ok_or(SERVICE_UNAVAILABLE)?;
    let cancelled = service
        .cancel_login()
        .await
        .map_err(|error| error_code(&error))?;
    Ok(if cancelled {
        CancelLoginResult::Cancelled
    } else {
        CancelLoginResult::NotPending
    })
}

struct TauriBrowserOpener<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> BrowserOpener for TauriBrowserOpener<R> {
    fn open(&self, url: &url::Url) -> Result<(), BrowserOpenError> {
        self.app
            .opener()
            .open_url(url.as_str(), None::<&str>)
            .map_err(|_| BrowserOpenError)
    }
}

pub struct TauriAccountEventSink<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> AccountEventSink for TauriAccountEventSink<R> {
    fn account_state_changed(&self, reason: AccountChangeReason) {
        let _ = self.0.emit(
            "account-state-changed",
            AccountStateChangedPayload { reason },
        );
    }
}

#[derive(Clone, Copy, Serialize)]
struct AccountStateChangedPayload {
    reason: AccountChangeReason,
}

fn error_code(error: &crate::codex::auth::AuthError) -> String {
    match error {
        crate::codex::auth::AuthError::Transport(_) => "account_transport_failed",
        crate::codex::auth::AuthError::UnexpectedLoginResponse => "invalid_login_response",
        crate::codex::auth::AuthError::UnsafeAuthUrl => "unsafe_auth_url",
        crate::codex::auth::AuthError::LoginAlreadyPending => "login_already_pending",
        crate::codex::auth::AuthError::LoginCleanupPending => "login_cleanup_pending",
        crate::codex::auth::AuthError::BrowserOpenFailed => "browser_open_failed",
        crate::codex::auth::AuthError::BrowserOpenFailedLoginPending => {
            "browser_open_failed_login_pending"
        }
    }
    .to_owned()
}
