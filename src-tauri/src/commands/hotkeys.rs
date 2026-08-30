use chrono::Utc;
use std::sync::mpsc;
use tauri::{AppHandle, Runtime, State};

use crate::{
    app_state::AppState,
    commands::settings::open_store,
    hotkeys::{
        AppIdentity, AppInspector, BlockedApp, ConflictAnalyzer, ConflictReport,
        ForegroundAppProvider, HotkeyBinding, KeyEventSource, Platform, PlatformError,
        RegistrationProbe, RegistrationProbeStatus, ShortcutCatalog, Trigger,
    },
};

#[cfg(target_os = "macos")]
use crate::platform::macos::{
    MacForegroundAppProvider as NativeForeground, MacKeyEventSource as NativeSource,
    MacRegistrationProbe as NativeProbe,
};
#[cfg(windows)]
use crate::platform::windows::{
    WindowsForegroundAppProvider as NativeForeground, WindowsKeyEventSource as NativeSource,
    WindowsRegistrationProbe as NativeProbe,
};

const MAX_HOTKEYS: usize = 32;

struct ForegroundInspector;
struct RestoringUiProbe;

impl AppInspector for ForegroundInspector {
    fn running_apps(&self) -> Vec<AppIdentity> {
        NativeForeground.current().into_iter().collect()
    }
}

impl RegistrationProbe for RestoringUiProbe {
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus {
        if matches!(trigger, Trigger::Chord { .. }) {
            return NativeProbe::new().probe_and_restore(trigger);
        }
        if matches!(
            NativeProbe::new().probe_and_restore(trigger),
            RegistrationProbeStatus::AvailableViaObserver
        ) {
            return RegistrationProbeStatus::AvailableViaObserver;
        }
        let (sender, _receiver) = mpsc::sync_channel(1);
        match NativeSource::new().start(sender) {
            Ok(mut observer) => match observer.stop() {
                Ok(()) => RegistrationProbeStatus::AvailableViaObserver,
                Err(_) => RegistrationProbeStatus::BackendError,
            },
            Err(
                PlatformError::AccessibilityPermissionRequired | PlatformError::PermissionDenied,
            ) => RegistrationProbeStatus::PermissionDenied,
            Err(PlatformError::InvalidKey) => RegistrationProbeStatus::Invalid,
            Err(_) => RegistrationProbeStatus::BackendError,
        }
    }
}

fn platform() -> Platform {
    #[cfg(windows)]
    {
        Platform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
}

fn analyze(trigger: &Trigger) -> Result<ConflictReport, String> {
    let catalog = ShortcutCatalog::from_embedded(Utc::now().date_naive())
        .map_err(|_| "hotkey_catalog_unavailable".to_owned())?;
    let probe = RestoringUiProbe;
    Ok(ConflictAnalyzer::new(platform(), &catalog, &probe, &ForegroundInspector).analyze(trigger))
}

#[tauri::command]
pub fn suspend_hotkeys(state: State<'_, AppState>, suspended: bool) {
    state.set_hotkeys_suspended(suspended);
}

#[tauri::command]
pub fn analyze_hotkey(trigger: Trigger) -> Result<ConflictReport, String> {
    analyze(&trigger)
}

#[tauri::command]
pub async fn list_hotkeys<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Vec<HotkeyBinding>, String> {
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    Ok(open_store(&app)?
        .load()
        .await
        .map_err(|error| error.code().to_owned())?
        .hotkeys)
}

#[tauri::command]
pub async fn save_hotkey<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    binding: HotkeyBinding,
) -> Result<Vec<HotkeyBinding>, String> {
    let report = analyze(&binding.trigger)?;
    if !report.registration_allowed(binding.force) {
        return Err("hotkey_conflict".to_owned());
    }
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    let store = open_store(&app)?;
    let mut settings = store
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    if let Some(existing) = settings
        .hotkeys
        .iter_mut()
        .find(|existing| existing.id == binding.id)
    {
        *existing = binding;
    } else {
        if settings.hotkeys.len() >= MAX_HOTKEYS {
            return Err("too_many_hotkeys".to_owned());
        }
        settings.hotkeys.push(binding);
    }
    store
        .save(&settings)
        .await
        .map_err(|error| error.code().to_owned())?;
    crate::commands::windows::restart_quick_hotkeys(app.clone()).await?;
    Ok(settings.hotkeys)
}

#[tauri::command]
pub async fn list_blocked_apps<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Vec<BlockedApp>, String> {
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    Ok(open_store(&app)?
        .load()
        .await
        .map_err(|error| error.code().to_owned())?
        .blocked_apps)
}

#[tauri::command]
pub async fn save_blocked_apps<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    blocked_apps: Vec<BlockedApp>,
) -> Result<Vec<BlockedApp>, String> {
    let _operation = state
        .lock_settings_operation()
        .await
        .map_err(|_| "settings_shutting_down".to_owned())?;
    let store = open_store(&app)?;
    let mut settings = store
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    settings.blocked_apps = blocked_apps;
    store
        .save(&settings)
        .await
        .map_err(|error| error.code().to_owned())?;
    Ok(settings.blocked_apps)
}
