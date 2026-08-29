use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use url::Url;

use crate::codex::transport::{AppServerTransport, TransportError};

#[derive(Clone, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AccountState {
    SignedOut,
    SignedIn {
        email_hint: Option<String>,
        plan: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub account: AccountState,
    pub login_pending: bool,
}

impl fmt::Debug for AccountState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedOut => formatter.write_str("SignedOut"),
            Self::SignedIn { .. } => formatter
                .debug_struct("SignedIn")
                .field("email_hint", &"<redacted>")
                .field("plan", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitState {
    pub primary_used_percent: Option<f64>,
    pub primary_resets_at: Option<i64>,
    pub secondary_used_percent: Option<f64>,
    pub secondary_resets_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountChangeReason {
    LoginSucceeded,
    LoginFailed,
    LoginCancelled,
    AccountUpdated,
}

pub trait AccountEventSink: Send + Sync + 'static {
    fn account_state_changed(&self, reason: AccountChangeReason);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserOpenError;

pub trait BrowserOpener: Send + Sync {
    fn open(&self, url: &Url) -> Result<(), BrowserOpenError>;
}

pub struct LoginStart {
    auth_url: Url,
}

impl fmt::Debug for LoginStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginStart")
            .field("auth_url", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("the Codex App Server account request failed")]
    Transport(#[from] TransportError),
    #[error("the account response did not contain a managed ChatGPT login")]
    UnexpectedLoginResponse,
    #[error("the browser login URL is not an approved ChatGPT URL")]
    UnsafeAuthUrl,
    #[error("a managed ChatGPT login is already pending")]
    LoginAlreadyPending,
    #[error("an invalid managed login response could not be cancelled and must be retried")]
    LoginCleanupPending,
    #[error("the system browser could not be opened")]
    BrowserOpenFailed,
    #[error("the system browser could not be opened and login cancellation must be retried")]
    BrowserOpenFailedLoginPending,
}

struct PendingLoginId(String);

impl fmt::Debug for PendingLoginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PendingLoginId(<redacted>)")
    }
}

pub struct AccountService {
    transport: Arc<dyn AppServerTransport>,
    event_sink: Arc<dyn AccountEventSink>,
    pending_login: Arc<Mutex<Option<PendingLoginId>>>,
    notification_task: JoinHandle<()>,
}

impl AccountService {
    pub fn new(
        transport: Arc<dyn AppServerTransport>,
        event_sink: Arc<dyn AccountEventSink>,
    ) -> Self {
        let pending_login = Arc::new(Mutex::new(None));
        let notifications = transport.subscribe();
        let notification_task = tokio::spawn(monitor_notifications(
            notifications,
            pending_login.clone(),
            event_sink.clone(),
        ));
        Self {
            transport,
            event_sink,
            pending_login,
            notification_task,
        }
    }

    pub async fn read(&self) -> Result<AccountState, AuthError> {
        self.read_with_refresh(false).await
    }

    pub async fn read_snapshot(&self) -> Result<AccountSnapshot, AuthError> {
        let account = self.read().await?;
        let login_pending = self.pending_login.lock().await.is_some();
        Ok(AccountSnapshot {
            account,
            login_pending,
        })
    }

    pub async fn read_with_refresh(&self, refresh_token: bool) -> Result<AccountState, AuthError> {
        let value = self
            .transport
            .request("account/read", json!({ "refreshToken": refresh_token }))
            .await?;
        Ok(map_account_state(&value))
    }

    pub async fn read_rate_limits(&self) -> Result<RateLimitState, AuthError> {
        let value = self
            .transport
            .request("account/rateLimits/read", json!({}))
            .await?;
        Ok(map_rate_limits(&value))
    }

