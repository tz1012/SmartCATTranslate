use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, PhysicalPosition, Runtime};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    commands::settings::open_store,
    core::types::{GlossaryMapping, TranslationMode, TranslationModel, TranslationRequest},
    hotkeys::{
        Blocklist, ClipboardGuard, HotkeyAction, NativeController, SelectedTextAcquirer,
        SequenceEngine,
    },
    settings::types::{AppLocale, AppSettings, SavedProfile},
};

#[cfg(target_os = "macos")]
use crate::platform::macos::{
    MacClipboard as NativeClipboard, MacCopySynthesizer as NativeCopy,
    MacForegroundAppProvider as NativeForeground, MacKeyEventSource as NativeSource,
};
#[cfg(windows)]
use crate::platform::windows::{
    WindowsClipboard as NativeClipboard, WindowsCopySynthesizer as NativeCopy,
    WindowsForegroundAppProvider as NativeForeground, WindowsKeyEventSource as NativeSource,
};

const POPUP_LABEL: &str = "quick-popup";
const POPUP_EVENT: &str = "quick-popup-request";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickPopupPayload {
    request_id: Uuid,
    request: Option<TranslationRequest>,
    profile_name: String,
    locale: AppLocale,
    error: Option<String>,
}

pub struct QuickHotkeyRuntime {
    controller: NativeController,
    stopped: Arc<AtomicBool>,
    dispatcher: Option<JoinHandle<()>>,
}

impl Drop for QuickHotkeyRuntime {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.controller.stop();
        if let Some(dispatcher) = self.dispatcher.take() {
            let _ = dispatcher.join();
        }
    }
}

#[tauri::command]
pub fn close_quick_popup<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    let popup = app
        .get_webview_window(POPUP_LABEL)
        .ok_or("popup_unavailable")?;
    popup.hide().map_err(|_| "popup_hide_failed".to_owned())?;
    let _ = popup.set_always_on_top(false);
    Ok(())
}

#[tauri::command]
pub fn open_main_window<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    let main = app
        .get_webview_window("main")
        .ok_or("main_window_unavailable")?;
    main.show()
        .map_err(|_| "main_window_unavailable".to_owned())?;
    main.set_focus()
        .map_err(|_| "main_window_unavailable".to_owned())
}

#[tauri::command]
pub fn show_quick_popup<R: Runtime>(
    app: tauri::AppHandle<R>,
    payload: QuickPopupPayload,
) -> Result<(), String> {
    present_popup(&app, payload)
}

pub async fn restart_quick_hotkeys<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    let settings = open_store(&app)?
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    let state = app.state::<AppState>();
    state.replace_quick_hotkeys(None);
    if settings.hotkeys.is_empty() {
        return Ok(());
    }
    let engine = SequenceEngine::new(settings.hotkeys.clone())
        .map_err(|_| "hotkey_runtime_unavailable".to_owned())?;
    let (controller, receiver) = NativeController::start(NativeSource::new(), engine)
        .map_err(|_| "hotkey_runtime_unavailable".to_owned())?;
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    let worker_app = app.clone();
    let dispatcher = thread::Builder::new()
        .name("smartcat-quick-popup-dispatch".to_owned())
        .spawn(move || {
            while !worker_stopped.load(Ordering::Acquire) {
                let ids = match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(ids) => ids,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };
                for id in ids {
                    let activation_app = worker_app.clone();
                    tauri::async_runtime::spawn(async move {
                        process_activation(activation_app, id).await;
                    });
                }
            }
        })
        .map_err(|_| "hotkey_runtime_unavailable".to_owned())?;
    state.replace_quick_hotkeys(Some(QuickHotkeyRuntime {
        controller,
        stopped,
        dispatcher: Some(dispatcher),
    }));
    Ok(())
}

