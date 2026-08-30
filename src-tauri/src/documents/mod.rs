pub mod docx;
pub mod ooxml;
pub mod output;
pub mod pdf;
pub mod pipeline;
pub mod pptx;
pub mod preview;
pub mod segments;
pub mod translate;
pub mod types;
pub mod xlsx;

pub use pipeline::{inspect_document, rebuild_document};
pub use types::*;

use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::Zeroize;

const RETENTION_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct DocumentJobStore {
    active: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    retention: Arc<Mutex<HashMap<Uuid, OutstandingRetention>>>,
    resume_backend: Mutex<Option<Arc<dyn DocumentResumeBackend>>>,
}

struct OutstandingRetention {
    token: String,
    expires_at: Instant,
    payload: DocumentRetentionPayload,
}

#[derive(Clone)]
pub struct DocumentRetentionPayload {
    pub checkpoint: DocumentCheckpoint,
    pub translated_results: HashMap<String, TranslatedSegment>,
}

impl Drop for DocumentRetentionPayload {
    fn drop(&mut self) {
        wipe_translated_results(&mut self.translated_results);
    }
}

#[derive(Clone)]
pub struct DocumentRetentionNotice {
    pub job_id: Uuid,
    pub retention_token: String,
    pub checkpoint: DocumentCheckpoint,
}

/// Backend-only bridge for the future encrypted history JobStore. The public Tauri API never
/// receives retention tokens, checkpoints paired with plaintext, or translated segment values.
pub trait DocumentResumeBackend: Send + Sync {
    /// Return true only when the backend will synchronously claim the short-lived payload with
    /// `DocumentJobStore::take_retention_payload` and persist it encrypted.
    fn retention_requested(&self, notice: DocumentRetentionNotice) -> bool;

    /// Resolve an opaque encrypted-record identifier. Implementations own decryption and record
    /// authentication; this document module only validates the source fingerprint and prefix.
    fn resolve_resume_record(&self, record_id: &str) -> Option<DocumentRetentionPayload>;
}

impl DocumentJobStore {
    pub fn install_resume_backend(&self, backend: Arc<dyn DocumentResumeBackend>) {
        *self
            .resume_backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(backend);
    }

    pub fn begin(&self, id: Uuid) -> Arc<AtomicBool> {
        self.purge_expired();
        self.discard_retention(id);
        let flag = Arc::new(AtomicBool::new(false));
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, flag.clone());
        flag
    }

    pub fn cancel(&self, id: Uuid) -> bool {
        self.purge_expired();
        self.discard_retention(id);
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
            .is_some_and(|value| {
                value.store(true, std::sync::atomic::Ordering::Release);
                true
            })
    }

    pub fn finish(&self, id: Uuid) {
        self.purge_expired();
        self.discard_retention(id);
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
    }

    /// Offers a retry payload to the backend-only consumer. With no encrypted JobStore consumer
    /// installed, the plaintext payload is zeroized and dropped immediately.
    pub fn request_retention(
        &self,
        id: Uuid,
        token: String,
        checkpoint: DocumentCheckpoint,
        translated_results: HashMap<String, TranslatedSegment>,
    ) -> bool {
        self.purge_expired();
        self.discard_retention(id);
        if !valid_opaque_value(&token, 32, 64) {
            let mut translated_results = translated_results;
            wipe_translated_results(&mut translated_results);
            return false;
        }
        self.retention
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                id,
                OutstandingRetention {
                    token: token.clone(),
                    expires_at: Instant::now() + RETENTION_TTL,
                    payload: DocumentRetentionPayload {
                        checkpoint: checkpoint.clone(),
                        translated_results,
                    },
                },
            );
        let accepted = self
            .resume_backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .is_some_and(|backend| {
                backend.retention_requested(DocumentRetentionNotice {
                    job_id: id,
                    retention_token: token.clone(),
                    checkpoint,
                })
            });
        if !accepted {
            self.discard_retention(id);
            return false;
        }

        let retention = Arc::clone(&self.retention);
        std::thread::spawn(move || {
            std::thread::sleep(RETENTION_TTL);
            let mut values = retention
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if values
                .get(&id)
                .is_some_and(|value| value.token == token && value.expires_at <= Instant::now())
            {
                values.remove(&id);
            }
        });
        true
    }

    /// Backend-only one-shot plaintext handoff. Taking the payload is the ACK: after this call no
    /// copy remains in DocumentJobStore, and the consumer must encrypt or zeroize it immediately.
    pub fn take_retention_payload(
        &self,
        id: Uuid,
        token: &str,
    ) -> Option<DocumentRetentionPayload> {
        self.purge_expired();
        if !valid_opaque_value(token, 32, 64) {
            return None;
        }
        let mut values = self
            .retention
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let outstanding = values
            .get(&id)
            .filter(|value| value.token == token && value.expires_at > Instant::now())?;
        let payload = outstanding.payload.clone();
        values.remove(&id);
        Some(payload)
    }

    pub fn resolve_resume_record(&self, record_id: &str) -> Option<DocumentRetentionPayload> {
        self.purge_expired();
        if !valid_opaque_value(record_id, 16, 256) {
            return None;
        }
        self.resume_backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()?
            .resolve_resume_record(record_id)
    }

    fn discard_retention(&self, id: Uuid) {
        self.retention
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
    }

    fn purge_expired(&self) {
        let now = Instant::now();
        self.retention
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, value| value.expires_at > now);
    }
}

pub fn wipe_translated_results(values: &mut HashMap<String, TranslatedSegment>) {
    for (mut reference, mut translated) in values.drain() {
        reference.zeroize();
        translated.zeroize();
    }
}

fn valid_opaque_value(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
