use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use base64::Engine as _;
use serde::Serialize;
use tauri::{Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    capture::{
        layout::{group_lines, TextBlock},
        render::RenderEngine,
        store::CaptureJob,
        translate::{fallback_by_order, parse_structured_translation, structured_source},
        CaptureCoordinator, CaptureJobResult, CaptureJobStatus, CaptureJobStore, CaptureSelection,
        MonitorInfo, NativeOcrEngine, OcrEngine, OverlayDescriptor,
    },
    commands::settings::open_store,
    commands::translation::{TranslationEvent, TranslationEventSink},
    core::{
        diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome, JobKind},
        types::{GlossaryMapping, TranslationMode, TranslationModel, TranslationRequest},
    },
    hotkeys::{Blocklist, ForegroundAppProvider},
    storage::CleanupService,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProgress {
    pub job_id: Uuid,
    pub stage: &'static str,
    pub percent: u8,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureSessionEnded<'a> {
    session_id: Uuid,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureSourceReady<'a> {
    #[serde(flatten)]
    result: &'a CaptureJobResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    capture_session_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionTeardownPolicy {
    destroy_requested_session_windows: bool,
    emit_terminal_event: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureFailurePolicy {
    KeepSessionForRetry,
    TerminateSession,
}

impl CaptureFailurePolicy {
    fn for_reason(reason: &str) -> Self {
        if reason == "invalid_capture_selection" {
            Self::KeepSessionForRetry
        } else {
            Self::TerminateSession
        }
    }
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
    let mut windows = Vec::with_capacity(monitors.len());
    for monitor in &monitors {
        let label = overlay_label(session_id, &monitor.id);
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
            .visible(false)
            .build();
        let Ok(window) = built else {
            cancel_capture_setup(&app, session_id);
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
            cancel_capture_setup(&app, session_id);
            return Err("capture_overlay_unavailable".to_owned());
        }
        windows.push(window);
    }
    for window in &windows {
        if window.show().is_err() {
            cancel_capture_setup(&app, session_id);
            return Err("capture_overlay_unavailable".to_owned());
        }
    }
    for window in &windows {
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
    let job_id = Uuid::new_v4();
    let cleanup = app.state::<CleanupService>();
    let root = match cleanup.create_job_root(&job_id.simple().to_string()) {
        Ok(root) => root,
        Err(_) => {
            return Err(fail_capture_session(
                &app,
                session_id,
                "capture_storage_unavailable",
            ));
        }
    };
    let decoded = match app
        .state::<CaptureCoordinator>()
        .complete(session_id, selection, &root)
    {
        Ok(decoded) => decoded,
        Err(error) => {
            let _ = cleanup.on_job_cancel(&job_id.simple().to_string());
            crate::commands::history::emit_privacy_status(&app);
            let safe_reason = safe_capture_session_reason(error.code());
            return match CaptureFailurePolicy::for_reason(safe_reason) {
                CaptureFailurePolicy::KeepSessionForRetry => Err(safe_reason.to_owned()),
                CaptureFailurePolicy::TerminateSession => {
                    Err(fail_capture_session(&app, session_id, safe_reason))
                }
            };
        }
    };
    destroy_overlays_for_session(&app, session_id);
    let result = source_result(&decoded, job_id);
    app.state::<CaptureJobStore>().insert(CaptureJob {
        source: decoded,
        result: result.clone(),
        rendered: None,
        cancelled: Arc::new(AtomicBool::new(false)),
        translation_job: None,
    });
    let _ = app.emit(
        "capture-source-ready",
        CaptureSourceReady {
            result: &result,
            capture_session_id: Some(session_id),
        },
    );
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
    apply_session_teardown(
        &app,
        session_id,
        session_teardown_policy(result.is_ok()),
        "cancelled",
        None,
    );
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
    let job_id = Uuid::new_v4();
    let cleanup = app.state::<CleanupService>();
    let root = cleanup
        .create_job_root(&job_id.simple().to_string())
        .map_err(|_| "capture_storage_unavailable".to_owned())?;
    let decoded = match crate::capture::ImageInput::open_read_only(path, root) {
        Ok(decoded) => decoded,
        Err(error) => {
            let _ = cleanup.on_job_cancel(&job_id.simple().to_string());
            crate::commands::history::emit_privacy_status(&app);
            return Err(error.code().to_owned());
        }
    };
    let result = source_result(&decoded, job_id);
    app.state::<CaptureJobStore>().insert(CaptureJob {
        source: decoded,
        result: result.clone(),
        rendered: None,
        cancelled: Arc::new(AtomicBool::new(false)),
        translation_job: None,
    });
    let _ = app.emit(
        "capture-source-ready",
        CaptureSourceReady {
            result: &result,
            capture_session_id: None,
        },
    );
    Ok(Some(result))
}

fn source_result(decoded: &crate::capture::DecodedImage, job_id: Uuid) -> CaptureJobResult {
    CaptureJobResult {
        job_id,
        status: CaptureJobStatus::SourceReady,
        image_width: decoded.width,
        image_height: decoded.height,
        ocr: None,
        translated_blocks: Vec::new(),
        warnings: Vec::new(),
        source_preview: encode_preview(decoded.width, decoded.height, &decoded.rgba).ok(),
        translated_preview: None,
    }
}

#[tauri::command]
pub async fn translate_image(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    jobs: tauri::State<'_, CaptureJobStore>,
    job_id: Uuid,
    language_hints: Vec<String>,
    secret: bool,
) -> Result<CaptureJobResult, String> {
    DiagnosticEvent::new(
        DiagnosticEventName::JobLifecycle,
        DiagnosticOutcome::Started,
    )
    .with_job_kind(JobKind::Capture)
    .with_stage("ocr")
    .emit();
    let (source, cancelled) = jobs
        .with(job_id, |job| (job.source.clone(), job.cancelled.clone()))
        .ok_or_else(|| "capture_job_not_found".to_owned())?;
    cancelled.store(false, Ordering::Release);
    emit_progress(&app, job_id, "ocr", 15);
    let ocr = NativeOcrEngine::default()
        .recognize(&source, &language_hints)
        .await
        .map_err(|error| error.code().to_owned())?;
    if cancelled.load(Ordering::Acquire) {
        return Err("capture_cancelled".to_owned());
    }
    let blocks = group_lines(&ocr);
    let settings = open_store(&app)?
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    let saved = settings
        .default_profile()
        .cloned()
        .ok_or_else(|| "invalid_default_profile".to_owned())?;
    emit_progress(&app, job_id, "translate", 45);
    let translated =
        translate_blocks(&state, &jobs, job_id, &blocks, &settings, &saved, secret).await?;
    if cancelled.load(Ordering::Acquire) {
        return Err("capture_cancelled".to_owned());
    }
    emit_progress(&app, job_id, "render", 78);
    let rendered = RenderEngine::render(&source, &translated)
        .map_err(|_| "capture_render_failed".to_owned())?;
    let translated_preview = encode_preview(rendered.width, rendered.height, &rendered.rgba)
        .map_err(|_| "capture_preview_failed".to_owned())?;
    if let Some(history) = app.try_state::<Arc<crate::storage::HistoryStore>>() {
        let _ = history.save(crate::storage::NewHistoryRecord {
            kind: "capture".to_owned(),
            source_language: saved.profile.source_language.clone(),
            target_language: saved.profile.target_language.clone(),
            source: blocks
                .iter()
                .map(|value| value.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            result: translated
                .iter()
                .map(|value| value.translated_text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            display_name: None,
            warning_count: rendered.warnings.len() as u32,
            secret,
        });
    }
    let result = CaptureJobResult {
        job_id,
        status: CaptureJobStatus::Rendered,
        image_width: source.width,
        image_height: source.height,
        ocr: Some(ocr),
        translated_blocks: translated,
        warnings: rendered.warnings.clone(),
        source_preview: encode_preview(source.width, source.height, &source.rgba).ok(),
        translated_preview: Some(translated_preview),
    };
    jobs.with_mut(job_id, |job| {
        job.result = result.clone();
        job.rendered = Some(rendered);
        job.translation_job = None;
    })
    .ok_or_else(|| "capture_job_not_found".to_owned())?;
    cleanup_capture_source(&app, job_id);
    emit_progress(&app, job_id, "complete", 100);
    DiagnosticEvent::new(
        DiagnosticEventName::JobLifecycle,
        DiagnosticOutcome::Succeeded,
    )
    .with_job_kind(JobKind::Capture)
    .with_stage("completed")
    .emit();
    Ok(result)
}

async fn translate_blocks(
    state: &AppState,
    jobs: &CaptureJobStore,
    job_id: Uuid,
    blocks: &[TextBlock],
    settings: &crate::settings::types::AppSettings,
    saved: &crate::settings::types::SavedProfile,
    secret: bool,
) -> Result<Vec<crate::capture::TranslatedBlock>, String> {
    if blocks.is_empty() {
        return Ok(Vec::new());
    }
    let source =
        structured_source(blocks).map_err(|_| "capture_translation_input_invalid".to_owned())?;
    let source_lang = saved.profile.source_language.as_deref().unwrap_or("");
    let glossary = settings
        .glossary
        .iter()
        .filter(|entry| {
            !entry.protect_only
                && (source_lang.is_empty() || entry.source_language == source_lang)
                && entry.target_language == saved.profile.target_language
        })
        .map(|entry| GlossaryMapping {
            source_term: entry.source_term.clone(),
            target_term: entry.target_term.clone(),
        })
        .collect();
    let mut profile = saved.profile.clone();
    for token in blocks
        .iter()
        .flat_map(|block| block.text.split_whitespace())
    {
        let candidate = token
            .trim_matches(|value: char| matches!(value, '(' | ')' | '[' | ']' | ',' | '.' | ';'));
        if (candidate.starts_with("https://") || candidate.starts_with("http://"))
            && candidate.len() <= 1_024
            && !profile.protected_terms.iter().any(|term| term == candidate)
        {
            profile.protected_terms.push(candidate.to_owned());
        }
    }
    let request = TranslationRequest {
        text: source,
        profile,
        field: saved.field,
        glossary,
        mode: TranslationMode::Translate,
        secret,
        model: TranslationModel::Automatic,
    };
    let manager = state
        .translation_jobs()
        .await
        .ok_or_else(|| "translation_service_unavailable".to_owned())?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sink = Arc::new(CaptureTranslationSink(Mutex::new(Some(sender))));
    let translation_job = manager
        .start(format!("capture:{job_id}"), request, sink)
        .await
        .map_err(|_| "capture_translation_failed".to_owned())?;
    jobs.with_mut(job_id, |job| job.translation_job = Some(translation_job));
    let output = tokio::time::timeout(std::time::Duration::from_secs(125), receiver)
        .await
        .map_err(|_| "capture_translation_timed_out".to_owned())?
        .map_err(|_| "capture_translation_failed".to_owned())??;
    Ok(parse_structured_translation(blocks, &output)
        .unwrap_or_else(|_| fallback_by_order(blocks, &output)))
}

struct CaptureTranslationSink(Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>);
impl TranslationEventSink for CaptureTranslationSink {
    fn emit(&self, event: TranslationEvent) {
        let resolved = match event {
            TranslationEvent::Completed { result, .. } => Some(Ok(result.translated_text)),
            TranslationEvent::Failed { code, .. } => Some(Err(code)),
            _ => None,
        };
        if let Some(resolved) = resolved {
            if let Some(sender) = self.0.lock().unwrap_or_else(|p| p.into_inner()).take() {
                let _ = sender.send(resolved);
            }
        }
    }
}

#[tauri::command]
pub async fn cancel_image_translation(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    jobs: tauri::State<'_, CaptureJobStore>,
    job_id: Uuid,
) -> Result<(), String> {
    if let Some(translation_job) = jobs.cancel(job_id).flatten() {
        if let Some(manager) = state.translation_jobs().await {
            let _ = manager
                .cancel(&format!("capture:{job_id}"), translation_job)
                .await;
        }
    }
    cleanup_capture_source(&app, job_id);
    DiagnosticEvent::new(
        DiagnosticEventName::JobLifecycle,
        DiagnosticOutcome::Cancelled,
    )
    .with_job_kind(JobKind::Capture)
    .emit();
    Ok(())
}

#[tauri::command]
pub fn update_capture_block(
    jobs: tauri::State<'_, CaptureJobStore>,
    job_id: Uuid,
    block_id: Uuid,
    translated_text: String,
    visible: bool,
) -> Result<CaptureJobResult, String> {
    if translated_text.chars().count() > 200_000 {
        return Err("capture_text_too_large".to_owned());
    }
    jobs.with_mut(job_id, |job| {
        let block = job
            .result
            .translated_blocks
            .iter_mut()
            .find(|block| block.id == block_id)
            .ok_or_else(|| "capture_block_not_found".to_owned())?;
        block.translated_text = translated_text;
        block.visible = visible;
        let rendered = RenderEngine::render(&job.source, &job.result.translated_blocks)
            .map_err(|_| "capture_render_failed".to_owned())?;
        job.result.translated_preview =
            encode_preview(rendered.width, rendered.height, &rendered.rgba).ok();
        job.result.warnings = rendered.warnings.clone();
        job.rendered = Some(rendered);
        Ok(job.result.clone())
    })
    .ok_or_else(|| "capture_job_not_found".to_owned())?
}

#[tauri::command]
pub fn export_translated_image(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, CaptureJobStore>,
    job_id: Uuid,
    format: String,
    replace: bool,
) -> Result<Option<String>, String> {
    let extension = match format.as_str() {
        "png" => "png",
        "jpeg" => "jpg",
        "webp" => "webp",
        _ => return Err("unsupported_export_format".to_owned()),
    };
    let Some(path) = app
        .dialog()
        .file()
        .set_file_name(format!("smartcat-translation.{extension}"))
        .add_filter("Translated image", &[extension])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|_| "unsupported_image_path".to_owned())?;
    jobs.with(job_id, |job| {
        job.rendered
            .as_ref()
            .ok_or_else(|| "capture_not_rendered".to_owned())
            .and_then(|rendered| {
                crate::capture::export::export_atomic(rendered, &path, replace)
                    .map(|value| value.to_string_lossy().into_owned())
                    .map_err(|_| "capture_export_failed".to_owned())
            })
    })
    .ok_or_else(|| "capture_job_not_found".to_owned())?
    .map(Some)
}

fn encode_preview(width: u32, height: u32, rgba: &[u8]) -> Result<String, image::ImageError> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| {
        image::ImageError::Limits(image::error::LimitError::from_kind(
            image::error::LimitErrorKind::DimensionError,
        ))
    })?;
    let preview = if width > 1_600 || height > 1_200 {
        image::imageops::thumbnail(&image, 1_600, 1_200)
    } else {
        image
    };
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(preview).write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
    ))
}

