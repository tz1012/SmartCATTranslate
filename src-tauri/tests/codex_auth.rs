use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use smartcat_translate::codex::auth::{
    start_login_and_open, validate_browser_login_url, AccountChangeReason, AccountEventSink,
    AccountService, AccountSnapshot, AccountState, AuthError, BrowserOpener, RateLimitState,
};
use smartcat_translate::codex::protocol::AppServerNotification;
use smartcat_translate::codex::transport::{AppServerTransport, TransportError};
use tokio::sync::broadcast;

struct FakeTransport {
    responses: Mutex<VecDeque<Result<Value, TransportError>>>,
    requests: Mutex<Vec<(String, Value)>>,
    notifications: broadcast::Sender<AppServerNotification>,
}

struct ImmediateCompletionTransport {
    notifications: broadcast::Sender<AppServerNotification>,
}

impl ImmediateCompletionTransport {
    fn new() -> Arc<Self> {
        let (notifications, _) = broadcast::channel(4);
        Arc::new(Self { notifications })
    }
}

#[async_trait]
impl AppServerTransport for ImmediateCompletionTransport {
    async fn request(&self, method: &str, _params: Value) -> Result<Value, TransportError> {
        assert_eq!(method, "account/login/start");
        let _ = self.notifications.send(AppServerNotification {
            method: "account/login/completed".to_owned(),
            params: json!({ "loginId": "RACING-ID", "success": true, "error": null }),
            server_request: false,
        });
        tokio::task::yield_now().await;
        Ok(login_response(
            "RACING-ID",
            "https://chatgpt.com/auth/start",
        ))
    }

    fn subscribe(&self) -> broadcast::Receiver<AppServerNotification> {
        self.notifications.subscribe()
    }
}

impl FakeTransport {
    fn new(responses: Vec<Result<Value, TransportError>>) -> Arc<Self> {
        let (notifications, _) = broadcast::channel(16);
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
            notifications,
        })
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self.notifications.send(AppServerNotification {
            method: method.to_owned(),
            params,
            server_request: false,
        });
    }

    fn request_snapshot(&self) -> Vec<(String, Value)> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl AppServerTransport for FakeTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError> {
        self.requests
            .lock()
            .unwrap()
            .push((method.to_owned(), params));
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("test response is present")
    }

    fn subscribe(&self) -> broadcast::Receiver<AppServerNotification> {
        self.notifications.subscribe()
    }
}

#[derive(Default)]
struct FakeSink(Mutex<Vec<AccountChangeReason>>);

impl AccountEventSink for FakeSink {
    fn account_state_changed(&self, reason: AccountChangeReason) {
        self.0.lock().unwrap().push(reason);
    }
}

impl FakeSink {
    fn snapshot(&self) -> Vec<AccountChangeReason> {
        self.0.lock().unwrap().clone()
    }
}

struct FakeOpener {
    fail: bool,
    opened: Mutex<usize>,
}

impl BrowserOpener for FakeOpener {
    fn open(&self, _url: &url::Url) -> Result<(), ()> {
        *self.opened.lock().unwrap() += 1;
        if self.fail {
            Err(())
        } else {
            Ok(())
        }
    }
}

fn service(transport: Arc<FakeTransport>, sink: Arc<FakeSink>) -> AccountService {
    AccountService::new(transport, sink)
}

fn login_response(id: &str, url: &str) -> Value {
    json!({ "type": "chatgpt", "loginId": id, "authUrl": url })
}

