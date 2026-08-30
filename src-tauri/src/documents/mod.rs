pub mod docx;
pub mod ooxml;
pub mod output;
pub mod pdf;
pub mod pipeline;
pub mod pptx;
pub mod segments;
pub mod translate;
pub mod types;
pub mod xlsx;

pub use pipeline::{inspect_document, rebuild_document};
pub use types::*;

use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use uuid::Uuid;

#[derive(Default)]
pub struct DocumentJobStore {
    active: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    retention_acks: Mutex<HashSet<Uuid>>,
}
impl DocumentJobStore {
    pub fn begin(&self, id: Uuid) -> Arc<AtomicBool> {
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
    pub fn acknowledge_retention(&self, id: Uuid) {
        self.retention_acks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id);
    }
    pub fn take_retention_ack(&self, id: Uuid) -> bool {
        self.retention_acks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id)
    }
}