fn emit_progress(app: &tauri::AppHandle, job_id: Uuid, stage: &'static str, percent: u8) {
    let _ = app.emit(
        "capture-progress",
        CaptureProgress {
            job_id,
            stage,
            percent,
        },
    );
}

fn cleanup_capture_source<R: Runtime>(app: &tauri::AppHandle<R>, job_id: Uuid) {
    if let Some(cleanup) = app.try_state::<CleanupService>() {
        let _ = cleanup.on_job_complete(&job_id.simple().to_string());
    }
    crate::commands::history::emit_privacy_status(app);
}

fn destroy_overlays_for_session<R: Runtime>(app: &tauri::AppHandle<R>, session_id: Uuid) {
    for (_, window) in app.webview_windows() {
        if is_overlay_label_for_session(window.label(), session_id) {
            let _ = window.destroy();
        }
    }
}

fn cancel_capture_setup<R: Runtime>(app: &tauri::AppHandle<R>, session_id: Uuid) {
    let _ = app.state::<CaptureCoordinator>().cancel(session_id);
    destroy_overlays_for_session(app, session_id);
}

fn session_teardown_policy(cancelled_exact_session: bool) -> SessionTeardownPolicy {
    SessionTeardownPolicy {
        destroy_requested_session_windows: true,
        emit_terminal_event: cancelled_exact_session,
    }
}