async fn yield_until(mut predicate: impl FnMut() -> bool) {
    for _ in 0..64 {
        if predicate() {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert!(predicate(), "condition did not become true");
}

#[tokio::test]
async fn account_read_maps_only_managed_chatgpt_as_connected() {
    let transport = FakeTransport::new(vec![
        Ok(json!({
            "account": { "type": "chatgpt", "email": "person@example.com", "planType": "pro", "unknown": "ignored" },
            "requiresOpenaiAuth": true
        })),
        Ok(json!({ "account": { "type": "apiKey" }, "requiresOpenaiAuth": true })),
        Ok(json!({ "account": { "type": "futureMode" }, "requiresOpenaiAuth": true })),
    ]);
    let service = service(transport.clone(), Arc::new(FakeSink::default()));

    assert_eq!(
        service.read().await.unwrap(),
        AccountState::SignedIn {
            email_hint: Some("p***@example.com".to_owned()),
            plan: Some("pro".to_owned()),
        }
    );
    assert_eq!(service.read().await.unwrap(), AccountState::SignedOut);
    assert_eq!(service.read().await.unwrap(), AccountState::SignedOut);
    assert_eq!(
        transport.request_snapshot(),
        vec![
            ("account/read".to_owned(), json!({ "refreshToken": false })),
            ("account/read".to_owned(), json!({ "refreshToken": false })),
            ("account/read".to_owned(), json!({ "refreshToken": false })),
        ]
    );
}

#[test]
fn account_state_serializes_the_frontend_field_names() {
    let value = serde_json::to_value(AccountState::SignedIn {
        email_hint: Some("p***@example.com".to_owned()),
        plan: Some("plus".to_owned()),
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "state": "signedIn",
            "emailHint": "p***@example.com",
            "plan": "plus"
        })
    );
}

#[test]
fn account_snapshot_serializes_authoritative_pending_state_without_an_identifier() {
    let value = serde_json::to_value(AccountSnapshot {
        account: AccountState::SignedOut,
        login_pending: true,
    })
    .unwrap();

    assert_eq!(
        value,
        json!({ "account": { "state": "signedOut" }, "loginPending": true })
    );
    assert!(!format!("{value:?}").contains("loginId"));
}

#[tokio::test]
async fn account_snapshot_reports_a_server_pending_login_after_remount() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com/auth/start",
        )),
        Ok(json!({ "account": null, "requiresOpenaiAuth": true })),
    ]);
    let service = service(transport, Arc::new(FakeSink::default()));
    service.start_chatgpt_login().await.unwrap();

    assert_eq!(
        service.read_snapshot().await.unwrap(),
        AccountSnapshot {
            account: AccountState::SignedOut,
            login_pending: true,
        }
    );
}

#[test]
fn account_state_debug_never_exposes_email_or_untrusted_plan_text() {
    let state = AccountState::SignedIn {
        email_hint: Some("private@example.com".to_owned()),
        plan: Some("Bearer TOKEN-SECRET".to_owned()),
    };

    let rendered = format!("{state:?}");

    assert!(!rendered.contains("private@example.com"));
    assert!(!rendered.contains("TOKEN-SECRET"));
    assert!(rendered.contains("<redacted>"));
}

#[tokio::test]
async fn rate_limits_map_usage_and_unix_seconds_without_inventing_secondary_capacity() {
    let transport = FakeTransport::new(vec![Ok(json!({
        "rateLimits": {
            "primary": { "usedPercent": 25.5, "resetsAt": 1730947200, "windowDurationMins": 15 },
            "secondary": null,
            "credits": 999,
            "futureField": true
        }
    }))]);
    let service = service(transport.clone(), Arc::new(FakeSink::default()));

    assert_eq!(
        service.read_rate_limits().await.unwrap(),
        RateLimitState {
            primary_used_percent: Some(25.5),
            primary_resets_at: Some(1_730_947_200),
            secondary_used_percent: None,
            secondary_resets_at: None,
        }
    );
    assert_eq!(
        transport.request_snapshot(),
        vec![("account/rateLimits/read".to_owned(), json!({}))]
    );
}

#[tokio::test]
async fn rate_limit_timestamps_are_bounded_to_the_javascript_date_range() {
    let transport = FakeTransport::new(vec![Ok(json!({
        "rateLimits": {
            "primary": { "usedPercent": 1, "resetsAt": 8_640_000_000_000_i64 },
            "secondary": { "usedPercent": 2, "resetsAt": 8_640_000_000_001_i64 }
        }
    }))]);
    let service = service(transport, Arc::new(FakeSink::default()));

    assert_eq!(
        service.read_rate_limits().await.unwrap(),
        RateLimitState {
            primary_used_percent: Some(1.0),
            primary_resets_at: Some(8_640_000_000_000),
            secondary_used_percent: Some(2.0),
            secondary_resets_at: None,
        }
    );
}

