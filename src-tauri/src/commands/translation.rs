use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::codex::translation::{
    validate_translation_request, CodexTranslationBackend, TranslationJobPermit,
    TranslationObserver,
};
use crate::core::errors::TranslationError;
use crate::core::types::{TranslationMode, TranslationRequest, TranslationResult};
use crate::settings::types::{
    language_pair_action, resolve_model_for_job, AvailableModel, LanguagePairAction,
    ModelCatalogAuthority, ModelChoice,
};

const TRANSLATION_EVENT_NAME: &str = "translation-event";
const TRANSLATION_SERVICE_UNAVAILABLE: &str = "translation_service_unavailable";
const MODEL_PREFLIGHT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

enum ModelPreflightOutcome {
    Available(Vec<AvailableModel>),
    Unavailable,
    SignedOut,
    TimedOut,
    Cancelled,
}

fn resolve_preflight_model(
    choice: &ModelChoice,
    outcome: ModelPreflightOutcome,
) -> Result<crate::core::types::TranslationModel, String> {
    match outcome {
        ModelPreflightOutcome::Available(models) => {
            resolve_model_for_job(choice, ModelCatalogAuthority::Available(&models))
                .map_err(|error| error.code().to_owned())
        }
        ModelPreflightOutcome::Unavailable | ModelPreflightOutcome::TimedOut => match choice {
            ModelChoice::Automatic => Ok(crate::core::types::TranslationModel::Automatic),
            ModelChoice::Specific { .. } => Err("model_catalog_unavailable".to_owned()),
        },
        ModelPreflightOutcome::SignedOut => Err("model_catalog_signed_out".to_owned()),
        ModelPreflightOutcome::Cancelled => Err("translation_cancelled".to_owned()),
    }
}

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
    registry: SharedOwnerJobRegistry,
}

pub struct PreparedTranslationJob {
    job_id: Uuid,
    permit: TranslationJobPermit,
}

impl PreparedTranslationJob {
    pub fn job_id(&self) -> Uuid {
        self.job_id
    }

    pub async fn wait_for_preflight<F, T>(
        &mut self,
        maximum_wait: std::time::Duration,
        future: F,
    ) -> Result<T, TranslationError>
    where
        F: std::future::Future<Output = T>,
    {
        self.permit.wait_for_preflight(maximum_wait, future).await
    }
}

#[derive(Default)]
pub(crate) struct OwnerJobRegistry {
    jobs: HashMap<Uuid, String>,
    tombstones: HashSet<String>,
}

pub(crate) type SharedOwnerJobRegistry = Arc<std::sync::Mutex<OwnerJobRegistry>>;

pub(crate) fn new_owner_job_registry() -> SharedOwnerJobRegistry {
    Arc::new(std::sync::Mutex::new(OwnerJobRegistry::default()))
}

fn lock_registry(registry: &SharedOwnerJobRegistry) -> std::sync::MutexGuard<'_, OwnerJobRegistry> {
    registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl OwnerJobRegistry {
    pub(crate) fn begin(&mut self, owner: String, job_id: Uuid) -> Result<(), TranslationError> {
        if self.tombstones.contains(&owner) {
            return Err(TranslationError::Cancelled);
        }
        self.jobs.insert(job_id, owner);
        Ok(())
    }

    fn owner_is_live(&self, job_id: Uuid) -> bool {
        self.jobs
            .get(&job_id)
            .is_some_and(|owner| !self.tombstones.contains(owner))
    }

    pub(crate) fn remove(&mut self, job_id: Uuid) {
        self.jobs.remove(&job_id);
    }

    pub(crate) fn tombstone(&mut self, owner: &str) -> Vec<Uuid> {
        self.tombstones.insert(owner.to_owned());
        self.jobs
            .iter()
            .filter_map(|(job_id, job_owner)| (job_owner == owner).then_some(*job_id))
            .collect()
    }
}

impl TranslationJobManager {
    pub fn new(backend: Arc<CodexTranslationBackend>) -> Self {
        Self::with_registry(backend, new_owner_job_registry())
    }

    pub(crate) fn with_registry(
        backend: Arc<CodexTranslationBackend>,
        registry: SharedOwnerJobRegistry,
    ) -> Self {
        Self { backend, registry }
    }

    pub async fn start(
        self: &Arc<Self>,
        owner: impl Into<String>,
        request: TranslationRequest,
        sink: Arc<dyn TranslationEventSink>,
    ) -> Result<Uuid, TranslationError> {
        validate_translation_request(&request)?;
        let prepared = self.prepare(owner).await?;
        self.start_prepared(prepared, request, sink).await
    }