fn apply_session_teardown<R: Runtime>(
    app: &tauri::AppHandle<R>,
    session_id: Uuid,
    policy: SessionTeardownPolicy,
    status: &str,
    reason: Option<&str>,
) {
    if policy.destroy_requested_session_windows {
        destroy_overlays_for_session(app, session_id);
    }
    if policy.emit_terminal_event {
        emit_capture_session_ended(app, session_id, status, reason);
    }
}

fn fail_capture_session<R: Runtime>(
    app: &tauri::AppHandle<R>,
    session_id: Uuid,
    reason: &str,
) -> String {
    let safe_reason = safe_capture_session_reason(reason);
    let cancelled = app.state::<CaptureCoordinator>().cancel(session_id).is_ok();
    apply_session_teardown(
        app,
        session_id,
        session_teardown_policy(cancelled),
        "failed",
        Some(safe_reason),
    );
    safe_reason.to_owned()
}

pub(crate) fn handle_capture_overlay_close<R: Runtime>(
    app: &tauri::AppHandle<R>,
    label: &str,
) -> bool {
    let Some(session_id) = overlay_session_id(label) else {
        return false;
    };
    let cancelled = app.state::<CaptureCoordinator>().cancel(session_id).is_ok();
    apply_session_teardown(
        app,
        session_id,
        session_teardown_policy(cancelled),
        "cancelled",
        None,
    );
    true
}

