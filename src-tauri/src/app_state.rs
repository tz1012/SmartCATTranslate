use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::codex::auth::AccountService;
use crate::codex::transport::{JsonlAppServerTransport, TransportError};

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
            });
        }
    }

    pub async fn install_account_runtime(
        &self,
        service: Arc<AccountService>,
        transport: Arc<JsonlAppServerTransport>,
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

    pub async fn shutdown(&self) -> Result<(), TransportError> {
        self.shutting_down.store(true, Ordering::Release);
        let installed = self.runtime.write().await.take();
        let Some(installed) = installed else {
            return Ok(());
        };
        drop(installed.service);
        match installed.transport {
            Some(transport) => transport.shutdown().await,
            None => Ok(()),
        }
    }
}

struct InstalledAccountRuntime {
    service: Arc<AccountService>,
    transport: Option<Arc<JsonlAppServerTransport>>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppStateError {
    #[error("the Codex account runtime is already installed")]
    AlreadyInstalled,
    #[error("the application is shutting down")]
    ShuttingDown,
}
