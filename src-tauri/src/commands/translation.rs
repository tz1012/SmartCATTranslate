use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::codex::translation::{
    CodexTranslationBackend, TranslationJobPermit, TranslationObserver,
};
use crate::core::errors::TranslationError;
use crate::core::types::{TranslationRequest, TranslationResult};

const TRANSLATION_EVENT_NAME: &str = "translation-event";
const TRANSLATION_SERVICE_UNAVAILABLE: &str = "translation_service_unavailable";

#[derive(Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TranslationEvent {
    Delta {
        job_id: Uuid,
        text: String,
    },
    Completed {
        job_id: Uuid,
        result: TranslationResult,
    },
    Failed {
        job_id: Uuid,
        code: String,
        message: String,
    },
}

impl fmt::Debug for TranslationEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, job_id) = match self {
            Self::Delta { job_id, .. } => ("Delta", job_id),
            Self::Completed { job_id, .. } => ("Completed", job_id),
            Self::Failed { job_id, .. } => ("Failed", job_id),
        };
        formatter
            .debug_struct("TranslationEvent")
            .field("kind", &kind)
            .field("job_id", job_id)
            .field("content", &"<redacted>")
            .finish()
    }
}

pub trait TranslationEventSink: Send + Sync + 'static {
    fn emit(&self, event: TranslationEvent);
}

pub struct TranslationJobManager {
    backend: Arc<CodexTranslationBackend>,
    jobs: Mutex<HashMap<Uuid, String>>,
}

impl TranslationJobManager {
    pub fn new(backend: Arc<CodexTranslationBackend>) -> Self {
        Self {
            backend,
            jobs: Mutex::new(HashMap::new()),
        }
    }

    pub async fn start(
        self: &Arc<Self>,
        owner: impl Into<String>,
        request: TranslationRequest,
        sink: Arc<dyn TranslationEventSink>,
    ) -> Result<Uuid, TranslationError> {
        let job_id = Uuid::new_v4();
        let permit = self.backend.reserve_job(job_id).await?;
        self.jobs.lock().await.insert(job_id, owner.into());
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.run(job_id, permit, request, sink).await;
        });
        Ok(job_id)
    }

    pub async fn cancel(&self, owner: &str, job_id: Uuid) -> bool {
        let is_owner = self
            .jobs
            .lock()
            .await
            .get(&job_id)
            .is_some_and(|job_owner| job_owner == owner);
        is_owner && self.backend.cancel_job(job_id).await
    }

    pub async fn cancel_owner(&self, owner: &str) {
        let job_ids: Vec<_> = self
            .jobs
            .lock()
            .await
            .iter()
            .filter_map(|(job_id, job_owner)| (job_owner == owner).then_some(*job_id))
            .collect();
        for job_id in job_ids {
            let _ = self.backend.cancel_job(job_id).await;
        }
    }

    pub async fn shutdown(&self) -> Result<(), TranslationError> {
        self.backend.shutdown().await
    }

    async fn run(
        &self,
        job_id: Uuid,
        permit: TranslationJobPermit,
        request: TranslationRequest,
        sink: Arc<dyn TranslationEventSink>,
    ) {
        let observer = JobObserver {
            job_id,
            sink: sink.clone(),
        };
        let result = self
            .backend
            .translate_reserved(permit, request, &observer)
            .await;
        match result {
            Ok(result) => sink.emit(TranslationEvent::Completed { job_id, result }),
            Err(error) => sink.emit(TranslationEvent::Failed {
                job_id,
                code: error_code(&error).to_owned(),
                message: user_message(&error).to_owned(),
            }),
        }
        self.jobs.lock().await.remove(&job_id);
    }
}

struct JobObserver {
    job_id: Uuid,
    sink: Arc<dyn TranslationEventSink>,
}

impl TranslationObserver for JobObserver {
    fn on_delta(&self, text: &str) {
        self.sink.emit(TranslationEvent::Delta {
            job_id: self.job_id,
            text: text.to_owned(),
        });
    }
}

struct TauriWindowEventSink(tauri::Window);

impl TranslationEventSink for TauriWindowEventSink {
    fn emit(&self, event: TranslationEvent) {
        let _ = self.0.emit(TRANSLATION_EVENT_NAME, event);
    }
}

#[tauri::command]
pub async fn translate_text(
    window: tauri::Window,
    state: State<'_, AppState>,
    request: TranslationRequest,
) -> Result<Uuid, String> {
    let manager = state
        .translation_jobs()
        .await
        .ok_or_else(|| TRANSLATION_SERVICE_UNAVAILABLE.to_owned())?;
    let owner = window.label().to_owned();
    manager
        .start(owner, request, Arc::new(TauriWindowEventSink(window)))
        .await
        .map_err(|error| error_code(&error).to_owned())
}

#[tauri::command]
pub async fn cancel_translation(
    window: tauri::Window,
    state: State<'_, AppState>,
    job_id: Uuid,
) -> Result<bool, String> {
    let manager = state
        .translation_jobs()
        .await
        .ok_or_else(|| TRANSLATION_SERVICE_UNAVAILABLE.to_owned())?;
    Ok(manager.cancel(window.label(), job_id).await)
}

fn error_code(error: &TranslationError) -> &'static str {
    match error {
        TranslationError::InvalidInput => "invalid_translation_input",
        TranslationError::UnsafeWorkspace => "unsafe_translation_workspace",
        TranslationError::InvalidOutput => "invalid_translation_output",
        TranslationError::SizeLimitExceeded => "translation_size_limit",
        TranslationError::ToolUseRejected => "translation_tool_rejected",
        TranslationError::RuntimeUnavailable => "translation_runtime_unavailable",
        TranslationError::ProtocolViolation => "translation_protocol_violation",
        TranslationError::TimedOut => "translation_timed_out",
        TranslationError::Cancelled => "translation_cancelled",
        TranslationError::ShuttingDown => "translation_shutting_down",
    }
}

fn user_message(error: &TranslationError) -> &'static str {
    match error {
        TranslationError::Cancelled => "번역이 취소되었습니다.",
        TranslationError::TimedOut => "번역 시간이 초과되었습니다.",
        TranslationError::ToolUseRejected => "안전하지 않은 동작 요청을 차단했습니다.",
        TranslationError::InvalidInput | TranslationError::SizeLimitExceeded => {
            "번역할 텍스트를 확인해 주세요."
        }
        _ => "번역을 완료하지 못했습니다.",
    }
}
