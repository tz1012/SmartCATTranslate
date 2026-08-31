use std::sync::Arc;
use std::time::Duration;

use smartcat_translate::app_state::AppState;

#[tokio::test]
async fn pending_account_service_wait_is_released_when_shutdown_starts() {
    let state = Arc::new(AppState::default());
    let waiting_state = state.clone();
    let waiter = tokio::spawn(async move {
        waiting_state
            .wait_for_account_service(Duration::from_secs(5))
            .await
    });

    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());

    state.shutdown().await.unwrap();
    assert!(waiter.await.unwrap().is_none());
}
