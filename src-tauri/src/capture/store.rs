use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use uuid::Uuid;

use super::{render::RenderedImage, CaptureJobResult, DecodedImage};

pub struct CaptureJob {
    pub source: DecodedImage,
    pub result: CaptureJobResult,
    pub rendered: Option<RenderedImage>,
    pub cancelled: Arc<AtomicBool>,
    pub translation_job: Option<Uuid>,
}

#[derive(Default)]
pub struct CaptureJobStore {
    jobs: Mutex<HashMap<Uuid, CaptureJob>>,
}

impl CaptureJobStore {
    pub fn insert(&self, job: CaptureJob) {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(job.result.job_id, job);
    }
    pub fn with<T>(&self, id: Uuid, operation: impl FnOnce(&CaptureJob) -> T) -> Option<T> {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .map(operation)
    }
    pub fn with_mut<T>(&self, id: Uuid, operation: impl FnOnce(&mut CaptureJob) -> T) -> Option<T> {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&id)
            .map(operation)
    }
    pub fn cancel(&self, id: Uuid) -> Option<Option<Uuid>> {
        self.with_mut(id, |job| {
            job.cancelled.store(true, Ordering::Release);
            job.translation_job
        })
    }
}
