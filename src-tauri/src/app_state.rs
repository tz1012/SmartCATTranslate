use std::sync::Arc;

use tokio::sync::RwLock;

use crate::codex::auth::AccountService;

#[derive(Default)]
pub struct AppState {
    account_service: RwLock<Option<Arc<AccountService>>>,
}

impl AppState {
    pub async fn install_account_service(&self, service: Arc<AccountService>) {
        *self.account_service.write().await = Some(service);
    }

    pub async fn account_service(&self) -> Option<Arc<AccountService>> {
        self.account_service.read().await.clone()
    }
}
