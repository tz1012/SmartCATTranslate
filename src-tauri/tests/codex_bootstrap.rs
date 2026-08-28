#![cfg(feature = "test-helper")]

use std::sync::Arc;

use semver::Version;
use smartcat_translate::app_state::{AppState, AppStateError};
use smartcat_translate::codex::auth::{AccountChangeReason, AccountEventSink, AccountState};
use smartcat_translate::codex::bootstrap::{bootstrap_with_resolver, BootstrapError};
use smartcat_translate::codex::process::{ProcessRuntimeLauncher, CODEX_APP_SERVER_PROTOCOL};
use smartcat_translate::codex::runtime::{
    RuntimeCandidate, RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeResolver,
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

#[tokio::test]
async fn production_wiring_hands_the_initialized_process_to_account_commands_and_cleans_up() {
    let app_data = tempdir().unwrap();
    let work_root = app_data.path().join("runtime-work");
    let launcher = Arc::new(ProcessRuntimeLauncher::new(work_root.clone()));
    let candidate = RuntimeCandidate::system(
        env!("CARGO_BIN_EXE_smartcat-fake-codex"),
        Version::parse("0.144.4").unwrap(),
    );
    let resolver = RuntimeResolver::system_only(
        vec![candidate],
        "0.144.4",
        CODEX_APP_SERVER_PROTOCOL,
        launcher,
        Arc::new(NoopRecorder),
    );
    let state = AppState::default();

    bootstrap_with_resolver(
        &state,
        &resolver,
        Arc::new(NoopSink),
        app_data.path().to_path_buf(),
    )
    .await
    .unwrap();

    let service = state
        .account_service()
        .await
        .expect("bootstrap installs the account service");
    let snapshot = service.read_snapshot().await.unwrap();
    assert_eq!(
        snapshot.account,
        AccountState::SignedIn {
            email_hint: Some("p***@example.com".to_owned()),
            plan: Some("plus".to_owned()),
        }
    );
    assert!(!snapshot.login_pending);
    assert!(state.translation_jobs().await.is_some());
    drop(service);

    state.shutdown().await.unwrap();
    assert!(state.account_service().await.is_none());
    assert!(state.translation_jobs().await.is_none());
    assert!(work_root.read_dir().unwrap().next().is_none());
}

#[tokio::test]
async fn shutdown_wins_a_race_with_late_bootstrap_and_leaves_no_process_or_workdir() {
    let app_data = tempdir().unwrap();
    let work_root = app_data.path().join("runtime-work");
    let launcher = Arc::new(ProcessRuntimeLauncher::new(work_root.clone()));
    let candidate = RuntimeCandidate::system(
        env!("CARGO_BIN_EXE_smartcat-fake-codex"),
        Version::parse("0.144.4").unwrap(),
    );
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
}
