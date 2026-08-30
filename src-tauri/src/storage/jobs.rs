use crate::{
    documents::{
        DocumentCheckpoint, DocumentJobStore, DocumentOptions, DocumentResumeBackend,
        DocumentRetentionNotice, DocumentRetentionPayload, TranslatedSegment,
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
    checkpoint: DocumentCheckpoint,
    translated_results: HashMap<String, TranslatedSegment>,
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
    secret: Arc<Mutex<HashMap<String, DocumentRecoveryPayload>>>,
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
        let serializable = DocumentRecoveryPayload {
            checkpoint: payload.checkpoint.clone(),
            translated_results: payload.translated_results.clone(),
        };
        if secret {
            self.secret
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(record_id.clone(), serializable);
            return Ok(None);
        }
        if context.option_snapshot.hash()? != context.option_hash || context.option_hash.len() != 64
        {
            return Err(JobError::Invalid);
        }
        let metadata = DocumentRecoveryMetadata {
            source_path: context.source_path.clone(),
            options: context.options.clone(),
            option_snapshot: context.option_snapshot.clone(),
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
        self.database.0.lock().unwrap_or_else(|p| p.into_inner()).execute(
            "INSERT OR REPLACE INTO recovery_jobs (id,created_at,expires_at,kind,stage,completed,total,source_fingerprint,option_hash,display_name_blob,metadata_blob,payload_blob) VALUES (?1,?2,?3,'document',?4,?5,?6,?7,?8,?9,?10,?11)",
            params![record_id,created_at.to_rfc3339(),(created_at+Duration::days(7)).to_rfc3339(),format!("{:?}",checkpoint.stage).to_ascii_lowercase(),checkpoint.completed as i64,checkpoint.total as i64,checkpoint.source_fingerprint,context.option_hash,display,metadata,payload_blob]
        )?;
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
            });
        }
        Ok(jobs)
    }

    pub fn prepare_document(&self, record_id: &str) -> Result<PreparedDocumentRecovery, JobError> {
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
        let payload: DocumentRecoveryPayload =
            self.crypto.open_json(&blob, &aad(record_id, "payload"))?;
        Ok(DocumentRetentionPayload {
            checkpoint: payload.checkpoint,
            translated_results: payload.translated_results,
        })
    }

    pub fn delete(&self, record_id: &str) -> Result<bool, JobError> {
        if !valid_id(record_id) {
            return Err(JobError::Invalid);
        }
        Ok(self
            .database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute("DELETE FROM recovery_jobs WHERE id=?1", [record_id])?
            > 0)
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