    pub async fn prepare(
        self: &Arc<Self>,
        owner: impl Into<String>,
    ) -> Result<PreparedTranslationJob, TranslationError> {
        let job_id = Uuid::new_v4();
        lock_registry(&self.registry).begin(owner.into(), job_id)?;
        self.prepare_reserved(job_id).await
    }

    pub(crate) async fn prepare_reserved(
        &self,
        job_id: Uuid,
    ) -> Result<PreparedTranslationJob, TranslationError> {
        let permit = match self.backend.reserve_job(job_id).await {
            Ok(permit) => permit,
            Err(error) => {
                lock_registry(&self.registry).remove(job_id);
                return Err(error);
            }
        };
        if !lock_registry(&self.registry).owner_is_live(job_id) {
            self.backend.discard_reserved_job(permit).await;
            lock_registry(&self.registry).remove(job_id);
            return Err(TranslationError::Cancelled);
        }
        Ok(PreparedTranslationJob { job_id, permit })
    }

    pub(crate) async fn start_prepared(
        self: &Arc<Self>,
        prepared: PreparedTranslationJob,
        request: TranslationRequest,
        sink: Arc<dyn TranslationEventSink>,
    ) -> Result<Uuid, TranslationError> {
        let job_id = prepared.job_id;
        if let Err(error) = validate_translation_request(&request) {
            self.discard_prepared(prepared).await;
            return Err(error);
        }
        if !lock_registry(&self.registry).owner_is_live(job_id) {
            self.discard_prepared(prepared).await;
            return Err(TranslationError::Cancelled);
        }
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.run(prepared, request, sink).await;
        });
        Ok(job_id)
    }

    pub async fn discard_prepared(&self, prepared: PreparedTranslationJob) {
        self.backend.discard_reserved_job(prepared.permit).await;
        lock_registry(&self.registry).remove(prepared.job_id);
    }

    pub async fn cancel(&self, owner: &str, job_id: Uuid) -> bool {
        let is_owner = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .jobs
            .get(&job_id)
            .is_some_and(|job_owner| job_owner == owner);
        is_owner && self.backend.cancel_job(job_id).await
    }

    pub async fn cancel_owner(&self, owner: &str) {
        let job_ids = lock_registry(&self.registry).tombstone(owner);
        self.cancel_job_ids(&job_ids).await;
    }

    pub(crate) async fn cancel_job_ids(&self, job_ids: &[Uuid]) {
        for job_id in job_ids {
            let _ = self.backend.cancel_job(*job_id).await;
        }
    }

    pub async fn shutdown(&self) -> Result<(), TranslationError> {
        self.backend.shutdown().await
    }

    async fn run(
        &self,
        prepared: PreparedTranslationJob,
        request: TranslationRequest,
        sink: Arc<dyn TranslationEventSink>,
    ) {
        let job_id = prepared.job_id;
        let observer = JobObserver {
            job_id,
            sink: sink.clone(),
        };
        let result = self
            .backend
            .translate_reserved(prepared.permit, request, &observer)
            .await;
        match result {
            Ok(result) => sink.emit(TranslationEvent::Completed { job_id, result }),
            Err(error) => sink.emit(TranslationEvent::Failed {
                job_id,
                code: error_code(&error).to_owned(),
                message: user_message(&error).to_owned(),
            }),
        }
        lock_registry(&self.registry).remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_preflight_model, validate_translation_action, ModelPreflightOutcome,
        OwnerJobRegistry,
    };
    use crate::core::types::{
        Quality, Tone, TranslationMode, TranslationModel, TranslationProfile, TranslationRequest,
    };
    use crate::settings::types::ModelChoice;
    use uuid::Uuid;

    #[test]
    fn destroy_between_reservation_and_activation_tombstones_the_job() {
        let mut registry = OwnerJobRegistry::default();
        let job_id = Uuid::new_v4();
        registry.begin("main".to_owned(), job_id).unwrap();

        let jobs_to_cancel = registry.tombstone("main");

        assert_eq!(jobs_to_cancel, [job_id]);
        assert!(!registry.owner_is_live(job_id));
        assert!(registry.begin("main".to_owned(), Uuid::new_v4()).is_err());
    }

    #[test]
    fn explicit_same_language_requires_the_rewrite_api_path() {
        let mut request = TranslationRequest {
            text: "same language".into(),
            profile: TranslationProfile {
                source_language: Some("ko".into()),
                target_language: "ko".into(),
                quality: Quality::Balanced,
                tone: Tone::Natural,
                protected_terms: Vec::new(),
            },
            mode: TranslationMode::Translate,
            secret: false,
            model: TranslationModel::Automatic,
        };
        assert_eq!(
            validate_translation_action(&request),
            Err("rewrite_suggested")
        );
        request.mode = TranslationMode::Rewrite;
        assert_eq!(validate_translation_action(&request), Ok(()));
    }

    #[test]
    fn automatic_model_uses_protocol_default_after_bounded_catalog_failure() {
        for outcome in [
            ModelPreflightOutcome::TimedOut,
            ModelPreflightOutcome::Unavailable,
        ] {
            assert_eq!(
                resolve_preflight_model(&ModelChoice::Automatic, outcome),
                Ok(TranslationModel::Automatic)
            );
        }
    }

    #[test]
    fn explicit_model_fails_closed_when_catalog_cannot_be_authoritatively_checked() {
        let choice = ModelChoice::Specific {
            id: "saved-model".into(),
        };
        for outcome in [
            ModelPreflightOutcome::TimedOut,
            ModelPreflightOutcome::Unavailable,
        ] {
            assert_eq!(
                resolve_preflight_model(&choice, outcome),
                Err("model_catalog_unavailable".to_owned())
            );
        }
        assert_eq!(
            resolve_preflight_model(&ModelChoice::Automatic, ModelPreflightOutcome::SignedOut),
            Err("model_catalog_signed_out".to_owned())
        );
        assert_eq!(
            resolve_preflight_model(&ModelChoice::Automatic, ModelPreflightOutcome::Cancelled),
            Err("translation_cancelled".to_owned())
        );
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
    app: tauri::AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    mut request: TranslationRequest,
) -> Result<Uuid, String> {
    let owner = window.label().to_owned();
    validate_translation_action(&request).map_err(str::to_owned)?;
    validate_translation_request(&request).map_err(|error| error_code(&error).to_owned())?;
    let job_id = state
        .reserve_window_translation_job(&owner)
        .map_err(|error| error_code(&error).to_owned())?;
    let manager = match state.translation_jobs().await {
        Some(manager) => manager,
        None => {
            state.release_window_translation_job(job_id);
            return Err(TRANSLATION_SERVICE_UNAVAILABLE.to_owned());
        }
    };
    let mut prepared = manager
        .prepare_reserved(job_id)
        .await
        .map_err(|error| error_code(&error).to_owned())?;
    let selected_model = match prepared
        .wait_for_preflight(MODEL_PREFLIGHT_TIMEOUT, async {
            let _operation = state
                .lock_settings_operation()
                .await
                .map_err(|_| "settings_shutting_down".to_owned())?;
            crate::commands::settings::open_store(&app)?
                .load()
                .await
                .map(|settings| settings.selected_model)
                .map_err(|error| error.code().to_owned())
        })
        .await
    {
        Ok(Ok(choice)) => choice,
        Ok(Err(code)) => {
            manager.discard_prepared(prepared).await;
            return Err(code);
        }
        Err(error) => {
            manager.discard_prepared(prepared).await;
            return Err(error_code(&error).to_owned());
        }
    };
    let preflight = prepared
        .wait_for_preflight(
            MODEL_PREFLIGHT_TIMEOUT,
            crate::commands::settings::read_authoritative_models(&state),
        )
        .await;
    let outcome = match preflight {
        Ok(Ok(models)) => ModelPreflightOutcome::Available(models),
        Ok(Err(code)) if code == "model_catalog_signed_out" => ModelPreflightOutcome::SignedOut,
        Ok(Err(_)) => ModelPreflightOutcome::Unavailable,
        Err(TranslationError::TimedOut) => ModelPreflightOutcome::TimedOut,
        Err(_) => ModelPreflightOutcome::Cancelled,
    };
    request.model = match resolve_preflight_model(&selected_model, outcome) {
        Ok(model) => model,
        Err(code) => {
            manager.discard_prepared(prepared).await;
            return Err(code);
        }
    };
    manager
        .start_prepared(prepared, request, Arc::new(TauriWindowEventSink(window)))
        .await
        .map_err(|error| error_code(&error).to_owned())
}

fn validate_translation_action(request: &TranslationRequest) -> Result<(), &'static str> {
    if request.mode == TranslationMode::Translate
        && language_pair_action(
            request.profile.source_language.as_deref(),
            &request.profile.target_language,
        ) == LanguagePairAction::RewriteSuggested
    {
        Err("rewrite_suggested")
    } else {
        Ok(())
    }
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
