use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::codex::auth::AccountService;
use crate::codex::transport::{JsonlAppServerTransport, TransportError};
use crate::commands::translation::TranslationJobManager;
use crate::core::errors::TranslationError;

#[derive(Default)]
pub struct AppState {
    runtime: RwLock<Option<InstalledAccountRuntime>>,
    shutting_down: AtomicBool,
}

impl AppState {
    pub async fn install_account_service(&self, service: Arc<AccountService>) {
        let mut runtime = self.runtime.write().await;
        if runtime.is_none() && !self.shutting_down.load(Ordering::Acquire) {
            *runtime = Some(InstalledAccountRuntime {
                service,
                transport: None,
                translation_jobs: None,
            });
        }
    }

    pub async fn install_account_runtime(
        &self,
        service: Arc<AccountService>,
        transport: Arc<JsonlAppServerTransport>,
        translation_jobs: Arc<TranslationJobManager>,
    ) -> Result<(), AppStateError> {
        let mut runtime = self.runtime.write().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(AppStateError::ShuttingDown);
        }
        if runtime.is_some() {
            return Err(AppStateError::AlreadyInstalled);
        }
        *runtime = Some(InstalledAccountRuntime {
            service,
            transport: Some(transport),
            translation_jobs: Some(translation_jobs),
        });
        Ok(())
    }

    pub async fn account_service(&self) -> Option<Arc<AccountService>> {
        self.runtime
            .read()
            .await
            .as_ref()
            .map(|runtime| runtime.service.clone())
    }

    pub async fn translation_jobs(&self) -> Option<Arc<TranslationJobManager>> {
        self.runtime
            .read()
            .await
            .as_ref()
            .and_then(|runtime| runtime.translation_jobs.clone())
    }

    pub async fn cancel_window_translation_jobs(&self, owner: &str) {
        if let Some(manager) = self.translation_jobs().await {
            manager.cancel_owner(owner).await;
        }
    }

    pub async fn shutdown(&self) -> Result<(), AppShutdownError> {
        self.shutting_down.store(true, Ordering::Release);
        let installed = self.runtime.write().await.take();
        let Some(installed) = installed else {
            return Ok(());
        };
        let translation_result = match installed.translation_jobs {
            Some(manager) => manager
                .shutdown()
                .await
                .map_err(AppShutdownError::Translation),
            None => Ok(()),
        };
        drop(installed.service);
        let transport_result = match installed.transport {
            Some(transport) => transport
                .shutdown()
                .await
                .map_err(AppShutdownError::Transport),
            None => Ok(()),
        };
        match (translation_result, transport_result) {
            (Err(error), _) | (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

struct InstalledAccountRuntime {
    service: Arc<AccountService>,
    transport: Option<Arc<JsonlAppServerTransport>>,
    translation_jobs: Option<Arc<TranslationJobManager>>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppStateError {
    #[error("the Codex account runtime is already installed")]
    AlreadyInstalled,
    #[error("the application is shutting down")]
    ShuttingDown,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppShutdownError {
    #[error("the translation coordinator could not stop")]
    Translation(TranslationError),
    #[error("the Codex transport could not stop")]
    Transport(TransportError),
}