#[tokio::test]
async fn missing_and_malformed_rate_limit_fields_are_reported_as_unknown() {
    let transport = FakeTransport::new(vec![Ok(json!({
        "rateLimits": {
            "primary": { "usedPercent": "25", "resetsAt": -1 },
            "secondary": { "usedPercent": 150, "resetsAt": 1.5 }
        }
    }))]);
    let service = service(transport, Arc::new(FakeSink::default()));

    assert_eq!(
        service.read_rate_limits().await.unwrap(),
        RateLimitState {
            primary_used_percent: None,
            primary_resets_at: None,
            secondary_used_percent: None,
            secondary_resets_at: None,
        }
    );
}

#[tokio::test]
async fn login_uses_the_documented_browser_flow_and_cancel_clears_only_after_success() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com/auth/start?opaque=TOKEN-SECRET",
        )),
        Ok(json!({})),
    ]);
    let service = service(transport.clone(), Arc::new(FakeSink::default()));

    let start = service.start_chatgpt_login().await.unwrap();
    assert!(service.has_pending_login().await);
    assert!(!format!("{start:?}").contains("LOGIN-SECRET"));
    assert!(!format!("{start:?}").contains("TOKEN-SECRET"));
    assert!(service.cancel_login().await.unwrap());
    assert!(!service.has_pending_login().await);

    assert_eq!(
        transport.request_snapshot(),
        vec![
            (
                "account/login/start".to_owned(),
                json!({ "type": "chatgpt", "useHostedLoginSuccessPage": true, "appBrand": "chatgpt" }),
            ),
            (
                "account/login/cancel".to_owned(),
                json!({ "loginId": "LOGIN-SECRET" })
            ),
        ]
    );
}

#[tokio::test]
async fn cancel_failure_retains_the_pending_login_for_retry() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com/auth/start",
        )),
        Err(TransportError::Remote { code: -32_000 }),
    ]);
    let service = service(transport, Arc::new(FakeSink::default()));
    service.start_chatgpt_login().await.unwrap();

    assert!(matches!(
        service.cancel_login().await,
        Err(AuthError::Transport(_))
    ));
    assert!(service.has_pending_login().await);
}

#[tokio::test]
async fn successful_cancel_emits_only_a_sanitized_pending_state_change() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com/auth/start?opaque=TOKEN-SECRET",
        )),
        Ok(json!({})),
    ]);
    let sink = Arc::new(FakeSink::default());
    let service = service(transport, sink.clone());
    service.start_chatgpt_login().await.unwrap();

    assert!(service.cancel_login().await.unwrap());

    assert_eq!(sink.snapshot(), vec![AccountChangeReason::LoginCancelled]);
    let rendered = format!("{:?}", sink.snapshot());
    assert!(!rendered.contains("LOGIN-SECRET"));
    assert!(!rendered.contains("TOKEN-SECRET"));
}

#[tokio::test]
async fn a_second_start_is_rejected_without_replacing_the_pending_login() {
    let transport = FakeTransport::new(vec![Ok(login_response(
        "FIRST-ID",
        "https://chatgpt.com/auth/start",
    ))]);
    let service = service(transport.clone(), Arc::new(FakeSink::default()));
    service.start_chatgpt_login().await.unwrap();

    assert_eq!(
        service.start_chatgpt_login().await.unwrap_err(),
        AuthError::LoginAlreadyPending
    );
    assert_eq!(transport.request_snapshot().len(), 1);
    assert!(service.has_pending_login().await);
}

#[tokio::test]
async fn completion_arriving_before_the_start_response_is_not_lost() {
    let transport = ImmediateCompletionTransport::new();
    let sink = Arc::new(FakeSink::default());
    let service = AccountService::new(transport, sink.clone());

    service.start_chatgpt_login().await.unwrap();
    yield_until(|| !sink.snapshot().is_empty()).await;

    assert!(!service.has_pending_login().await);
    assert_eq!(sink.snapshot(), vec![AccountChangeReason::LoginSucceeded]);
}

