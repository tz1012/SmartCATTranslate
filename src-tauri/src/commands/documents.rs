use serde::Serialize;
use std::sync::{atomic::Ordering, Arc, Mutex};
use tauri::{Emitter, Manager};
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
        DocumentCheckpoint, DocumentJobStore, DocumentManifest, DocumentOptions, DocumentReport,
        DocumentStage, PdfRasterSpool,
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenDocument {
    pub source_path: String,
    pub manifest: DocumentManifest,
}
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum DocumentJobEvent {
    Progress {
        job_id: Uuid,
        checkpoint: DocumentCheckpoint,
    },
    Warning {
        job_id: Uuid,
        warning: crate::documents::DocumentWarning,
    },
    Completed {
        job_id: Uuid,
        report: DocumentReport,
    },
    Failed {
        job_id: Uuid,
        code: String,
        location: Option<String>,
    },
    RetentionRequested {
        job_id: Uuid,
    },
}

#[tauri::command]
pub async fn choose_document(
    app: tauri::AppHandle,
    options: DocumentOptions,
) -> Result<Option<ChosenDocument>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("Documents", &["docx", "pptx", "xlsx", "pdf"])
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
pub fn inspect_document_path(
    source_path: String,
    options: DocumentOptions,
) -> Result<ChosenDocument, String> {
    let path = std::path::Path::new(&source_path);
    if !path.is_file() {
        return Err("document_path_unsupported".into());
    }
    let plan = documents::inspect_document(path, &options).map_err(error_code)?;
    Ok(ChosenDocument {
        source_path,
        manifest: plan.manifest,
    })
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
    let secret = options.secret;
    let cancelled = jobs.begin(job_id);
    let jobs_root = document_jobs_root(&app)?;
    let job_root = jobs_root.join(job_id.simple().to_string());
    std::fs::create_dir_all(&job_root).map_err(|_| "document_io_failed".to_owned())?;
    let outcome = translate_document_inner(
        &app,
        &state,
        &cancelled,
        job_id,
        source_path,
        options,
        &job_root,
    )
    .await;
    jobs.finish(job_id);
    match &outcome {
        Ok(report) => {
            for warning in &report.warnings {
                let _ = app.emit(
                    "document-job",
                    DocumentJobEvent::Warning {
                        job_id,
                        warning: warning.clone(),
                    },
                );
            }
            let _ = app.emit(
                "document-job",
                DocumentJobEvent::Completed {
                    job_id,
                    report: report.clone(),
                },
            );
        }
        Err(code) => {
            let _ = app.emit(
                "document-job",
                DocumentJobEvent::Failed {
                    job_id,
                    code: code.clone(),
                    location: None,
                },
            );
        }
    }
    let retryable = outcome
        .as_ref()
        .err()
        .is_some_and(|code| retryable_document_error(code));
    let retain = if !secret && retryable {
        let _ = app.emit(
            "document-job",
            DocumentJobEvent::RetentionRequested { job_id },
        );
        let mut acknowledged = false;
        for _ in 0..10 {
            if jobs.take_retention_ack(job_id) {
                acknowledged = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        acknowledged
    } else {
        false
    };
    if !retain {
        cleanup_job_root(&jobs_root, &job_root);
    }
    outcome
}

async fn translate_document_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    cancelled: &std::sync::atomic::AtomicBool,
    job_id: Uuid,
    source_path: String,
    options: DocumentOptions,
    job_root: &std::path::Path,
) -> Result<DocumentReport, String> {
    let mut plan = documents::inspect_document(std::path::Path::new(&source_path), &options)
        .map_err(error_code)?;
    if plan.format == crate::documents::DocumentFormat::Pdf {
        crate::documents::pdf::preflight_spool(job_root, plan.manifest.page_count)
            .map_err(error_code)?;
        plan.pdf_spool = Some(PdfRasterSpool {
            root: job_root.to_owned(),
            refs: Default::default(),
        });
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("document_cancelled".into());
    }
    emit(
        app,
        job_id,
        checkpoint(
            &plan,
            DocumentStage::Inspect,
            "inspect:completed",
            1,
            1,
            &[],
        ),
    );
    emit(
        app,
        job_id,
        checkpoint(
            &plan,
            DocumentStage::Extract,
            "extract:completed",
            1,
            1,
            &[],
        ),
    );
    let settings = open_store(app)?
        .load()
        .await
        .map_err(|e| e.code().to_owned())?;
    let saved = options
        .profile_id
        .as_ref()
        .and_then(|id| settings.profile(*id))
        .or_else(|| settings.default_profile())
        .cloned()
        .ok_or_else(|| "invalid_default_profile".to_owned())?;
    if plan.format == crate::documents::DocumentFormat::Pdf {
        let hints = options
            .source_language
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        crate::documents::pdf::append_native_ocr(
            &mut plan,
            &hints,
            options.pdf_force_ocr,
            job_id,
            cancelled,
            &|value| emit(app, job_id, value.clone()),
        )
        .await
        .map_err(error_code)?;
    }
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
        let translated_refs = translated
            .iter()
            .map(|value: &crate::documents::TranslatedSegment| format!("segment:{}", value.id))
            .collect::<Vec<_>>();
        emit(
            app,
            job_id,
            checkpoint(
                &plan,
                DocumentStage::Translate,
                &format!("batch:{index}"),
                index,
                total,
                &translated_refs,
            ),
        );
        let prepared_batch =
            prepare_batch(segment_batch, &protected, job_id).map_err(error_code)?;
        let mut profile = saved.profile.clone();
        profile.target_language = options.target_language.clone();
        profile.source_language = options.source_language.clone().or(profile.source_language);
        if let Some(quality) = &options.quality {
            profile.quality = quality.clone();
        }
        profile.protected_terms = protected.clone();
        let request = TranslationRequest {
            text: prepared_batch.source.clone(),
            profile,
            field: saved.field,
            glossary: glossary.clone(),
            mode: TranslationMode::Translate,
            secret: false,
            model: options
                .model
                .as_ref()
                .filter(|value| {
                    !value.is_empty()
                        && value.len() <= 128
                        && value
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                })
                .map(|value| TranslationModel::Specific(value.clone()))
                .unwrap_or(TranslationModel::Automatic),
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
        let translated_refs = translated
            .iter()
            .map(|value| format!("segment:{}", value.id))
            .collect::<Vec<_>>();
        emit(
            app,
            job_id,
            checkpoint(
                &plan,
                DocumentStage::Translate,
                &format!("batch:{index}:completed"),
                index + 1,
                total,
                &translated_refs,
            ),
        );
    }
    let report = documents::pipeline::rebuild_document_checked(
        &plan,
        &translated,
        &options,
        job_id,
        cancelled,
        &|value| emit(app, job_id, value.clone()),
    )
    .map_err(error_code)?;
    if plan.format == crate::documents::DocumentFormat::Pdf {
        let inspection = crate::documents::pdf::inspect(&plan.source, options.pdf_force_ocr)
            .map_err(error_code)?;
        if let Err(error) = crate::documents::pdf::validate_rendered_output(
            &plan.source,
            std::path::Path::new(&report.output_path),
            &inspection,
            cancelled,
            &|value| emit(app, job_id, value.clone()),
            &plan.manifest.source_hash,
        )
        .await
        {
            let _ = std::fs::remove_file(&report.output_path);
            return Err(error_code(error));
        }
    }
    emit(
        app,
        job_id,
        checkpoint(
            &plan,
            DocumentStage::Completed,
            "completed",
            1,
            1,
            &translated
                .iter()
                .map(|value| format!("segment:{}", value.id))
                .collect::<Vec<_>>(),
        ),
    );
    Ok(report)
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
pub fn acknowledge_document_resume_retention(
    jobs: tauri::State<'_, DocumentJobStore>,
    job_id: Uuid,
) {
    jobs.acknowledge_retention(job_id);
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
#[tauri::command]
pub fn open_document_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let candidate = std::path::Path::new(&path);
    if !candidate.is_file() || crate::documents::DocumentFormat::from_path(candidate).is_none() {
        return Err("document_open_refused".to_owned());
    }
    let parent = candidate
        .parent()
        .ok_or_else(|| "document_open_refused".to_owned())?;
    app.opener()
        .open_path(parent.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|_| "document_open_failed".to_owned())
}
#[tauri::command]
pub fn choose_document_output_directory(app: tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|folder| folder.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}
fn emit(app: &tauri::AppHandle, job_id: Uuid, checkpoint: DocumentCheckpoint) {
    let _ = app.emit(
        "document-job",
        DocumentJobEvent::Progress { job_id, checkpoint },
    );
}

fn checkpoint(
    plan: &documents::DocumentPlan,
    stage: DocumentStage,
    stable_unit_id: &str,
    completed: usize,
    total: usize,
    translated_result_refs: &[String],
) -> DocumentCheckpoint {
    let mut raster_refs = plan
        .pdf_spool
        .as_ref()
        .map(|spool| spool.refs.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    raster_refs.sort();
    DocumentCheckpoint {
        source_fingerprint: plan.manifest.source_hash.clone(),
        stage,
        stable_unit_id: stable_unit_id.to_owned(),
        completed,
        total,
        raster_refs,
        translated_result_refs: translated_result_refs.to_vec(),
    }
}

fn cleanup_job_root(root: &std::path::Path, job_root: &std::path::Path) {
    if job_root.parent() == Some(root)
        && job_root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.len() == 32 && value.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let _ = std::fs::remove_dir_all(job_root);
    }
}

fn document_jobs_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    #[cfg(windows)]
    {
        let preferred = std::path::PathBuf::from(r"D:\SmartCATTranslateData\document-jobs");
        if preferred.parent().is_some_and(std::path::Path::exists)
            && std::fs::create_dir_all(&preferred).is_ok()
        {
            return Ok(preferred);
        }
    }
    app.path()
        .app_data_dir()
        .map(|path| path.join("document-jobs"))
        .map_err(|_| "document_io_failed".to_owned())
}

fn retryable_document_error(code: &str) -> bool {
    matches!(
        code,
        "document_translation_failed"
            | "document_translation_timed_out"
            | "translation_auth_required"
            | "translation_quota_exceeded"
            | "translation_network_failed"
    )
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
        documents::types::DocumentError::PasswordRequired => "document_password_required",
        documents::types::DocumentError::LimitExceeded => "document_limits_exceeded",
        documents::types::DocumentError::OcrUnavailable => "document_ocr_unavailable",
    }
    .into()
}
