#![cfg(feature = "test-helper")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use smartcat_translate::app_state::{AppState, AppStateError};
use smartcat_translate::codex::auth::{
    start_login_and_open, AccountChangeReason, AccountEventSink, AccountState, BrowserOpenError,
    BrowserOpener,
};
use smartcat_translate::codex::bootstrap::{bootstrap_with_resolver, BootstrapError};
use smartcat_translate::codex::process::{
    sandboxed_app_data_root, ProcessRuntimeLauncher, CODEX_APP_SERVER_PROTOCOL,
};
use smartcat_translate::codex::runtime::{
    RuntimeCandidate, RuntimeError, RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeLauncher,
    RuntimeResolver,
};
use tempfile::tempdir;

struct NoopSink;

impl AccountEventSink for NoopSink {
    fn account_state_changed(&self, _reason: AccountChangeReason) {}
}

struct NoopRecorder;

impl RuntimeFailureRecorder for NoopRecorder {
    fn record(&self, _record: RuntimeFailureRecord) {}
}

struct SafeRecordingOpener(AtomicUsize);

impl BrowserOpener for SafeRecordingOpener {
    fn open(&self, url: &url::Url) -> Result<(), BrowserOpenError> {
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("chatgpt.com"));
        assert!(url.username().is_empty());
        assert!(url.password().is_none());
        assert!(url.port().is_none());
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

async fn lock_process_integration() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

#[tokio::test]
#[ignore = "requires the locally built SmartCAT patched Codex sidecar"]
async fn actual_patched_sidecar_initializes_and_attests_on_the_live_session() {
    let _serial = lock_process_integration().await;
    let binary = std::env::var_os("SMARTCAT_PATCHED_CODEX_TEST_BINARY")
        .expect("set SMARTCAT_PATCHED_CODEX_TEST_BINARY for this acceptance test");
    let app_data = tempdir().unwrap();
    let empty_credentials = tempdir().unwrap();
    let launcher = ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        Some(empty_credentials.path().to_path_buf()),
    );
    let candidate = RuntimeCandidate::system(binary, Version::parse("0.144.4").unwrap());

    let mut session = launcher
        .start(&candidate)
        .await
        .expect("the patched sidecar must start");
    let protocol = session
        .initialize()
        .await
        .expect("the patched sidecar must initialize and attest on this session");
    assert_eq!(protocol, CODEX_APP_SERVER_PROTOCOL);
    session.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an explicitly selected unpatched stock Codex binary"]
async fn actual_stock_codex_is_rejected_without_smartcat_attestation() {
    let _serial = lock_process_integration().await;
    let binary = std::env::var_os("SMARTCAT_STOCK_CODEX_TEST_BINARY")
        .expect("set SMARTCAT_STOCK_CODEX_TEST_BINARY for this acceptance test");
    let app_data = tempdir().unwrap();
    let empty_credentials = tempdir().unwrap();
    let launcher = ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        Some(empty_credentials.path().to_path_buf()),
    );
    // Deliberately provide the expected semantic version so this test reaches
    // the live-session attestation boundary rather than a version precheck.
    let candidate = RuntimeCandidate::system(binary, Version::parse("0.144.4").unwrap());

    let mut session = launcher
        .start(&candidate)
        .await
        .expect("stock Codex starts");
    let error = session
        .initialize()
        .await
        .expect_err("an unpatched stock Codex session must not attest");
    let _ = session.stop().await;
    assert_eq!(error, RuntimeError::HandshakeFailed);
}

#[tokio::test]
async fn installed_runtime_bootstrap_waits_then_opens_a_validated_login_and_cleans_up() {
    let _serial = lock_process_integration().await;
    let app_data = tempdir().unwrap();
    let sandbox_root = sandboxed_app_data_root(app_data.path()).unwrap();
    let work_root = sandbox_root.join("runtime-work");
    let source_home = tempdir().unwrap();
    std::fs::write(
        source_home.path().join("config.toml"),
        "developer_instructions = 'HOSTILE_USER_CONFIG'\n[features]\nshell_tool = true\n",
    )
    .unwrap();
    let account_state = br#"{"auth_mode":"chatgpt","tokens":{"id_token":"header.payload.signature","access_token":"PRIVATE_TEST_TOKEN","refresh_token":"PRIVATE_REFRESH","account_id":"account-1"},"last_refresh":"2026-08-28T12:00:00Z"}"#;
    std::fs::write(source_home.path().join("auth.json"), account_state).unwrap();
    let isolated_home = sandbox_root.join("codex-home");
    std::fs::create_dir_all(&isolated_home).unwrap();
    std::fs::write(
        isolated_home.join("config.toml"),
        "developer_instructions = 'HOSTILE_USER_CONFIG'\n",
    )
    .unwrap();
    let launcher = Arc::new(ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        Some(source_home.path().to_path_buf()),
    ));
    let sandbox_bin = sandbox_root.join("bin");
    std::fs::create_dir_all(&sandbox_bin).unwrap();
    let fake_binary = sandbox_bin.join("smartcat-fake-codex.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_smartcat-fake-codex"), &fake_binary).unwrap();
    let candidate = RuntimeCandidate::system(fake_binary, Version::parse("0.144.4").unwrap());
    let resolver = RuntimeResolver::system_only(
        vec![candidate],
        "0.144.4",
        CODEX_APP_SERVER_PROTOCOL,
        launcher,
        Arc::new(NoopRecorder),
    );
    let state = Arc::new(AppState::default());
    let waiting_state = state.clone();
    let service_waiter = tokio::spawn(async move {
        waiting_state
            .wait_for_account_service(Duration::from_secs(10))
            .await
            .expect("an installed-app login request waits for runtime bootstrap")
    });

    bootstrap_with_resolver(
        &state,
        &resolver,
        Arc::new(NoopSink),
        app_data.path().to_path_buf(),
    )
    .await
    .unwrap();

    let service = service_waiter.await.unwrap();
    let snapshot = service.read_snapshot().await.unwrap();
    assert_eq!(
        snapshot.account,
        AccountState::SignedIn {
            email_hint: Some("p***@example.com".to_owned()),
            plan: Some("plus".to_owned()),
        }
    );
    assert!(!snapshot.login_pending);
    let opener = SafeRecordingOpener(AtomicUsize::new(0));
    start_login_and_open(&service, &opener).await.unwrap();
    assert_eq!(opener.0.load(Ordering::Relaxed), 1);
    assert!(service.cancel_login().await.unwrap());
    assert!(!service.has_pending_login().await);
    assert!(state.translation_jobs().await.is_some());
    let isolated_config = std::fs::read_to_string(isolated_home.join("config.toml")).unwrap();
    assert!(!isolated_config.contains("HOSTILE_USER_CONFIG"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(isolated_home.join("auth.json")).unwrap()
        )
        .unwrap(),
        serde_json::from_slice::<serde_json::Value>(account_state).unwrap()
    );
    assert!(
        std::fs::read_to_string(source_home.path().join("config.toml"))
            .unwrap()
            .contains("HOSTILE_USER_CONFIG")
    );
    drop(service);

    state.shutdown().await.unwrap();
    assert!(state.account_service().await.is_none());
    assert!(state.translation_jobs().await.is_none());
    assert!(work_root.read_dir().unwrap().next().is_none());
}