#[tokio::test]
async fn an_unapproved_server_url_is_never_opened_and_its_login_is_cancelled() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com.evil.example/auth?token=TOKEN-SECRET",
        )),
        Ok(json!({})),
    ]);
    let service = service(transport.clone(), Arc::new(FakeSink::default()));

    let error = service.start_chatgpt_login().await.unwrap_err();

    assert_eq!(error, AuthError::UnsafeAuthUrl);
    assert!(!service.has_pending_login().await);
    assert_eq!(
        transport.request_snapshot(),
        vec![
            (
                "account/login/start".to_owned(),
                json!({ "type": "chatgpt", "useHostedLoginSuccessPage": true, "appBrand": "chatgpt" }),
            ),
            (
                "account/login/cancel".to_owned(),
                json!({ "loginId": "LOGIN-SECRET" })
            ),
        ]
    );
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("LOGIN-SECRET"));
    assert!(!rendered.contains("TOKEN-SECRET"));
}

#[tokio::test]
async fn a_malformed_start_with_a_login_id_is_cancelled_without_becoming_orphaned() {
    let transport = FakeTransport::new(vec![
        Ok(json!({ "type": "chatgpt", "loginId": "LOGIN-SECRET" })),
        Ok(json!({})),
    ]);
    let service = service(transport.clone(), Arc::new(FakeSink::default()));

    assert_eq!(
        service.start_chatgpt_login().await.unwrap_err(),
        AuthError::UnexpectedLoginResponse
    );
    assert!(!service.has_pending_login().await);
    assert_eq!(transport.request_snapshot().len(), 2);
    assert_eq!(transport.request_snapshot()[1].0, "account/login/cancel");
}

#[tokio::test]
async fn malformed_start_cleanup_failure_remains_explicitly_cancellable() {
    let transport = FakeTransport::new(vec![
        Ok(json!({ "type": "chatgpt", "loginId": "LOGIN-SECRET" })),
        Err(TransportError::Remote { code: -32_000 }),
    ]);
    let service = service(transport, Arc::new(FakeSink::default()));

    assert_eq!(
        service.start_chatgpt_login().await.unwrap_err(),
        AuthError::LoginCleanupPending
    );
    assert!(service.has_pending_login().await);
}

#[tokio::test]
async fn only_a_matching_successful_login_completion_clears_pending_and_emits_a_safe_event() {
    let transport = FakeTransport::new(vec![Ok(login_response(
        "MATCHING-ID",
        "https://chatgpt.com/auth/start",
    ))]);
    let sink = Arc::new(FakeSink::default());
    let service = service(transport.clone(), sink.clone());
    service.start_chatgpt_login().await.unwrap();

    transport.notify(
        "account/login/completed",
        json!({ "loginId": "OTHER-ID", "success": true, "error": "TOKEN-SECRET" }),
    );
    tokio::task::yield_now().await;
    assert!(service.has_pending_login().await);
    assert!(sink.snapshot().is_empty());

    transport.notify(
        "account/login/completed",
        json!({ "loginId": "MATCHING-ID", "success": true, "error": "TOKEN-SECRET" }),
    );
    yield_until(|| !sink.snapshot().is_empty()).await;
    assert!(!service.has_pending_login().await);
    assert_eq!(sink.snapshot(), vec![AccountChangeReason::LoginSucceeded]);
    assert!(!format!("{:?}", sink.snapshot()).contains("TOKEN-SECRET"));
}

#[tokio::test]
async fn matching_failed_login_completion_clears_pending_and_emits_only_failure_state() {
    let transport = FakeTransport::new(vec![Ok(login_response(
        "MATCHING-ID",
        "https://chatgpt.com/auth/start",
    ))]);
    let sink = Arc::new(FakeSink::default());
    let service = service(transport.clone(), sink.clone());
    service.start_chatgpt_login().await.unwrap();

    transport.notify(
        "account/login/completed",
        json!({ "loginId": "MATCHING-ID", "success": false, "error": "TOKEN-SECRET remote detail" }),
    );
    yield_until(|| !sink.snapshot().is_empty()).await;

    assert!(!service.has_pending_login().await);
    assert_eq!(sink.snapshot(), vec![AccountChangeReason::LoginFailed]);
    let rendered = format!("{:?}", sink.snapshot());
    assert!(!rendered.contains("TOKEN-SECRET"));
    assert!(!rendered.contains("remote detail"));
}

