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
    core::{
        diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome, JobKind},
        types::{GlossaryMapping, TranslationMode, TranslationModel, TranslationRequest},
    },
    documents::{
        self,
        translate::{batches, finish_batch, prepare_batch},
        DocumentCheckpoint, DocumentJobStore, DocumentManifest, DocumentOptions, DocumentReport,
        DocumentResumeRequest, DocumentRetentionPayload, DocumentStage, PdfRasterSpool,
        TranslatedSegment,
    },
    storage::{
        CanonicalTranslationOptions, CleanupService, DocumentRecoveryContext, JobStage, JobStore,
    },
};
use zeroize::Zeroizing;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenDocument {
    pub source_path: String,
    pub manifest: DocumentManifest,
}
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum DocumentJobEvent {
    Inspect {
        job_id: Uuid,
        manifest: DocumentManifest,
    },
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
    jobs: tauri::State<'_, Arc<DocumentJobStore>>,
    job_id: Uuid,
    source_path: String,
    options: DocumentOptions,
    resume: Option<DocumentResumeRequest>,
) -> Result<DocumentReport, String> {
    let secret = options.secret;
    let target_language = options.target_language.clone();
    let recovery_context = document_recovery_context(&app, &source_path, &options).await?;
    let resume_record_id = resume.as_ref().map(|value| value.record_id.clone());
    let resume = resume
        .map(|request| {
            if request.option_hash != recovery_context.option_hash {
                return Err("document_resume_options_changed".to_owned());
            }
            jobs.resolve_resume_record(&request.record_id, &source_path, &request.option_hash)
                .ok_or_else(|| "document_resume_record_unavailable".to_owned())
        })
        .transpose()?;
    let cleanup = app.state::<CleanupService>();
    let job_root = cleanup
        .create_job_root(&job_id.simple().to_string())
        .map_err(|_| "document_io_failed".to_owned())?;
    let cancelled = jobs.begin(job_id);
    let translated_results = Arc::new(Mutex::new(std::collections::HashMap::new()));
    DiagnosticEvent::new(
        DiagnosticEventName::JobLifecycle,
        DiagnosticOutcome::Started,
    )
    .with_job_kind(JobKind::Document)
    .with_stage("queued")
    .emit();
    let outcome = translate_document_inner(
        &app,
        &state,
        &cancelled,
        job_id,
        source_path,
        options,
        resume,
        &job_root,
        &translated_results,
        &recovery_context,
        secret,
    )
    .await;
    jobs.finish(job_id);
    let record_id = job_id.simple().to_string();
    let retryable = outcome
        .as_ref()
        .err()
        .is_some_and(|code| retryable_document_error(code));
    if let Some(store) = app.try_state::<Arc<JobStore>>() {
        let terminal = match &outcome {
            Ok(_) => JobStage::Completed,
            Err(code) if code == "document_cancelled" => JobStage::Cancelled,
            Err(_) if retryable => JobStage::Paused,
            Err(_) => JobStage::Failed,
        };
        if store.transition_terminal(&record_id, terminal).is_err() {
            DiagnosticEvent::new(DiagnosticEventName::JobLifecycle, DiagnosticOutcome::Failed)
                .with_job_kind(JobKind::Document)
                .with_error_code("recovery_terminal_write_failed")
                .emit();
        }
        if retryable {
            if let Some(previous) = &resume_record_id {
                let _ = store.delete(previous);
            }
        }
    }
    documents::wipe_translated_results(
        &mut translated_results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    let cleanup_result = cleanup.on_job_complete(&record_id);
    crate::commands::history::emit_privacy_status(&app);
    match &outcome {
        Ok(report) => {
            if let Some(store) = app.try_state::<Arc<JobStore>>() {
                let _ = store.delete(&record_id);
                if let Some(record_id) = &resume_record_id {
                    let _ = store.delete(record_id);
                }
            }
            if let Some(history) = app.try_state::<Arc<crate::storage::HistoryStore>>() {
                let _ = history.save(crate::storage::NewHistoryRecord {
                    kind: "document".to_owned(),
                    source_language: recovery_context.option_snapshot.source_language.clone(),
                    target_language: target_language.clone(),
                    source: String::new(),
                    result: String::new(),
                    display_name: Some(recovery_context.display_name.clone()),
                    warning_count: report.warnings.len() as u32,
                    secret,
                });
            }
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
            DiagnosticEvent::new(
                DiagnosticEventName::JobLifecycle,
                DiagnosticOutcome::Succeeded,
            )
            .with_job_kind(JobKind::Document)
            .with_stage("completed")
            .with_error_code(if cleanup_result.is_err() {
                "temporary_cleanup_pending"
            } else {
                ""
            })
            .emit();
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
            let diagnostic_outcome = if code == "document_cancelled" {
                DiagnosticOutcome::Cancelled
            } else {
                DiagnosticOutcome::Failed
            };
            DiagnosticEvent::new(DiagnosticEventName::JobLifecycle, diagnostic_outcome)
                .with_job_kind(JobKind::Document)
                .with_error_code(code)
                .emit();
        }
    }
    if !retryable && outcome.is_err() {
        if let Some(store) = app.try_state::<Arc<JobStore>>() {
            let _ = store.delete(&record_id);
            if let Some(previous) = &resume_record_id {
                let _ = store.delete(previous);
            }
        }
    }
    let _ = app.emit("recovery-updated", ());
    outcome
}

async fn translate_document_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    cancelled: &std::sync::atomic::AtomicBool,
    job_id: Uuid,
    source_path: String,
    options: DocumentOptions,
    resume: Option<DocumentRetentionPayload>,
    job_root: &std::path::Path,
    retained_results: &Arc<Mutex<std::collections::HashMap<String, TranslatedSegment>>>,
    recovery_context: &DocumentRecoveryContext,
    secret: bool,
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
    let _ = app.emit(
        "document-job",
        DocumentJobEvent::Inspect {
            job_id,
            manifest: plan.manifest.clone(),
        },
    );
    emit(
        app,
        job_id,
        checkpoint(
            &plan,
            DocumentStage::Inspect,
            "inspect:completed",
            1,
            1,
            0,
            &[],
        ),
        retained_results,
        recovery_context,
        secret,
    )
    .map_err(error_code)?;
    emit(
        app,
        job_id,
        checkpoint(
            &plan,
            DocumentStage::Extract,
            "extract:completed",
            1,
            1,
            0,
            &[],
        ),
        retained_results,
        recovery_context,
        secret,
    )
    .map_err(error_code)?;
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
            cancelled,
            &|value| {
                emit(
                    app,
                    job_id,
                    value.clone(),
                    retained_results,
                    recovery_context,
                    secret,
                )
            },
        )
        .await
        .map_err(error_code)?;
    }
    let effective_source_language = options
        .source_language
        .as_deref()
        .or(saved.profile.source_language.as_deref());
    let (glossary, protected) = applied_document_glossary(
        &settings,
        &saved,
        effective_source_language,
        &options.target_language,
    );
    let total = batches(&plan.segments).len();
    let resume_state = resume
        .as_ref()
        .map(|value| {
            documents::pipeline::set_resume_checkpoint(
                &mut plan,
                &value.checkpoint,
                &value.translated_results,
            )
        })
        .transpose()
        .map_err(error_code)?;
    let start_cursor = resume_state
        .as_ref()
        .map(|value| value.batch_cursor)
        .unwrap_or(0);
    if start_cursor > total {
        return Err("document_invalid_package".into());
    }
    let mut translated = Zeroizing::new(
        resume_state
            .map(|value| value.translated)
            .unwrap_or_else(|| Vec::with_capacity(plan.segments.len())),
    );
    remember_translated(retained_results, &translated);
    if start_cursor > 0 {
        let refs = translated_refs(&translated);
        emit(
            app,
            job_id,
            checkpoint(
                &plan,
                DocumentStage::Translate,
                &format!("batch:{}:completed", start_cursor - 1),
                start_cursor,
                total,
                start_cursor,
                &refs,
            ),
            retained_results,
            recovery_context,
            secret,
        )
        .map_err(error_code)?;
    }
    let all_batches = batches(&plan.segments);
    let manager = state
        .translation_jobs()
        .await
        .ok_or_else(|| "translation_service_unavailable".to_owned())?;
    for (index, segment_batch) in all_batches.into_iter().enumerate().skip(start_cursor) {
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
                index,
                &translated_refs,
            ),
            retained_results,
            recovery_context,
            secret,
        )
        .map_err(error_code)?;
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
            secret: options.secret,
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
        let output = Zeroizing::new(loop {
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
        });
        translated.extend(finish_batch(&prepared_batch, &output).map_err(error_code)?);
        remember_translated(retained_results, &translated);
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
                index + 1,
                &translated_refs,
            ),
            retained_results,
            recovery_context,
            secret,
        )
        .map_err(error_code)?;
    }
    let report = documents::pipeline::rebuild_document_checked(
        &plan,
        &translated,
        &options,
        job_id,
        cancelled,
        &|value| {
            emit(
                app,
                job_id,
                value.clone(),
                retained_results,
                recovery_context,
                secret,
            )
        },
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
            &|value| {
                emit(
                    app,
                    job_id,
                    value.clone(),
                    retained_results,
                    recovery_context,
                    secret,
                )
            },
            &plan.manifest.source_hash,
            total,
            &translated_refs(&translated),
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
            total,
            &translated
                .iter()
                .map(|value| format!("segment:{}", value.id))
                .collect::<Vec<_>>(),
        ),
        retained_results,
        recovery_context,
        secret,
    )
    .map_err(error_code)?;
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
pub fn cancel_document_translation(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, Arc<DocumentJobStore>>,
    job_id: Uuid,
) -> bool {
    let cancelled = jobs.cancel(job_id);
    if cancelled {
        if let Some(store) = app.try_state::<Arc<JobStore>>() {
            let record_id = job_id.simple().to_string();
            let _ = store.transition_terminal(&record_id, JobStage::Cancelled);
            let _ = store.delete(&record_id);
        }
        if let Some(cleanup) = app.try_state::<CleanupService>() {
            let _ = cleanup.on_job_cancel(&job_id.simple().to_string());
        }
        crate::commands::history::emit_privacy_status(&app);
        let _ = app.emit("recovery-updated", ());
    }
    cancelled
}