#[tokio::test]
async fn stock_codex_without_the_live_smartcat_attestation_is_rejected() {
    let _serial = lock_process_integration().await;
    let app_data = tempdir().unwrap();
    let sandbox_root = sandboxed_app_data_root(app_data.path()).unwrap();
    let launcher = Arc::new(ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        None,
    ));
    let binary_dir = sandbox_root.join("bin");
    std::fs::create_dir_all(&binary_dir).unwrap();
    let stock_binary = binary_dir.join("stock-codex.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_smartcat-fake-codex"), &stock_binary).unwrap();
    let resolver = RuntimeResolver::system_only(
        vec![RuntimeCandidate::system(
            stock_binary,
            Version::parse("0.144.4").unwrap(),
        )],
        "0.144.4",
        CODEX_APP_SERVER_PROTOCOL,
        launcher,
        Arc::new(NoopRecorder),
    );

    let error = match resolver.resolve().await {
        Ok(_) => panic!("stock Codex was accepted without SmartCAT attestation"),
        Err(error) => error,
    };

    assert_eq!(error, RuntimeError::NoCompatibleRuntime);
}

#[tokio::test]
async fn shutdown_wins_a_race_with_late_bootstrap_and_leaves_no_process_or_workdir() {
    let _serial = lock_process_integration().await;
    let app_data = tempdir().unwrap();
    let sandbox_root = sandboxed_app_data_root(app_data.path()).unwrap();
    let work_root = sandbox_root.join("runtime-work");
    let launcher = Arc::new(ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        None,
    ));
    let sandbox_bin = sandbox_root.join("bin");
    std::fs::create_dir_all(&sandbox_bin).unwrap();
    let fake_binary = sandbox_bin.join("smartcat-fake-codex.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_smartcat-fake-codex"), &fake_binary).unwrap();
    let candidate = RuntimeCandidate::system(fake_binary, Version::parse("0.144.4").unwrap());
    let resolver = RuntimeResolver::system_only(
        vec![candidate],
        "0.144.4",
        CODEX_APP_SERVER_PROTOCOL,
        launcher,
        Arc::new(NoopRecorder),
    );
    let state = AppState::default();
    state.shutdown().await.unwrap();

    let error = bootstrap_with_resolver(
        &state,
        &resolver,
        Arc::new(NoopSink),
        app_data.path().to_path_buf(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, BootstrapError::State(AppStateError::ShuttingDown));
    assert!(state.account_service().await.is_none());
    assert!(work_root.read_dir().unwrap().next().is_none());
    let isolated_config =
        std::fs::read_to_string(sandbox_root.join("codex-home").join("config.toml")).unwrap();
    assert!(isolated_config.contains("cli_auth_credentials_store = \"keyring\""));
}

#[tokio::test]
async fn refuses_to_import_credentials_through_a_reparse_point() {
    let _serial = lock_process_integration().await;
    let app_data = tempdir().unwrap();
    let source_root = tempdir().unwrap();
    let actual = source_root.path().join("actual-codex-home");
    std::fs::create_dir(&actual).unwrap();
    std::fs::write(
        actual.join("auth.json"),
        br#"{"auth_mode":"chatgpt","tokens":{"id_token":"header.payload.signature","access_token":"PRIVATE","refresh_token":"PRIVATE_REFRESH"}}"#,
    )
    .unwrap();
    let linked = source_root.path().join("linked-codex-home");
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&linked)
            .arg(&actual)
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&actual, &linked).unwrap();
    let launcher = Arc::new(ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        Some(linked),
    ));
    let resolver = RuntimeResolver::system_only(
        vec![RuntimeCandidate::system(
            env!("CARGO_BIN_EXE_smartcat-fake-codex"),
            Version::parse("0.144.4").unwrap(),
        )],
        "0.144.4",
        CODEX_APP_SERVER_PROTOCOL,
        launcher,
        Arc::new(NoopRecorder),
    );

    let error = match resolver.resolve().await {
        Ok(_) => panic!("credentials behind a reparse point were imported"),
        Err(error) => error,
    };

    assert_eq!(error, RuntimeError::FilesystemFailed);
}