#[tokio::test]
async fn malformed_matching_login_completion_does_not_clear_or_emit() {
    for malformed in [
        json!({ "loginId": "MATCHING-ID" }),
        json!({ "loginId": "MATCHING-ID", "success": "true" }),
        json!({ "loginId": "MATCHING-ID", "success": null }),
    ] {
        let transport = FakeTransport::new(vec![Ok(login_response(
            "MATCHING-ID",
            "https://chatgpt.com/auth/start",
        ))]);
        let sink = Arc::new(FakeSink::default());
        let service = service(transport.clone(), sink.clone());
        service.start_chatgpt_login().await.unwrap();

        transport.notify("account/login/completed", malformed);
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }

        assert!(service.has_pending_login().await);
        assert!(sink.snapshot().is_empty());
    }
}

#[tokio::test]
async fn account_updated_emits_only_a_sanitized_reason() {
    let transport = FakeTransport::new(vec![]);
    let sink = Arc::new(FakeSink::default());
    let _service = service(transport.clone(), sink.clone());

    transport.notify(
        "account/updated",
        json!({ "authMode": "chatgpt", "accessToken": "TOKEN-SECRET", "email": "private@example.com" }),
    );
    yield_until(|| !sink.snapshot().is_empty()).await;

    assert_eq!(sink.snapshot(), vec![AccountChangeReason::AccountUpdated]);
    let rendered = format!("{:?}", sink.snapshot());
    assert!(!rendered.contains("TOKEN-SECRET"));
    assert!(!rendered.contains("private@example.com"));
}

#[test]
fn browser_login_url_policy_allows_only_https_chatgpt_hosts_without_authority_tricks() {
    for allowed in [
        "https://chatgpt.com/auth/start",
        "https://auth.chatgpt.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A1455",
    ] {
        assert!(
            validate_browser_login_url(&url::Url::parse(allowed).unwrap()).is_ok(),
            "{allowed}"
        );
    }

    for denied in [
        "http://chatgpt.com/auth",
        "https://chatgpt.com:444/auth",
        "https://user@chatgpt.com/auth",
        "https://chatgpt.com.evil.example/auth",
        "https://evilchatgpt.com/auth",
        "https://xn--chatgpt-9za.com/auth",
        "https://127.0.0.1/auth",
    ] {
        assert!(
            validate_browser_login_url(&url::Url::parse(denied).unwrap()).is_err(),
            "{denied}"
        );
    }
}

#[tokio::test]
async fn opener_failure_is_sanitized_and_cancels_the_server_login() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com/auth/start?opaque=TOKEN-SECRET",
        )),
        Ok(json!({})),
    ]);
    let sink = Arc::new(FakeSink::default());
    let service = service(transport.clone(), sink.clone());
    let opener = FakeOpener {
        fail: true,
        opened: Mutex::new(0),
    };

    let error = start_login_and_open(&service, &opener).await.unwrap_err();
    let rendered = format!("{error:?} {error}");

    assert_eq!(error, AuthError::BrowserOpenFailed);
    assert!(!service.has_pending_login().await);
    assert!(sink.snapshot().is_empty());
    assert_eq!(*opener.opened.lock().unwrap(), 1);
    for secret in ["LOGIN-SECRET", "TOKEN-SECRET", "chatgpt.com/auth"] {
        assert!(!rendered.contains(secret));
    }
}

#[tokio::test]
async fn opener_and_cancel_failure_reports_that_the_pending_login_needs_retry() {
    let transport = FakeTransport::new(vec![
        Ok(login_response(
            "LOGIN-SECRET",
            "https://chatgpt.com/auth/start?opaque=TOKEN-SECRET",
        )),
        Err(TransportError::Remote { code: -32_000 }),
    ]);
    let service = service(transport, Arc::new(FakeSink::default()));
    let opener = FakeOpener {
        fail: true,
        opened: Mutex::new(0),
    };

    let error = start_login_and_open(&service, &opener).await.unwrap_err();

    assert_eq!(error, AuthError::BrowserOpenFailedLoginPending);
    assert!(service.has_pending_login().await);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("LOGIN-SECRET"));
    assert!(!rendered.contains("TOKEN-SECRET"));
}
