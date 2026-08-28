#![cfg(feature = "test-helper")]

use std::sync::Arc;

use semver::Version;
use smartcat_translate::app_state::{AppState, AppStateError};
use smartcat_translate::codex::auth::{AccountChangeReason, AccountEventSink, AccountState};
use smartcat_translate::codex::bootstrap::{bootstrap_with_resolver, BootstrapError};
use smartcat_translate::codex::process::{ProcessRuntimeLauncher, CODEX_APP_SERVER_PROTOCOL};
use smartcat_translate::codex::runtime::{
    RuntimeCandidate, RuntimeError, RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeResolver,
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
    let source_home = tempdir().unwrap();
    std::fs::write(
        source_home.path().join("config.toml"),
        "developer_instructions = 'HOSTILE_USER_CONFIG'\n[features]\nshell_tool = true\n",
    )
    .unwrap();
    let account_state = br#"{"tokens":{"access_token":"PRIVATE_TEST_TOKEN"}}"#;
    std::fs::write(source_home.path().join("auth.json"), account_state).unwrap();
    let isolated_home = app_data.path().join("codex-home");
    std::fs::create_dir(&isolated_home).unwrap();
    std::fs::write(
        isolated_home.join("config.toml"),
        "developer_instructions = 'HOSTILE_USER_CONFIG'\n",
    )
    .unwrap();
    let launcher = Arc::new(ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        Some(source_home.path().to_path_buf()),
    ));
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
    let isolated_config = std::fs::read_to_string(isolated_home.join("config.toml")).unwrap();
    assert!(!isolated_config.contains("HOSTILE_USER_CONFIG"));
    assert_eq!(
        std::fs::read(isolated_home.join("auth.json")).unwrap(),
        account_state
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
async fn shutdown_wins_a_race_with_late_bootstrap_and_leaves_no_process_or_workdir() {
    let app_data = tempdir().unwrap();
    let work_root = app_data.path().join("runtime-work");
    let launcher = Arc::new(ProcessRuntimeLauncher::with_credential_source(
        app_data.path().to_path_buf(),
        None,
    ));
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
    let isolated_config =
        std::fs::read_to_string(app_data.path().join("codex-home").join("config.toml")).unwrap();
    assert!(isolated_config.contains("cli_auth_credentials_store = \"keyring\""));
}

#[tokio::test]
async fn refuses_to_import_credentials_through_a_reparse_point() {
    let app_data = tempdir().unwrap();
    let source_root = tempdir().unwrap();
    let actual = source_root.path().join("actual-codex-home");
    std::fs::create_dir(&actual).unwrap();
    std::fs::write(
        actual.join("auth.json"),
        br#"{"tokens":{"access_token":"PRIVATE"}}"#,
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
