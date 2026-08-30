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
};
use uuid::Uuid;

#[derive(Default)]
pub struct DocumentJobStore {
    active: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    retention: Mutex<HashMap<Uuid, OutstandingRetention>>,
    retention_receipts: Mutex<HashMap<Uuid, String>>,
}

#[derive(Clone)]
struct OutstandingRetention {
    token: String,
    payload: DocumentRetentionPayload,
}

#[derive(Clone)]
pub struct DocumentRetentionPayload {
    pub checkpoint: DocumentCheckpoint,
    pub translated_results: HashMap<String, TranslatedSegment>,
}

impl DocumentJobStore {
    pub fn begin(&self, id: Uuid) -> Arc<AtomicBool> {
        self.retention
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        self.retention_receipts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        let flag = Arc::new(AtomicBool::new(false));
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, flag.clone());
        flag
    }
    pub fn cancel(&self, id: Uuid) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .is_some_and(|v| {
                v.store(true, std::sync::atomic::Ordering::Release);
                true
            })
    }
    pub fn finish(&self, id: Uuid) {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
    }
    pub fn request_retention(
        &self,
        id: Uuid,
        token: String,
        checkpoint: DocumentCheckpoint,
        translated_results: HashMap<String, TranslatedSegment>,
    ) {
        self.retention
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id,
                OutstandingRetention {
                    token,
                    payload: DocumentRetentionPayload {
                        checkpoint,
                        translated_results,
                    },
                },
            );
    }

    pub fn retention_payload(&self, id: Uuid, token: &str) -> Option<DocumentRetentionPayload> {
        self.retention
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .filter(|value| value.token == token)
            .map(|value| value.payload.clone())
    }

    pub fn acknowledge_retention(&self, id: Uuid, token: &str, receipt: &str) -> bool {
        if !valid_opaque_value(token, 32, 64) || !valid_opaque_value(receipt, 16, 256) {
            return false;
        }
        let mut retention = self.retention.lock().unwrap_or_else(|p| p.into_inner());
        if !retention
            .get(&id)
            .is_some_and(|outstanding| outstanding.token == token)
        {
            return false;
        }
        retention.remove(&id);
        drop(retention);
        self.retention_receipts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, receipt.to_owned());
        true
    }
}

fn valid_opaque_value(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