async fn process_activation<R: Runtime>(app: tauri::AppHandle<R>, binding_id: Uuid) {
    let state = app.state::<AppState>();
    if state.hotkeys_suspended() {
        return;
    }
    let Ok(store) = open_store(&app) else { return };
    let Ok(settings) = store.load().await else {
        return;
    };
    let Some(binding) = settings
        .hotkeys
        .iter()
        .find(|binding| binding.id == binding_id)
    else {
        return;
    };
    if binding.action != HotkeyAction::TranslateSelection {
        return;
    }
    let profile = settings
        .profile(binding.profile_id)
        .or_else(|| settings.default_profile());
    let Some(profile) = profile else { return };

    let blocklist = match Blocklist::new(settings.blocked_apps.clone()) {
        Ok(blocklist) => blocklist,
        Err(_) => return,
    };
    let acquirer = SelectedTextAcquirer::new(
        Arc::new(NativeForeground),
        blocklist,
        ClipboardGuard::new(Arc::new(NativeClipboard), Arc::new(NativeCopy)),
    );
    let captured = acquirer
        .capture_selected_text(Duration::from_millis(650), false)
        .await;
    let payload = match captured {
        Ok(selection) => QuickPopupPayload {
            request_id: Uuid::new_v4(),
            request: Some(build_request(&settings, profile, selection.text)),
            profile_name: profile.name.clone(),
            locale: settings.locale,
            error: None,
        },
        Err(error) if error.code() == "blocked_application" => return,
        Err(error) => QuickPopupPayload {
            request_id: Uuid::new_v4(),
            request: None,
            profile_name: profile.name.clone(),
            locale: settings.locale,
            error: Some(error.code().to_owned()),
        },
    };
    let _ = present_popup(&app, payload);
}

fn build_request(settings: &AppSettings, saved: &SavedProfile, text: String) -> TranslationRequest {
    let mut profile = saved.profile.clone();
    let mut glossary = Vec::new();
    for entry in &settings.glossary {
        let source_matches = profile
            .source_language
            .as_ref()
            .is_none_or(|source| source.eq_ignore_ascii_case(&entry.source_language));
        if !source_matches
            || !profile
                .target_language
                .eq_ignore_ascii_case(&entry.target_language)
        {
            continue;
        }
        if entry.protect_only {
            if !profile.protected_terms.contains(&entry.source_term) {
                profile.protected_terms.push(entry.source_term.clone());
            }
        } else {
            glossary.push(GlossaryMapping {
                source_term: entry.source_term.clone(),
                target_term: entry.target_term.clone(),
            });
        }
    }
    TranslationRequest {
        text,
        profile,
        field: saved.field,
        glossary,
        mode: TranslationMode::Translate,
        secret: false,
        model: TranslationModel::Automatic,
    }
}

fn present_popup<R: Runtime>(
    app: &tauri::AppHandle<R>,
    payload: QuickPopupPayload,
) -> Result<(), String> {
    let popup = app
        .get_webview_window(POPUP_LABEL)
        .ok_or("popup_unavailable")?;
    place_popup(&popup);
    popup
        .set_always_on_top(true)
        .map_err(|_| "popup_show_failed".to_owned())?;
    popup.show().map_err(|_| "popup_show_failed".to_owned())?;
    popup
        .set_focus()
        .map_err(|_| "popup_show_failed".to_owned())?;
    popup
        .emit(POPUP_EVENT, payload)
        .map_err(|_| "popup_show_failed".to_owned())
}

fn place_popup<R: Runtime>(popup: &tauri::WebviewWindow<R>) {
    #[cfg(windows)]
    if let Some((x, y)) = windows_popup_position() {
        let _ = popup.set_position(PhysicalPosition::new(x, y));
        return;
    }
    if let Ok(Some(monitor)) = popup.current_monitor() {
        let position = monitor.position();
        let size = monitor.size();
        let x = position.x + (size.width.saturating_sub(560) / 2) as i32;
        let y = position.y + (size.height.saturating_sub(360) / 2) as i32;
        let _ = popup.set_position(PhysicalPosition::new(x, y));
    }
}

#[cfg(windows)]
fn windows_popup_position() -> Option<(i32, i32)> {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::WindowsAndMessaging::GetCursorPos,
    };
    let mut cursor = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut cursor) } == 0 {
        return None;
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    if monitor.is_null() || unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let x = (cursor.x + 16)
        .min(info.rcWork.right - 560)
        .max(info.rcWork.left + 16);
    let y = (cursor.y + 16)
        .min(info.rcWork.bottom - 360)
        .max(info.rcWork.top + 16);
    Some((x, y))
}