pub(crate) fn is_capture_overlay_window(label: &str) -> bool {
    overlay_session_id(label).is_some()
}

fn emit_capture_session_ended<R: Runtime>(
    app: &tauri::AppHandle<R>,
    session_id: Uuid,
    status: &str,
    reason: Option<&str>,
) {
    let _ = app.emit(
        "capture-session-ended",
        CaptureSessionEnded {
            session_id,
            status,
            reason,
        },
    );
}

fn safe_capture_session_reason(reason: &str) -> &str {
    if reason == "invalid_capture_selection" {
        return reason;
    }
    let suffix = reason
        .strip_prefix("screen_capture_")
        .or_else(|| reason.strip_prefix("capture_"));
    match suffix {
        Some(value)
            if !value.is_empty()
                && value.len() <= 48
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_') =>
        {
            reason
        }
        _ => "screen_capture_completion_failed",
    }
}

fn overlay_session_id(label: &str) -> Option<Uuid> {
    let suffix = label.strip_prefix(OVERLAY_PREFIX)?;
    let (session, monitor) = suffix.split_once('-')?;
    if session.len() != 32
        || monitor.len() != 32
        || !monitor.bytes().all(|value| value.is_ascii_hexdigit())
    {
        return None;
    }
    Uuid::parse_str(session).ok()
}

