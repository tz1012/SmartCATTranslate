use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::{Mutex, MutexGuard, RwLock};

use crate::codex::auth::AccountService;
use crate::codex::transport::{JsonlAppServerTransport, TransportError};
use crate::commands::translation::{
    new_owner_job_registry, SharedOwnerJobRegistry, TranslationJobManager,
};
use crate::core::errors::TranslationError;
use crate::settings::ModelCatalogService;

pub struct AppState {
    runtime: RwLock<Option<InstalledAccountRuntime>>,
    shutting_down: AtomicBool,
    translation_owners: SharedOwnerJobRegistry,
    settings_operation: Mutex<()>,
    hotkeys_suspended: AtomicBool,
    quick_hotkeys: std::sync::Mutex<Option<crate::commands::windows::QuickHotkeyRuntime>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            runtime: RwLock::new(None),
            shutting_down: AtomicBool::new(false),
            translation_owners: new_owner_job_registry(),
            settings_operation: Mutex::new(()),
            hotkeys_suspended: AtomicBool::new(false),
            quick_hotkeys: std::sync::Mutex::new(None),
        }
    }
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

    pub async fn model_catalog_service(&self) -> Option<ModelCatalogService> {
        let runtime = self.runtime.read().await;
        let transport = runtime.as_ref()?.transport.as_ref()?.clone();
        Some(ModelCatalogService::new(transport))
    }

    pub(crate) async fn lock_settings_operation(
        &self,
    ) -> Result<MutexGuard<'_, ()>, SettingsOperationError> {
        let operation = self.settings_operation.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            Err(SettingsOperationError::ShuttingDown)
        } else {
            Ok(operation)
        }
    }

    pub(crate) fn translation_owner_registry(&self) -> SharedOwnerJobRegistry {
        self.translation_owners.clone()
    }

    pub(crate) fn set_hotkeys_suspended(&self, suspended: bool) {
        self.hotkeys_suspended.store(suspended, Ordering::Release);
    }

    pub fn hotkeys_suspended(&self) -> bool {
        self.hotkeys_suspended.load(Ordering::Acquire)
    }

    pub(crate) fn replace_quick_hotkeys(
        &self,
        runtime: Option<crate::commands::windows::QuickHotkeyRuntime>,
    ) {
        let mut slot = self
            .quick_hotkeys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::mem::replace(&mut *slot, runtime);
        drop(slot);
        drop(previous);
    }

    pub(crate) fn reserve_window_translation_job(
        &self,
        owner: &str,
    ) -> Result<uuid::Uuid, TranslationError> {
        let job_id = uuid::Uuid::new_v4();
        self.translation_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(owner.to_owned(), job_id)?;
        Ok(job_id)
    }

    pub(crate) fn release_window_translation_job(&self, job_id: uuid::Uuid) {
        self.translation_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(job_id);
    }

    pub fn tombstone_window_translation_jobs(&self, owner: &str) -> Vec<uuid::Uuid> {
        self.translation_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .tombstone(owner)
    }

    pub async fn cancel_tombstoned_translation_jobs(&self, job_ids: &[uuid::Uuid]) {
        if let Some(manager) = self.translation_jobs().await {
            manager.cancel_job_ids(job_ids).await;
        }
    }

    pub async fn cancel_window_translation_jobs(&self, owner: &str) {
        let job_ids = self.tombstone_window_translation_jobs(owner);
        self.cancel_tombstoned_translation_jobs(&job_ids).await;
    }

    pub async fn shutdown(&self) -> Result<(), AppShutdownError> {
        self.shutting_down.store(true, Ordering::Release);
        self.replace_quick_hotkeys(None);
        let settings_operation = self.settings_operation.lock().await;
        drop(settings_operation);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SettingsOperationError {
    #[error("the application is shutting down")]
    ShuttingDown,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::AppState;

    #[tokio::test]
    async fn settings_operations_are_serialized_and_shutdown_waits_for_the_active_operation() {
        let state = Arc::new(AppState::default());
        let active = state.lock_settings_operation().await.unwrap();
        let waiting_state = state.clone();
        let waiting = tokio::spawn(async move {
            let _operation = waiting_state.lock_settings_operation().await.unwrap();
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(active);
        waiting.await.unwrap();

        let active = state.lock_settings_operation().await.unwrap();
        let shutdown_state = state.clone();
        let shutdown = tokio::spawn(async move { shutdown_state.shutdown().await });
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());
        drop(active);
        shutdown.await.unwrap().unwrap();
        assert!(state.lock_settings_operation().await.is_err());
    }
}
