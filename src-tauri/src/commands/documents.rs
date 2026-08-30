use serde::Serialize;
use std::sync::{atomic::Ordering, Arc, Mutex};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    commands::{
        settings::open_store,
        translation::{TranslationEvent, TranslationEventSink},
    },
    core::types::{GlossaryMapping, TranslationMode, TranslationModel, TranslationRequest},
    documents::{
        self,
        translate::{batches, finish_batch, prepare_batch},
        DocumentJobStore, DocumentManifest, DocumentOptions, DocumentReport,
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenDocument {
    pub source_path: String,
    pub manifest: DocumentManifest,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentProgress {
    job_id: Uuid,
    stage: &'static str,
    completed: usize,
    total: usize,
}

#[tauri::command]
pub async fn choose_document(
    app: tauri::AppHandle,
    options: DocumentOptions,
) -> Result<Option<ChosenDocument>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("Office documents", &["docx", "pptx", "xlsx"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|_| "document_path_unsupported".to_owned())?;
    let plan = documents::inspect_document(&path, &options).map_err(error_code)?;
    Ok(Some(ChosenDocument {
        source_path: path.to_string_lossy().into_owned(),
        manifest: plan.manifest,
    }))
}

#[tauri::command]
pub async fn translate_document(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    jobs: tauri::State<'_, DocumentJobStore>,
    job_id: Uuid,
    source_path: String,
    options: DocumentOptions,
) -> Result<DocumentReport, String> {
    let cancelled = jobs.begin(job_id);
    let outcome =
        translate_document_inner(&app, &state, &cancelled, job_id, source_path, options).await;
    jobs.finish(job_id);
    outcome
}

async fn translate_document_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    cancelled: &std::sync::atomic::AtomicBool,
    job_id: Uuid,
    source_path: String,
    options: DocumentOptions,
) -> Result<DocumentReport, String> {
    emit(app, job_id, "inspect", 0, 1);
    let plan = documents::inspect_document(std::path::Path::new(&source_path), &options)
        .map_err(error_code)?;
    if cancelled.load(Ordering::Acquire) {
        return Err("document_cancelled".into());
    }
    let settings = open_store(app)?
        .load()
        .await
        .map_err(|e| e.code().to_owned())?;
    let saved = settings
        .default_profile()
        .cloned()
        .ok_or_else(|| "invalid_default_profile".to_owned())?;
    let glossary = settings
        .glossary
        .iter()
        .filter(|e| {
            !e.protect_only
                && e.target_language
                    .eq_ignore_ascii_case(&options.target_language)
        })
        .map(|e| GlossaryMapping {
            source_term: e.source_term.clone(),
            target_term: e.target_term.clone(),
        })
        .collect::<Vec<_>>();
    let mut protected = saved.profile.protected_terms.clone();
    protected.extend(
        settings
            .glossary
            .iter()
            .filter(|e| e.protect_only)
            .map(|e| e.source_term.clone()),
    );
    let all_batches = batches(&plan.segments);
    let total = all_batches.len();
    let mut translated = Vec::with_capacity(plan.segments.len());
    let manager = state
        .translation_jobs()
        .await
        .ok_or_else(|| "translation_service_unavailable".to_owned())?;
    for (index, segment_batch) in all_batches.into_iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err("document_cancelled".into());
        }
        emit(app, job_id, "translate", index, total);
        let prepared_batch =
            prepare_batch(segment_batch, &protected, job_id).map_err(error_code)?;
        let mut profile = saved.profile.clone();
        profile.target_language = options.target_language.clone();
        profile.protected_terms = protected.clone();
        let request = TranslationRequest {
            text: prepared_batch.source.clone(),
            profile,
            field: saved.field,
            glossary: glossary.clone(),
            mode: TranslationMode::Translate,
            secret: false,
            model: TranslationModel::Automatic,
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let sink = Arc::new(DocumentSink(Mutex::new(Some(sender))));
        let owner = format!("document:{job_id}");
        let translation_id = manager
            .start(owner.clone(), request, sink)
            .await
            .map_err(|_| "document_translation_failed".to_owned())?;
        let mut receiver = receiver;
        let started = tokio::time::Instant::now();
        let output = loop {
            if cancelled.load(Ordering::Acquire) {
                let _ = manager.cancel(&owner, translation_id).await;
                return Err("document_cancelled".into());
            }
            if started.elapsed() > std::time::Duration::from_secs(125) {
                let _ = manager.cancel(&owner, translation_id).await;
                return Err("document_translation_timed_out".into());
            }
            tokio::select! {
                result = &mut receiver => break result.map_err(|_| "document_translation_failed".to_owned())??,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        };
        translated.extend(finish_batch(&prepared_batch, &output).map_err(error_code)?);
    }
    emit(app, job_id, "validate", total, total);
    documents::rebuild_document(&plan, &translated, &options, job_id).map_err(error_code)
}

struct DocumentSink(Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>);
impl TranslationEventSink for DocumentSink {
    fn emit(&self, event: TranslationEvent) {
        let value = match event {
            TranslationEvent::Completed { result, .. } => Some(Ok(result.translated_text)),
            TranslationEvent::Failed { code, .. } => Some(Err(code)),
            _ => None,
        };
        if let Some(value) = value {
            if let Some(sender) = self.0.lock().unwrap_or_else(|p| p.into_inner()).take() {
                let _ = sender.send(value);
            }
        }
    }
}

#[tauri::command]
pub fn cancel_document_translation(jobs: tauri::State<'_, DocumentJobStore>, job_id: Uuid) -> bool {
    jobs.cancel(job_id)
}
#[tauri::command]
pub fn open_document_result(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let candidate = std::path::Path::new(&path);
    let name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !candidate.is_file()
        || !name.contains("_번역_")
        || crate::documents::DocumentFormat::from_path(candidate).is_none()
    {
        return Err("document_open_refused".to_owned());
    }
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|_| "document_open_failed".to_owned())
}
fn emit(app: &tauri::AppHandle, job_id: Uuid, stage: &'static str, completed: usize, total: usize) {
    let _ = app.emit(
        "document-progress",
        DocumentProgress {
            job_id,
            stage,
            completed,
            total,
        },
    );
}
fn error_code(error: documents::types::DocumentError) -> String {
    match error {
        documents::types::DocumentError::Unsupported => "document_unsupported",
        documents::types::DocumentError::UnsafePackage => "document_unsafe_or_macro",
        documents::types::DocumentError::InvalidPackage => "document_invalid_package",
        documents::types::DocumentError::SourceChanged => "document_source_changed",
        documents::types::DocumentError::OutputExists => "document_output_exists",
        documents::types::DocumentError::ValidationFailed => "document_validation_failed",
        documents::types::DocumentError::Cancelled => "document_cancelled",
        documents::types::DocumentError::Io => "document_io_failed",
    }
    .into()
}
