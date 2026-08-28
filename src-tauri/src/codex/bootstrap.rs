use std::path::PathBuf;
use std::sync::Arc;

use crate::app_state::{AppState, AppStateError};
use crate::codex::auth::{AccountEventSink, AccountService};
use crate::codex::install::{AppLocalRuntimeInstaller, ProductionRuntimeDownloader};
use crate::codex::manifest::{EmbeddedRuntimeManifest, HostTarget};
use crate::codex::process::{ProcessRuntimeLauncher, CODEX_APP_SERVER_PROTOCOL};
use crate::codex::runtime::{
    OfficialSystemDiscovery, ResolvedRuntime, RuntimeError, RuntimeFailureRecord,
    RuntimeFailureRecorder, RuntimeResolver,
};
use crate::codex::translation::{prepare_owned_empty_workspace, CodexTranslationBackend};
use crate::codex::transport::{JsonlAppServerTransport, TransportError};
use crate::commands::translation::TranslationJobManager;
use crate::core::errors::TranslationError;

pub async fn bootstrap_with_resolver(
    state: &AppState,
    resolver: &RuntimeResolver,
    event_sink: Arc<dyn AccountEventSink>,
    app_data_root: PathBuf,
) -> Result<(), BootstrapError> {
    let runtime = resolver.resolve().await?;
    install_resolved_account_service(state, runtime, event_sink, app_data_root).await
}

pub async fn bootstrap_account_service(
    state: &AppState,
    app_data_root: PathBuf,
    event_sink: Arc<dyn AccountEventSink>,
) -> Result<(), BootstrapError> {
    let pinned = EmbeddedRuntimeManifest::load_for_host(HostTarget::current())?;
    let downloader = Arc::new(ProductionRuntimeDownloader::new()?);
    let installer = AppLocalRuntimeInstaller::new(app_data_root.join("codex-runtime"), downloader)?;
    let launcher = Arc::new(ProcessRuntimeLauncher::new(
        app_data_root.join("runtime-work"),
    ));
    let resolver = RuntimeResolver::with_verified_installer(
        &OfficialSystemDiscovery::new(),
        installer,
        pinned.version().to_string(),
        CODEX_APP_SERVER_PROTOCOL,
        launcher,
        Arc::new(SilentRuntimeFailureRecorder),
    )?;
    bootstrap_with_resolver(state, &resolver, event_sink, app_data_root).await
}

struct SilentRuntimeFailureRecorder;

impl RuntimeFailureRecorder for SilentRuntimeFailureRecorder {
    fn record(&self, _record: RuntimeFailureRecord) {}
}

pub async fn install_resolved_account_service(
    state: &AppState,
    runtime: ResolvedRuntime,
    event_sink: Arc<dyn AccountEventSink>,
    app_data_root: PathBuf,
) -> Result<(), BootstrapError> {
    let transport = Arc::new(JsonlAppServerTransport::from_resolved_runtime(runtime)?);
    let service = Arc::new(AccountService::new(transport.clone(), event_sink));
    let workspace = match prepare_owned_empty_workspace(&app_data_root) {
        Ok(workspace) => workspace,
        Err(error) => {
            drop(service);
            let _ = transport.shutdown().await;
            return Err(error.into());
        }
    };
    let backend = match CodexTranslationBackend::new(transport.clone(), &workspace).await {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            drop(service);
            let _ = transport.shutdown().await;
            return Err(error.into());
        }
    };
    let translation_jobs = Arc::new(TranslationJobManager::new(backend));
    if let Err(error) = state
        .install_account_runtime(service.clone(), transport.clone(), translation_jobs.clone())
        .await
    {
        let _ = translation_jobs.shutdown().await;
        drop(service);
        let _ = transport.shutdown().await;
        return Err(error.into());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BootstrapError {
    #[error("the Codex account runtime could not be prepared")]
    Runtime(#[from] RuntimeError),
    #[error("the Codex App Server transport could not start")]
    Transport(#[from] TransportError),
    #[error("the translation coordinator could not start")]
    Translation(#[from] TranslationError),
    #[error("the Codex account runtime was already installed")]
    State(#[from] AppStateError),
}