fn is_overlay_label_for_session(label: &str, session_id: Uuid) -> bool {
    overlay_session_id(label) == Some(session_id)
}

fn overlay_label(session_id: Uuid, monitor_id: &str) -> String {
    let monitor_key = Uuid::new_v5(&session_id, monitor_id.as_bytes());
    format!(
        "{OVERLAY_PREFIX}{}-{}",
        session_id.simple(),
        monitor_key.simple()
    )
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

#[cfg(test)]
mod tests {
    use super::{
        is_overlay_label_for_session, overlay_label, overlay_session_id,
        safe_capture_session_reason, session_teardown_policy, CaptureFailurePolicy,
        SessionTeardownPolicy,
    };
    use uuid::Uuid;

    #[test]
    fn capture_session_reason_allows_only_safe_capture_codes() {
        assert_eq!(
            safe_capture_session_reason("screen_capture_failed"),
            "screen_capture_failed"
        );
        assert_eq!(
            safe_capture_session_reason("capture_storage_unavailable"),
            "capture_storage_unavailable"
        );
        assert_eq!(
            safe_capture_session_reason("invalid_capture_selection"),
            "invalid_capture_selection"
        );
        assert_eq!(
            safe_capture_session_reason("secret at C:\\capture.png"),
            "screen_capture_completion_failed"
        );
    }

    #[test]
    fn overlay_labels_round_trip_and_match_only_their_session() {
        let session_a = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
        let session_b = Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        let label = overlay_label(session_a, "display:/primary");

        assert_eq!(overlay_session_id(&label), Some(session_a));
        assert!(is_overlay_label_for_session(&label, session_a));
        assert!(!is_overlay_label_for_session(&label, session_b));
        assert_eq!(overlay_session_id("capture-overlay-primary"), None);
        assert_eq!(overlay_session_id("main"), None);
        assert_eq!(overlay_session_id("quick-popup"), None);
    }

    #[test]
    fn overlay_labels_are_unique_across_sessions_and_monitors() {
        let session_a = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
        let session_b = Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();

        assert_ne!(
            overlay_label(session_a, "display/1"),
            overlay_label(session_a, "display:1")
        );
        assert_ne!(
            overlay_label(session_a, "display/1"),
            overlay_label(session_b, "display/1")
        );
    }

    #[test]
    fn stale_session_teardown_destroys_only_requested_windows_without_terminal_event() {
        assert_eq!(
            session_teardown_policy(false),
            SessionTeardownPolicy {
                destroy_requested_session_windows: true,
                emit_terminal_event: false,
            }
        );
        assert_eq!(
            session_teardown_policy(true),
            SessionTeardownPolicy {
                destroy_requested_session_windows: true,
                emit_terminal_event: true,
            }
        );
    }

    #[test]
    fn invalid_selection_is_retryable_but_other_capture_failures_terminate_the_session() {
        assert_eq!(
            CaptureFailurePolicy::for_reason("invalid_capture_selection"),
            CaptureFailurePolicy::KeepSessionForRetry
        );
        assert_eq!(
            CaptureFailurePolicy::for_reason("capture_storage_unavailable"),
            CaptureFailurePolicy::TerminateSession
        );
    }
}