async fn document_recovery_context(
    app: &tauri::AppHandle,
    source_path: &str,
    options: &DocumentOptions,
) -> Result<DocumentRecoveryContext, String> {
    let settings = open_store(app)?
        .load()
        .await
        .map_err(|error| error.code().to_owned())?;
    let profile = options
        .profile_id
        .and_then(|id| settings.profile(id))
        .or_else(|| settings.default_profile())
        .ok_or_else(|| "invalid_default_profile".to_owned())?;
    let source_language = options
        .source_language
        .clone()
        .or_else(|| profile.profile.source_language.clone());
    let (glossary, protected_terms) = applied_document_glossary(
        &settings,
        profile,
        source_language.as_deref(),
        &options.target_language,
    );
    let snapshot = CanonicalTranslationOptions {
        source_language,
        target_language: options.target_language.clone(),
        profile: serde_json::to_value(profile).map_err(|_| "document_options_invalid")?,
        model: serde_json::to_value(options.model.as_ref().map_or_else(
            || serde_json::to_value(&settings.selected_model).unwrap_or(serde_json::Value::Null),
            |value| serde_json::Value::String(value.clone()),
        ))
        .map_err(|_| "document_options_invalid")?,
        quality: serde_json::to_value(options.quality.as_ref().unwrap_or(&profile.profile.quality))
            .map_err(|_| "document_options_invalid")?,
        glossary: serde_json::json!({
            "mappings": glossary,
            "protectedTerms": protected_terms,
        }),
        format_options: serde_json::to_value(options).map_err(|_| "document_options_invalid")?,
    };
    let option_hash = snapshot
        .hash()
        .map_err(|_| "document_options_invalid".to_owned())?;
    let display_name = std::path::Path::new(source_path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("document")
        .to_owned();
    Ok(DocumentRecoveryContext {
        source_path: source_path.to_owned(),
        source_fingerprint: crate::documents::pipeline::hash_bytes(
            &std::fs::read(source_path).map_err(|_| "document_source_changed".to_owned())?,
        ),
        display_name,
        options: options.clone(),
        option_snapshot: snapshot,
        option_hash,
    })
}

fn applied_document_glossary(
    settings: &crate::settings::types::AppSettings,
    saved: &crate::settings::types::SavedProfile,
    source_language: Option<&str>,
    target_language: &str,
) -> (Vec<GlossaryMapping>, Vec<String>) {
    let applicable = settings.glossary.iter().filter(|entry| {
        source_language.is_none_or(|source| entry.source_language.eq_ignore_ascii_case(source))
            && entry.target_language.eq_ignore_ascii_case(target_language)
    });
    let mut mappings = Vec::new();
    let mut protected_terms = saved.profile.protected_terms.clone();
    for entry in applicable {
        if entry.protect_only {
            if !protected_terms
                .iter()
                .any(|value| value == &entry.source_term)
            {
                protected_terms.push(entry.source_term.clone());
            }
        } else {
            mappings.push(GlossaryMapping {
                source_term: entry.source_term.clone(),
                target_term: entry.target_term.clone(),
            });
        }
    }
    (mappings, protected_terms)
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
pub async fn get_document_result_preview(
    output_path: String,
    location: String,
) -> Result<crate::documents::preview::DocumentResultPreview, String> {
    let output = std::path::Path::new(&output_path);
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !output.is_file()
        || !name.contains("_번역_")
        || crate::documents::DocumentFormat::from_path(output).is_none()
    {
        return Err("document_preview_refused".to_owned());
    }
    crate::documents::preview::read_result_preview(output, &location)
        .await
        .map_err(|error| match error {
            documents::types::DocumentError::Unsupported => {
                "document_preview_location_unsupported".to_owned()
            }
            documents::types::DocumentError::PreviewLimitExceeded => {
                "document_preview_limits_exceeded".to_owned()
            }
            _ => "document_preview_failed".to_owned(),
        })
}
#[tauri::command]
pub fn choose_document_output_directory(app: tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|folder| folder.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}
fn emit(
    app: &tauri::AppHandle,
    job_id: Uuid,
    mut checkpoint: DocumentCheckpoint,
    retained_results: &Arc<Mutex<std::collections::HashMap<String, TranslatedSegment>>>,
    recovery_context: &DocumentRecoveryContext,
    secret: bool,
) -> Result<(), documents::types::DocumentError> {
    let resumable = checkpoint.stage != DocumentStage::Translate
        || checkpoint.completed_batch_cursor == 0
        || checkpoint.stable_unit_id.ends_with(":completed");
    if resumable {
        checkpoint.raster_refs.clear();
        let translated_results = retained_results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        app.try_state::<Arc<JobStore>>()
            .ok_or(documents::types::DocumentError::Io)?
            .checkpoint_document(
                job_id,
                recovery_context,
                &DocumentRetentionPayload {
                    checkpoint: checkpoint.clone(),
                    translated_results,
                },
                secret,
            )
            .map_err(|_| documents::types::DocumentError::Io)?;
    }
    let _ = app.emit(
        "document-job",
        DocumentJobEvent::Progress { job_id, checkpoint },
    );
    Ok(())
}

fn checkpoint(
    plan: &documents::DocumentPlan,
    stage: DocumentStage,
    stable_unit_id: &str,
    completed: usize,
    total: usize,
    completed_batch_cursor: usize,
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
        completed_batch_cursor,
        raster_refs,
        translated_result_refs: translated_result_refs.to_vec(),
    }
}

fn translated_refs(translated: &[TranslatedSegment]) -> Vec<String> {
    translated
        .iter()
        .map(|value| format!("segment:{}", value.id))
        .collect()
}

fn remember_translated(
    retained_results: &Arc<Mutex<std::collections::HashMap<String, TranslatedSegment>>>,
    translated: &[TranslatedSegment],
) {
    let mut retained = retained_results.lock().unwrap_or_else(|p| p.into_inner());
    retained.clear();
    retained.extend(
        translated
            .iter()
            .cloned()
            .map(|value| (format!("segment:{}", value.id), value)),
    );
}

#[cfg(unix)]
pub(crate) fn set_private_permissions(path: &std::path::Path, directory: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(windows)]
pub(crate) fn set_private_permissions(path: &std::path::Path, directory: bool) {
    use std::{ffi::OsString, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };

    let descriptor_text = if directory {
        "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    } else {
        "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)"
    };
    let descriptor_text: Vec<u16> = OsString::from(descriptor_text)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return;
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    let dacl_ok =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            != 0
            && present != 0;
    if dacl_ok {
        let mut path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let _ = unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        };
    }
    unsafe {
        LocalFree(descriptor);
    }
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
        documents::types::DocumentError::PreviewLimitExceeded => "document_preview_limits_exceeded",
    }
    .into()
}
