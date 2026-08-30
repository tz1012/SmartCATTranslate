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
        DocumentResumeRequest, DocumentRetentionPayload, DocumentStage, PdfRasterSpool,
        TranslatedSegment,
    },
    storage::{CanonicalTranslationOptions, DocumentRecoveryContext},
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
    let resume = resume
        .map(|request| {
            if request.option_hash != recovery_context.option_hash {
                return Err("document_resume_options_changed".to_owned());
            }
            jobs.resolve_resume_record(&request.record_id, &source_path, &request.option_hash)
                .ok_or_else(|| "document_resume_record_unavailable".to_owned())
        })
        .transpose()?;
    let jobs_root = document_jobs_root(&app)?;
    let job_root = jobs_root.join(job_id.simple().to_string());
    if std::fs::create_dir_all(&job_root).is_err() {
        cleanup_job_root(&jobs_root, &job_root);
        return Err("document_io_failed".to_owned());
    }
    set_private_permissions(&jobs_root, true);
    set_private_permissions(&job_root, true);
    let cancelled = jobs.begin(job_id);
    let latest_checkpoint = Arc::new(Mutex::new(None));
    let translated_results = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let outcome = translate_document_inner(
        &app,
        &state,
        &cancelled,
        job_id,
        source_path,
        options,
        resume,
        &job_root,
        &latest_checkpoint,
        &translated_results,
    )
    .await;
    jobs.finish(job_id);
    match &outcome {
        Ok(report) => {
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
    if !secret && retryable {
        if let Some(mut checkpoint) = latest_checkpoint
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            checkpoint.raster_refs.clear();
            let token = Uuid::new_v4().simple().to_string();
            let retained_copy = translated_results
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            jobs.request_retention(
                job_id,
                token,
                checkpoint,
                retained_copy,
                recovery_context.clone(),
            );
        }
    }
    documents::wipe_translated_results(
        &mut translated_results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    cleanup_job_root(&jobs_root, &job_root);
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
    latest_checkpoint: &Arc<Mutex<Option<DocumentCheckpoint>>>,
    retained_results: &Arc<Mutex<std::collections::HashMap<String, TranslatedSegment>>>,
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
        latest_checkpoint,
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
            0,
            &[],
        ),
        latest_checkpoint,
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
            cancelled,
            &|value| emit(app, job_id, value.clone(), latest_checkpoint),
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
            latest_checkpoint,
        );
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
            latest_checkpoint,
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
            latest_checkpoint,
        );
    }
    let report = documents::pipeline::rebuild_document_checked(
        &plan,
        &translated,
        &options,
        job_id,
        cancelled,
        &|value| emit(app, job_id, value.clone(), latest_checkpoint),
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
            &|value| emit(app, job_id, value.clone(), latest_checkpoint),
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
        latest_checkpoint,
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
pub fn cancel_document_translation(
    jobs: tauri::State<'_, Arc<DocumentJobStore>>,
    job_id: Uuid,
) -> bool {
    jobs.cancel(job_id)
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
    let glossary = settings
        .glossary
        .iter()
        .filter(|entry| {
            source_language
                .as_deref()
                .is_none_or(|source| entry.source_language.eq_ignore_ascii_case(source))
                && entry
                    .target_language
                    .eq_ignore_ascii_case(&options.target_language)
        })
        .cloned()
        .collect::<Vec<_>>();
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
        glossary: serde_json::to_value(glossary).map_err(|_| "document_options_invalid")?,
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
        display_name,
        options: options.clone(),
        option_snapshot: snapshot,
        option_hash,
    })
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
    checkpoint: DocumentCheckpoint,
    latest_checkpoint: &Arc<Mutex<Option<DocumentCheckpoint>>>,
) {
    let resumable = checkpoint.stage != DocumentStage::Translate
        || checkpoint.completed_batch_cursor == 0
        || checkpoint.stable_unit_id.ends_with(":completed");
    if resumable {
        *latest_checkpoint.lock().unwrap_or_else(|p| p.into_inner()) = Some(checkpoint.clone());
    }
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

fn cleanup_job_root(root: &std::path::Path, job_root: &std::path::Path) {
    let Ok(root) = root.canonicalize() else {
        return;
    };
    let Ok(job_root) = job_root.canonicalize() else {
        return;
    };
    if job_root.parent() == Some(root.as_path())
        && job_root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.len() == 32 && value.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let _ = std::fs::remove_dir_all(job_root);
    }
}

fn document_jobs_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("document-jobs"))
        .map_err(|_| "document_io_failed".to_owned())
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
fn set_private_permissions(path: &std::path::Path, directory: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(windows)]
fn set_private_permissions(path: &std::path::Path, directory: bool) {
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
