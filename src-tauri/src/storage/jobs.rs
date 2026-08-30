use crate::{
    core::diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome, JobKind},
    documents::{
        DocumentCheckpoint, DocumentJobStore, DocumentOptions, DocumentResumeBackend,
        DocumentRetentionNotice, DocumentRetentionPayload, DocumentStage, TranslatedSegment,
    },
    storage::{CryptoBox, CryptoError, StorageDatabase},
};
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    Queued,
    Extract,
    Translate,
    Rebuild,
    Validate,
    Save,
    Completed,
    Paused,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCheckpoint {
    pub stage: JobStage,
    pub previous_active_stage: Option<JobStage>,
    pub completed_unit_ids: Vec<String>,
}

impl JobCheckpoint {
    pub fn transition(&mut self, next: JobStage) -> Result<(), JobError> {
        let active = matches!(
            self.stage,
            JobStage::Queued
                | JobStage::Extract
                | JobStage::Translate
                | JobStage::Rebuild
                | JobStage::Validate
                | JobStage::Save
        );
        let sequential = matches!(
            (self.stage, next),
            (JobStage::Queued, JobStage::Extract)
                | (JobStage::Extract, JobStage::Translate)
                | (JobStage::Translate, JobStage::Rebuild)
                | (JobStage::Rebuild, JobStage::Validate)
                | (JobStage::Validate, JobStage::Save)
                | (JobStage::Save, JobStage::Completed)
        );
        if sequential || (active && matches!(next, JobStage::Cancelled | JobStage::Failed)) {
            self.previous_active_stage = None;
            self.stage = next;
            return Ok(());
        }
        if active && next == JobStage::Paused {
            self.previous_active_stage = Some(self.stage);
            self.stage = next;
            return Ok(());
        }
        if self.stage == JobStage::Paused && self.previous_active_stage == Some(next) {
            self.stage = next;
            self.previous_active_stage = None;
            return Ok(());
        }
        Err(JobError::InvalidTransition)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalTranslationOptions {
    pub source_language: Option<String>,
    pub target_language: String,
    pub profile: serde_json::Value,
    pub model: serde_json::Value,
    pub quality: serde_json::Value,
    pub glossary: serde_json::Value,
    pub format_options: serde_json::Value,
}

impl CanonicalTranslationOptions {
    pub fn hash(&self) -> Result<String, JobError> {
        let bytes = serde_json::to_vec(self).map_err(|_| JobError::Invalid)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Clone, Debug)]
pub struct DocumentRecoveryContext {
    pub source_path: String,
    pub source_fingerprint: String,
    pub display_name: String,
    pub options: DocumentOptions,
    pub option_snapshot: CanonicalTranslationOptions,
    pub option_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DocumentRecoveryMetadata {
    source_path: String,
    options: DocumentOptions,
    option_snapshot: CanonicalTranslationOptions,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DocumentRecoveryPayload {
    #[serde(default = "initial_job_state")]
    job_state: JobCheckpoint,
    checkpoint: DocumentCheckpoint,
    translated_results: HashMap<String, TranslatedSegment>,
}

impl Drop for DocumentRecoveryPayload {
    fn drop(&mut self) {
        crate::documents::wipe_translated_results(&mut self.translated_results);
    }
}

struct SecretDocumentRecovery {
    created_at: String,
    context: DocumentRecoveryContext,
    payload: DocumentRecoveryPayload,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableJob {
    pub record_id: String,
    pub display_name: String,
    pub kind: String,
    pub stage: String,
    pub completed: usize,
    pub total: usize,
    pub created_at: String,
    pub can_resume: bool,
    pub disabled_reason: Option<String>,
    pub secret: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedDocumentRecovery {
    pub record_id: String,
    pub source_path: String,
    pub options: DocumentOptions,
    pub option_hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("invalid job transition")]
    InvalidTransition,
    #[error("recovery database unavailable")]
    Database(#[from] rusqlite::Error),
    #[error("recovery authentication failed")]
    Crypto(#[from] CryptoError),
    #[error("recovery record is invalid")]
    Invalid,
    #[error("source or translation settings changed")]
    ResumeMismatch,
}

#[derive(Clone)]
pub struct JobStore {
    database: StorageDatabase,
    crypto: Arc<CryptoBox>,
    secret: Arc<Mutex<HashMap<String, SecretDocumentRecovery>>>,
}

impl JobStore {
    pub fn new(database: StorageDatabase, crypto: Arc<CryptoBox>) -> Self {
        Self {
            database,
            crypto,
            secret: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn checkpoint_document(
        &self,
        job_id: Uuid,
        context: &DocumentRecoveryContext,
        payload: &DocumentRetentionPayload,
        secret: bool,
    ) -> Result<Option<String>, JobError> {
        let record_id = job_id.simple().to_string();
        if payload.checkpoint.source_fingerprint != context.source_fingerprint
            || payload.checkpoint.source_fingerprint.len() != 64
        {
            return Err(JobError::ResumeMismatch);
        }
        if context.option_snapshot.hash()? != context.option_hash || context.option_hash.len() != 64
        {
            return Err(JobError::Invalid);
        }
        if secret {
            let mut values = self.secret.lock().unwrap_or_else(|p| p.into_inner());
            let mut job_state = values
                .get(&record_id)
                .map(|value| value.payload.job_state.clone())
                .unwrap_or_else(initial_job_state);
            advance_job_state(&mut job_state, payload.checkpoint.stage)?;
            remember_completed_unit(&mut job_state, &payload.checkpoint);
            let created_at = values
                .get(&record_id)
                .map(|value| value.created_at.clone())
                .unwrap_or_else(|| Utc::now().to_rfc3339());
            values.insert(
                record_id.clone(),
                SecretDocumentRecovery {
                    created_at,
                    context: context.clone(),
                    payload: DocumentRecoveryPayload {
                        job_state,
                        checkpoint: payload.checkpoint.clone(),
                        translated_results: payload.translated_results.clone(),
                    },
                },
            );
            diagnostic_checkpoint(&payload.checkpoint, DiagnosticOutcome::Succeeded);
            return Ok(None);
        }
        let metadata = DocumentRecoveryMetadata {
            source_path: context.source_path.clone(),
            options: context.options.clone(),
            option_snapshot: context.option_snapshot.clone(),
        };
        let mut connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let existing_blob = connection
            .query_row(
                "SELECT payload_blob FROM recovery_jobs WHERE id=?1",
                [&record_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let mut job_state = existing_blob
            .map(|blob| {
                self.crypto
                    .open_json::<DocumentRecoveryPayload>(&blob, &aad(&record_id, "payload"))
            })
            .transpose()?
            .map(|value| value.job_state.clone())
            .unwrap_or_else(initial_job_state);
        advance_job_state(&mut job_state, payload.checkpoint.stage)?;
        remember_completed_unit(&mut job_state, &payload.checkpoint);
        let serializable = DocumentRecoveryPayload {
            job_state: job_state.clone(),
            checkpoint: payload.checkpoint.clone(),
            translated_results: payload.translated_results.clone(),
        };
        let display = self
            .crypto
            .seal_json(&context.display_name, &aad(&record_id, "display_name"))?;
        let metadata = self
            .crypto
            .seal_json(&metadata, &aad(&record_id, "metadata"))?;
        let payload_blob = self
            .crypto
            .seal_json(&serializable, &aad(&record_id, "payload"))?;
        let created_at = Utc::now();
        let checkpoint = &payload.checkpoint;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO recovery_jobs (id,created_at,expires_at,kind,stage,completed,total,source_fingerprint,option_hash,display_name_blob,metadata_blob,payload_blob) VALUES (?1,?2,?3,'document',?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET expires_at=excluded.expires_at,stage=excluded.stage,completed=excluded.completed,total=excluded.total,source_fingerprint=excluded.source_fingerprint,option_hash=excluded.option_hash,display_name_blob=excluded.display_name_blob,metadata_blob=excluded.metadata_blob,payload_blob=excluded.payload_blob",
            params![record_id,created_at.to_rfc3339(),(created_at+Duration::days(7)).to_rfc3339(),format!("{:?}",job_state.stage).to_ascii_lowercase(),checkpoint.completed as i64,checkpoint.total as i64,checkpoint.source_fingerprint,context.option_hash,display,metadata,payload_blob]
        )?;
        transaction.commit()?;
        diagnostic_checkpoint(checkpoint, DiagnosticOutcome::Succeeded);
        Ok(Some(record_id))
    }

    pub fn recoverable(&self) -> Result<Vec<RecoverableJob>, JobError> {
        self.purge_expired()?;
        let connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let mut statement = connection.prepare("SELECT id,created_at,kind,stage,completed,total,source_fingerprint,option_hash,display_name_blob,metadata_blob FROM recovery_jobs ORDER BY created_at DESC")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, usize>(4)?,
                row.get::<_, usize>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Vec<u8>>(8)?,
                row.get::<_, Vec<u8>>(9)?,
            ))
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            let (
                id,
                created_at,
                kind,
                stage,
                completed,
                total,
                fingerprint,
                option_hash,
                display,
                metadata,
            ) = row?;
            let display_name: String =
                self.crypto.open_json(&display, &aad(&id, "display_name"))?;
            let metadata: DocumentRecoveryMetadata =
                self.crypto.open_json(&metadata, &aad(&id, "metadata"))?;
            let source_matches = source_fingerprint(Path::new(&metadata.source_path)).as_deref()
                == Some(fingerprint.as_str());
            let option_matches =
                metadata.option_snapshot.hash().ok().as_deref() == Some(option_hash.as_str());
            let can_resume = source_matches && option_matches;
            jobs.push(RecoverableJob {
                record_id: id,
                display_name,
                kind,
                stage,
                completed,
                total,
                created_at,
                can_resume,
                disabled_reason: (!can_resume).then(|| {
                    if !source_matches {
                        "source_changed".into()
                    } else {
                        "options_changed".into()
                    }
                }),
                secret: false,
            });
        }
        drop(statement);
        drop(connection);
        let secret = self.secret.lock().unwrap_or_else(|p| p.into_inner());
        for (id, value) in secret.iter() {
            let source_matches = source_fingerprint(Path::new(&value.context.source_path))
                .as_deref()
                == Some(value.context.source_fingerprint.as_str());
            let option_matches = value.context.option_snapshot.hash().ok().as_deref()
                == Some(value.context.option_hash.as_str());
            let can_resume = source_matches && option_matches;
            jobs.push(RecoverableJob {
                record_id: id.clone(),
                display_name: value.context.display_name.clone(),
                kind: "document".into(),
                stage: format!("{:?}", value.payload.job_state.stage).to_ascii_lowercase(),
                completed: value.payload.checkpoint.completed,
                total: value.payload.checkpoint.total,
                created_at: value.created_at.clone(),
                can_resume,
                disabled_reason: (!can_resume).then(|| {
                    if !source_matches {
                        "source_changed".into()
                    } else {
                        "options_changed".into()
                    }
                }),
                secret: true,
            });
        }
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(jobs)
    }

    pub fn prepare_document(&self, record_id: &str) -> Result<PreparedDocumentRecovery, JobError> {
        if !valid_id(record_id) {
            return Err(JobError::Invalid);
        }
        if let Some(value) = self
            .secret
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(record_id)
        {
            if source_fingerprint(Path::new(&value.context.source_path)).as_deref()
                != Some(value.context.source_fingerprint.as_str())
                || value.context.option_snapshot.hash()? != value.context.option_hash
            {
                return Err(JobError::ResumeMismatch);
            }
            return Ok(PreparedDocumentRecovery {
                record_id: record_id.to_owned(),
                source_path: value.context.source_path.clone(),
                options: value.context.options.clone(),
                option_hash: value.context.option_hash.clone(),
            });
        }
        let (fingerprint, option_hash, metadata) = self.read_metadata(record_id)?;
        if source_fingerprint(Path::new(&metadata.source_path)).as_deref()
            != Some(fingerprint.as_str())
            || metadata.option_snapshot.hash()? != option_hash
        {
            return Err(JobError::ResumeMismatch);
        }
        Ok(PreparedDocumentRecovery {
            record_id: record_id.to_owned(),
            source_path: metadata.source_path,
            options: metadata.options,
            option_hash,
        })
    }

    pub fn resume_document(
        &self,
        record_id: &str,
        source_path: &str,
        option_hash: &str,
    ) -> Result<DocumentRetentionPayload, JobError> {
        if !valid_id(record_id) {
            return Err(JobError::Invalid);
        }
        {
            let mut secret = self.secret.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(value) = secret.get_mut(record_id) {
                if value.context.source_path != source_path
                    || value.context.option_hash != option_hash
                    || value.context.option_snapshot.hash()? != value.context.option_hash
                    || source_fingerprint(Path::new(source_path)).as_deref()
                        != Some(value.context.source_fingerprint.as_str())
                {
                    return Err(JobError::ResumeMismatch);
                }
                if value.payload.job_state.stage == JobStage::Paused {
                    let resume_stage = value
                        .payload
                        .job_state
                        .previous_active_stage
                        .ok_or(JobError::InvalidTransition)?;
                    value.payload.job_state.transition(resume_stage)?;
                }
                return Ok(DocumentRetentionPayload {
                    checkpoint: value.payload.checkpoint.clone(),
                    translated_results: value.payload.translated_results.clone(),
                });
            }
        }
        let (fingerprint, stored_hash, metadata) = self.read_metadata(record_id)?;
        if metadata.source_path != source_path
            || stored_hash != option_hash
            || metadata.option_snapshot.hash()? != stored_hash
            || source_fingerprint(Path::new(source_path)).as_deref() != Some(fingerprint.as_str())
        {
            return Err(JobError::ResumeMismatch);
        }
        let connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let blob: Vec<u8> = connection.query_row(
            "SELECT payload_blob FROM recovery_jobs WHERE id=?1",
            [record_id],
            |row| row.get(0),
        )?;
        drop(connection);
        let mut payload: DocumentRecoveryPayload =
            self.crypto.open_json(&blob, &aad(record_id, "payload"))?;
        if payload.job_state.stage == JobStage::Paused {
            let resume_stage = payload
                .job_state
                .previous_active_stage
                .ok_or(JobError::InvalidTransition)?;
            payload.job_state.transition(resume_stage)?;
            let resumed_blob = self
                .crypto
                .seal_json(&payload, &aad(record_id, "payload"))?;
            self.database
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .execute(
                    "UPDATE recovery_jobs SET stage=?1,payload_blob=?2 WHERE id=?3",
                    params![
                        format!("{:?}", resume_stage).to_ascii_lowercase(),
                        resumed_blob,
                        record_id
                    ],
                )?;
        }
        Ok(DocumentRetentionPayload {
            checkpoint: payload.checkpoint.clone(),
            translated_results: std::mem::take(&mut payload.translated_results),
        })
    }

    pub fn transition_terminal(&self, record_id: &str, next: JobStage) -> Result<bool, JobError> {
        if !valid_id(record_id)
            || !matches!(
                next,
                JobStage::Paused | JobStage::Cancelled | JobStage::Failed | JobStage::Completed
            )
        {
            return Err(JobError::Invalid);
        }
        if let Some(value) = self
            .secret
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(record_id)
        {
            if value.payload.job_state.stage != next {
                value.payload.job_state.transition(next)?;
            }
            return Ok(true);
        }
        let mut connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let blob = connection
            .query_row(
                "SELECT payload_blob FROM recovery_jobs WHERE id=?1",
                [record_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let Some(blob) = blob else { return Ok(false) };
        let mut payload: DocumentRecoveryPayload =
            self.crypto.open_json(&blob, &aad(record_id, "payload"))?;
        if payload.job_state.stage != next {
            payload.job_state.transition(next)?;
        }
        let payload_blob = self
            .crypto
            .seal_json(&payload, &aad(record_id, "payload"))?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE recovery_jobs SET stage=?1,payload_blob=?2 WHERE id=?3",
            params![
                format!("{:?}", next).to_ascii_lowercase(),
                payload_blob,
                record_id
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn delete(&self, record_id: &str) -> Result<bool, JobError> {
        if !valid_id(record_id) {
            return Err(JobError::Invalid);
        }
        let removed_secret = self
            .secret
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(record_id)
            .is_some();
        Ok(self
            .database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute("DELETE FROM recovery_jobs WHERE id=?1", [record_id])?
            > 0
            || removed_secret)
    }

    pub fn clear_secret(&self) {
        self.secret
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .clear();
    }

    pub fn purge_expired(&self) -> Result<u64, JobError> {
        Ok(self
            .database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute(
                "DELETE FROM recovery_jobs WHERE expires_at < ?1",
                [Utc::now().to_rfc3339()],
            )? as u64)
    }

    fn read_metadata(
        &self,
        record_id: &str,
    ) -> Result<(String, String, DocumentRecoveryMetadata), JobError> {
        if !valid_id(record_id) {
            return Err(JobError::Invalid);
        }
        let connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let row=connection.query_row("SELECT source_fingerprint,option_hash,metadata_blob FROM recovery_jobs WHERE id=?1",[record_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Vec<u8>>(2)?))).optional()?.ok_or(JobError::Invalid)?;
        drop(connection);
        Ok((
            row.0,
            row.1,
            self.crypto.open_json(&row.2, &aad(record_id, "metadata"))?,
        ))
    }
}

pub struct EncryptedDocumentResumeBackend {
    store: Arc<JobStore>,
    documents: Arc<DocumentJobStore>,
}
impl EncryptedDocumentResumeBackend {
    pub fn new(store: Arc<JobStore>, documents: Arc<DocumentJobStore>) -> Self {
        Self { store, documents }
    }
}
impl DocumentResumeBackend for EncryptedDocumentResumeBackend {
    fn retention_requested(&self, notice: DocumentRetentionNotice) -> bool {
        let Some(payload) = self
            .documents
            .take_retention_payload(notice.job_id, &notice.retention_token)
        else {
            return false;
        };
        self.store
            .checkpoint_document(notice.job_id, &notice.context, &payload, false)
            .is_ok()
    }
    fn resolve_resume_record(
        &self,
        record_id: &str,
        source_path: &str,
        option_hash: &str,
    ) -> Option<DocumentRetentionPayload> {
        self.store
            .resume_document(record_id, source_path, option_hash)
            .ok()
    }
}

fn aad(id: &str, column: &str) -> Vec<u8> {
    format!("recovery_jobs:{id}:{column}").into_bytes()
}
fn valid_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|v| v.is_ascii_hexdigit())
}
fn source_fingerprint(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

fn initial_job_state() -> JobCheckpoint {
    JobCheckpoint {
        stage: JobStage::Queued,
        previous_active_stage: None,
        completed_unit_ids: Vec::new(),
    }
}

fn document_job_stage(stage: DocumentStage) -> JobStage {
    match stage {
        DocumentStage::Inspect | DocumentStage::Extract | DocumentStage::Ocr => JobStage::Extract,
        DocumentStage::Translate => JobStage::Translate,
        DocumentStage::Reflow => JobStage::Rebuild,
        DocumentStage::Validate => JobStage::Validate,
        DocumentStage::Save => JobStage::Save,
        DocumentStage::Completed => JobStage::Completed,
    }
}

fn stage_rank(stage: JobStage) -> Option<usize> {
    match stage {
        JobStage::Queued => Some(0),
        JobStage::Extract => Some(1),
        JobStage::Translate => Some(2),
        JobStage::Rebuild => Some(3),
        JobStage::Validate => Some(4),
        JobStage::Save => Some(5),
        JobStage::Completed => Some(6),
        JobStage::Paused | JobStage::Cancelled | JobStage::Failed => None,
    }
}

fn advance_job_state(
    state: &mut JobCheckpoint,
    document_stage: DocumentStage,
) -> Result<(), JobError> {
    let target = document_job_stage(document_stage);
    let current_rank = stage_rank(state.stage).ok_or(JobError::InvalidTransition)?;
    let target_rank = stage_rank(target).ok_or(JobError::InvalidTransition)?;
    if target_rank < current_rank {
        return Err(JobError::InvalidTransition);
    }
    let sequence = [
        JobStage::Queued,
        JobStage::Extract,
        JobStage::Translate,
        JobStage::Rebuild,
        JobStage::Validate,
        JobStage::Save,
        JobStage::Completed,
    ];
    for next in sequence.iter().take(target_rank + 1).skip(current_rank + 1) {
        state.transition(*next)?;
    }
    Ok(())
}

fn diagnostic_checkpoint(checkpoint: &DocumentCheckpoint, outcome: DiagnosticOutcome) {
    DiagnosticEvent::new(DiagnosticEventName::JobLifecycle, outcome)
        .with_job_kind(JobKind::Document)
        .with_stage(&format!("{:?}", checkpoint.stage).to_ascii_lowercase())
        .with_counts(checkpoint.completed as u64, 0)
        .emit();
}

fn remember_completed_unit(state: &mut JobCheckpoint, checkpoint: &DocumentCheckpoint) {
    let completed = checkpoint.stage != DocumentStage::Translate
        || checkpoint.stable_unit_id.ends_with(":completed");
    if completed
        && state.completed_unit_ids.len() < 10_000
        && !state
            .completed_unit_ids
            .iter()
            .any(|value| value == &checkpoint.stable_unit_id)
    {
        state
            .completed_unit_ids
            .push(checkpoint.stable_unit_id.clone());
    }
}
