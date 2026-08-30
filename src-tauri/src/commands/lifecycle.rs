use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::{
    app_state::AppState,
    commands::settings::open_store,
    lifecycle::LifecycleState,
    settings::types::{AppSettings, CloseBehavior, QuickAccessPosition},
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleStatus {
    launch_at_login_available: bool,
    launch_at_login_enabled: bool,
    hotkeys_paused: bool,
}

#[tauri::command]
pub fn get_lifecycle_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LifecycleStatus, String> {
    let available = !cfg!(debug_assertions);
    let enabled = if available {
        app.autolaunch()
            .is_enabled()
            .map_err(|_| "launch_at_login_unavailable".to_owned())?
    } else {
        false
    };
    Ok(LifecycleStatus {
        launch_at_login_available: available,
        launch_at_login_enabled: enabled,
        hotkeys_paused: state.hotkeys_suspended(),
    })
}

#[tauri::command]
pub async fn set_launch_at_login(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSettings, String> {
    if cfg!(debug_assertions) {
        return Err("launch_at_login_installed_only".to_owned());
    }
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    let autostart = app.autolaunch();
    let previous = autostart
        .is_enabled()
        .map_err(|_| "launch_at_login_unavailable".to_owned())?;
    if enabled != previous {
        if enabled {
            autostart.enable()
        } else {
            autostart.disable()
        }
        .map_err(|_| "launch_at_login_permission_denied".to_owned())?;
    }
    let store = open_store(&app)?;
    let mut settings = store
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    settings.launch_at_login = enabled;
    if let Err(error) = store.save(&settings).await {
        let _ = if previous {
            autostart.enable()
        } else {
            autostart.disable()
        };
        return Err(error.code().to_owned());
    }
    Ok(settings)
}

#[tauri::command]
pub async fn set_close_behavior(
    app: AppHandle,
    state: State<'_, AppState>,
    close_behavior: CloseBehavior,
) -> Result<AppSettings, String> {
    update_lifecycle_settings(app, state, |settings| {
        settings.close_behavior = close_behavior
    })
    .await
}

#[tauri::command]
pub async fn set_quick_access_position(
    app: AppHandle,
    state: State<'_, AppState>,
    quick_access_position: QuickAccessPosition,
) -> Result<AppSettings, String> {
    update_lifecycle_settings(app, state, |settings| {
        settings.quick_access_position = quick_access_position
    })
    .await
}

async fn update_lifecycle_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    update: impl FnOnce(&mut AppSettings),
) -> Result<AppSettings, String> {
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    let store = open_store(&app)?;
    let mut settings = store
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    update(&mut settings);
    store
        .save(&settings)
        .await
        .map_err(|error| error.code().to_owned())?;
    Ok(settings)
}

#[tauri::command]
pub fn set_hotkeys_paused(app: AppHandle, app_state: State<'_, AppState>, paused: bool) {
    app_state.set_hotkeys_suspended(paused);
    app.state::<LifecycleState>().set_paused(paused);
}

#[tauri::command]
pub fn quit_application(app: AppHandle) {
    crate::lifecycle::request_quit(app);
}
