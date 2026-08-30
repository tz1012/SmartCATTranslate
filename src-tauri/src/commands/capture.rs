use std::path::PathBuf;

use serde::Serialize;
use tauri::{Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::{
    capture::{
        CaptureCoordinator, CaptureJobResult, CaptureJobStatus, CaptureSelection, MonitorInfo,
        OverlayDescriptor,
    },
    commands::settings::open_store,
    hotkeys::{Blocklist, ForegroundAppProvider},
};

#[cfg(target_os = "macos")]
use crate::platform::macos::MacForegroundAppProvider as NativeForeground;
#[cfg(windows)]
use crate::platform::windows::WindowsForegroundAppProvider as NativeForeground;

const OVERLAY_PREFIX: &str = "capture-overlay-";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCaptureResult {
    pub session_id: Uuid,
    pub monitors: Vec<MonitorInfo>,
}

#[tauri::command]
pub async fn start_screen_capture<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<StartCaptureResult, String> {
    let settings = open_store(&app)?
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    let identity = NativeForeground
        .current()
        .map_err(|_| "foreground_unavailable".to_owned())?;
    let blocklist =
        Blocklist::new(settings.blocked_apps).map_err(|_| "invalid_blocklist".to_owned())?;
    if !blocklist.allows(&identity) {
        return Err("screen_capture_blocked_application".to_owned());
    }

    let coordinator = app.state::<CaptureCoordinator>();
    let (session_id, monitors) = coordinator
        .begin()
        .map_err(|error| error.code().to_owned())?;
    let locale = match settings.locale {
        crate::settings::types::AppLocale::Ko => "ko",
        crate::settings::types::AppLocale::En => "en",
    };
    for monitor in &monitors {
        let label = overlay_label(&monitor.id);
        if let Some(existing) = app.get_webview_window(&label) {
            let _ = existing.close();
        }
        let query = format!(
            "index.html?captureOverlay=1&session={session_id}&monitor={}&locale={locale}",
            percent_encode(&monitor.id)
        );
        let built = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(query.into()))
            .title("SmartCAT Capture")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .build();
        let Ok(window) = built else {
            let _ = coordinator.cancel(session_id);
            close_overlays(&app);
            return Err("capture_overlay_unavailable".to_owned());
        };
        if window
            .set_position(tauri::PhysicalPosition::new(
                monitor.physical_bounds.x,
                monitor.physical_bounds.y,
            ))
            .is_err()
            || window
                .set_size(tauri::PhysicalSize::new(
                    monitor.physical_bounds.width,
                    monitor.physical_bounds.height,
                ))
                .is_err()
        {
            let _ = coordinator.cancel(session_id);
            close_overlays(&app);
            return Err("capture_overlay_unavailable".to_owned());
        }
        let _ = window.set_focus();
    }
    Ok(StartCaptureResult {
        session_id,
        monitors,
    })
}

#[tauri::command]
pub fn get_capture_overlay(
    state: tauri::State<'_, CaptureCoordinator>,
    session_id: Uuid,
    monitor_id: String,
) -> Result<OverlayDescriptor, String> {
    state
        .overlay(session_id, &monitor_id)
        .map_err(|error| error.code().to_owned())
}

#[tauri::command]
pub fn update_screen_selection<R: Runtime>(
    app: tauri::AppHandle<R>,
    session_id: Uuid,
    selection: CaptureSelection,
) -> Result<(), String> {
    app.emit(
        "capture-selection-updated",
        serde_json::json!({ "sessionId": session_id, "selection": selection }),
    )
    .map_err(|_| "capture_overlay_unavailable".to_owned())
}

#[tauri::command]
pub fn complete_screen_capture<R: Runtime>(
    app: tauri::AppHandle<R>,
    session_id: Uuid,
    selection: CaptureSelection,
) -> Result<CaptureJobResult, String> {
    let root = capture_root(&app)?;
    let decoded = app
        .state::<CaptureCoordinator>()
        .complete(session_id, selection, &root)
        .map_err(|error| error.code().to_owned())?;
    close_overlays(&app);
    let result = CaptureJobResult {
        job_id: Uuid::new_v4(),
        status: CaptureJobStatus::SourceReady,
        image_width: decoded.width,
        image_height: decoded.height,
        ocr: None,
        translated_blocks: Vec::new(),
        warnings: Vec::new(),
    };
    let _ = app.emit("capture-source-ready", &result);
    Ok(result)
}

#[tauri::command]
pub fn cancel_screen_capture<R: Runtime>(
    app: tauri::AppHandle<R>,
    session_id: Uuid,
) -> Result<(), String> {
    let result = app
        .state::<CaptureCoordinator>()
        .cancel(session_id)
        .map_err(|error| error.code().to_owned());
    close_overlays(&app);
    result
}

#[tauri::command]
pub fn choose_image<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<CaptureJobResult>, String> {
    let chosen = app
        .dialog()
        .file()
        .add_filter(
            "Images",
            &["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp"],
        )
        .blocking_pick_file();
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let path = chosen
        .into_path()
        .map_err(|_| "unsupported_image_path".to_owned())?;
    let decoded = crate::capture::ImageInput::open_read_only(path, capture_root(&app)?)
        .map_err(|error| error.code().to_owned())?;
    let result = CaptureJobResult {
        job_id: Uuid::new_v4(),
        status: CaptureJobStatus::SourceReady,
        image_width: decoded.width,
        image_height: decoded.height,
        ocr: None,
        translated_blocks: Vec::new(),
        warnings: Vec::new(),
    };
    let _ = app.emit("capture-source-ready", &result);
    Ok(Some(result))
}

fn capture_root<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("capture-inputs"))
        .map_err(|_| "capture_storage_unavailable".to_owned())
}

fn close_overlays<R: Runtime>(app: &tauri::AppHandle<R>) {
    for (_, window) in app.webview_windows() {
        if window.label().starts_with(OVERLAY_PREFIX) {
            let _ = window.close();
        }
    }
}

fn overlay_label(id: &str) -> String {
    let safe: String = id
        .chars()
        .filter(|value| value.is_ascii_alphanumeric() || *value == '-')
        .take(80)
        .collect();
    format!("{OVERLAY_PREFIX}{safe}")
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