    pub async fn start_chatgpt_login(&self) -> Result<LoginStart, AuthError> {
        // Holding this lock across the request orders an immediately arriving completion
        // notification after the returned login identifier has been stored.
        let mut pending = self.pending_login.lock().await;
        if pending.is_some() {
            return Err(AuthError::LoginAlreadyPending);
        }
        let value = self
            .transport
            .request(
                "account/login/start",
                json!({
                    "type": "chatgpt",
                    "useHostedLoginSuccessPage": true,
                    "appBrand": "chatgpt"
                }),
            )
            .await?;
        let login_id = value
            .get("loginId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(AuthError::UnexpectedLoginResponse)?
            .to_owned();
        *pending = Some(PendingLoginId(login_id.clone()));
        let parsed_start = (|| {
            if value.get("type").and_then(Value::as_str) != Some("chatgpt") {
                return Err(AuthError::UnexpectedLoginResponse);
            }
            let auth_url = value
                .get("authUrl")
                .and_then(Value::as_str)
                .ok_or(AuthError::UnexpectedLoginResponse)?;
            let auth_url = Url::parse(auth_url).map_err(|_| AuthError::UnsafeAuthUrl)?;
            validate_browser_login_url(&auth_url)?;
            Ok(auth_url)
        })();
        let auth_url = match parsed_start {
            Ok(url) => url,
            Err(error) => {
                let cancelled = self
                    .transport
                    .request("account/login/cancel", json!({ "loginId": login_id }))
                    .await
                    .is_ok();
                if cancelled {
                    *pending = None;
                    return Err(error);
                }
                return Err(AuthError::LoginCleanupPending);
            }
        };
        Ok(LoginStart { auth_url })
    }

    pub async fn cancel_login(&self) -> Result<bool, AuthError> {
        self.cancel_login_with_event(true).await
    }

    async fn cancel_login_after_opener_failure(&self) -> Result<bool, AuthError> {
        self.cancel_login_with_event(false).await
    }

    async fn cancel_login_with_event(&self, emit_change: bool) -> Result<bool, AuthError> {
        let mut pending = self.pending_login.lock().await;
        let Some(login) = pending.as_ref() else {
            return Ok(false);
        };
        self.transport
            .request(
                "account/login/cancel",
                json!({ "loginId": login.0.as_str() }),
            )
            .await?;
        *pending = None;
        drop(pending);
        if emit_change {
            self.event_sink
                .account_state_changed(AccountChangeReason::LoginCancelled);
        }
        Ok(true)
    }

    pub async fn has_pending_login(&self) -> bool {
        self.pending_login.lock().await.is_some()
    }
}

impl Drop for AccountService {
    fn drop(&mut self) {
        self.notification_task.abort();
    }
}

pub async fn start_login_and_open(
    service: &AccountService,
    opener: &dyn BrowserOpener,
) -> Result<(), AuthError> {
    let start = service.start_chatgpt_login().await?;
    validate_browser_login_url(&start.auth_url)?;
    if opener.open(&start.auth_url).is_err() {
        return match service.cancel_login_after_opener_failure().await {
            Ok(_) => Err(AuthError::BrowserOpenFailed),
            Err(_) => Err(AuthError::BrowserOpenFailedLoginPending),
        };
    }
    Ok(())
}

/// The documented hosted browser flow returns a `chatgpt.com` URL. SmartCAT
/// accepts that registrable domain and its DNS subdomains, but rejects every
/// other scheme/host, URL credentials, and nonstandard port before OS handoff.
pub fn validate_browser_login_url(url: &Url) -> Result<(), AuthError> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(AuthError::UnsafeAuthUrl);
    }
    let host = url.host_str().ok_or(AuthError::UnsafeAuthUrl)?;
    if host == "chatgpt.com" || host.ends_with(".chatgpt.com") {
        Ok(())
    } else {
        Err(AuthError::UnsafeAuthUrl)
    }
}

async fn monitor_notifications(
    mut notifications: tokio::sync::broadcast::Receiver<
        crate::codex::protocol::AppServerNotification,
    >,
    pending_login: Arc<Mutex<Option<PendingLoginId>>>,
    event_sink: Arc<dyn AccountEventSink>,
) {
    loop {
        let notification = match notifications.recv().await {
            Ok(notification) => notification,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        match notification.method.as_str() {
            "account/login/completed" => {
                let Some(completed_id) = notification.params.get("loginId").and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(success) = notification.params.get("success").and_then(Value::as_bool)
                else {
                    continue;
                };
                let matched = {
                    let mut pending = pending_login.lock().await;
                    if pending
                        .as_ref()
                        .is_some_and(|pending| pending.0 == completed_id)
                    {
                        *pending = None;
                        true
                    } else {
                        false
                    }
                };
                if matched {
                    event_sink.account_state_changed(if success {
                        AccountChangeReason::LoginSucceeded
                    } else {
                        AccountChangeReason::LoginFailed
                    });
                }
            }
            "account/updated" => {
                event_sink.account_state_changed(AccountChangeReason::AccountUpdated);
            }
            _ => {}
        }
    }
}

fn map_account_state(value: &Value) -> AccountState {
    let Some(account) = value.get("account").and_then(Value::as_object) else {
        return AccountState::SignedOut;
    };
    if account.get("type").and_then(Value::as_str) != Some("chatgpt") {
        return AccountState::SignedOut;
    }
    AccountState::SignedIn {
        email_hint: account
            .get("email")
            .and_then(Value::as_str)
            .and_then(mask_email),
        plan: account
            .get("planType")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    }
}

fn mask_email(email: &str) -> Option<String> {
    let (local, domain) = email.split_once('@')?;
    let first = local.chars().next()?;
    if domain.is_empty() {
        return None;
    }
    Some(format!("{first}***@{domain}"))
}

fn map_rate_limits(value: &Value) -> RateLimitState {
    let limits = value.get("rateLimits").unwrap_or(&Value::Null);
    let primary = limits.get("primary").unwrap_or(&Value::Null);
    let secondary = limits.get("secondary").unwrap_or(&Value::Null);
    RateLimitState {
        primary_used_percent: map_percent(primary.get("usedPercent")),
        primary_resets_at: map_timestamp(primary.get("resetsAt")),
        secondary_used_percent: map_percent(secondary.get("usedPercent")),
        secondary_resets_at: map_timestamp(secondary.get("resetsAt")),
    }
}

fn map_percent(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
}

fn map_timestamp(value: Option<&Value>) -> Option<i64> {
    const MAX_JAVASCRIPT_DATE_UNIX_SECONDS: i64 = 8_640_000_000_000;
    value
        .and_then(Value::as_i64)
        .filter(|value| (0..=MAX_JAVASCRIPT_DATE_UNIX_SECONDS).contains(value))
}
