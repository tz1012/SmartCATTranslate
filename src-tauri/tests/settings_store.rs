use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use smartcat_translate::codex::protocol::AppServerNotification;
use smartcat_translate::codex::transport::{AppServerTransport, TransportError};
use smartcat_translate::core::types::{Quality, Tone, TranslationModel, TranslationProfile};
use smartcat_translate::settings::store::{FileSettingsBackend, SettingsBackend, SettingsStore};
use smartcat_translate::settings::types::{
    language_pair_action, resolve_model_choice, resolve_model_for_job, AppLocale, AppSettings,
    AvailableModel, Field, GlossaryEntry, LanguagePairAction, ModelCatalogAuthority, ModelChoice,
    Theme,
};
use smartcat_translate::settings::ModelCatalogService;
use tokio::sync::broadcast;

#[derive(Clone, Default)]
struct MemoryBackend {
    state: Arc<Mutex<MemoryState>>,
}

#[derive(Default)]
struct MemoryState {
    values: BTreeMap<String, Value>,
    save_count: usize,
}

#[derive(Clone, Default)]
struct FailingSaveBackend {
    state: Arc<Mutex<MemoryState>>,
}

#[async_trait]
impl SettingsBackend for FailingSaveBackend {
    async fn read(&self) -> Result<Option<Value>, String> {
        Ok(self.state.lock().unwrap().values.get("settings").cloned())
    }

    async fn replace(&self, _value: Value) -> Result<(), String> {
        Err("durable replacement failed".into())
    }
}

#[async_trait]
impl SettingsBackend for MemoryBackend {
    async fn read(&self) -> Result<Option<Value>, String> {
        Ok(self.state.lock().unwrap().values.get("settings").cloned())
    }

    async fn replace(&self, value: Value) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.values.insert("settings".to_owned(), value);
        state.save_count += 1;
        Ok(())
    }
}

fn profile(source: Option<&str>, target: &str) -> TranslationProfile {
    TranslationProfile {
        source_language: source.map(str::to_owned),
        target_language: target.to_owned(),
        quality: Quality::Balanced,
        tone: Tone::Natural,
        protected_terms: Vec::new(),
    }
}

#[tokio::test]
async fn defaults_are_auto_to_korean_balanced_natural_system_and_korean_ui() {
    let store = SettingsStore::new(MemoryBackend::default());
    let settings = store.load().await.unwrap();
    let default_profile = settings.default_profile().unwrap();

    assert_eq!(settings.locale, AppLocale::Ko);
    assert_eq!(settings.theme, Theme::System);
    assert_eq!(default_profile.profile.source_language, None);
    assert_eq!(default_profile.profile.target_language, "ko");
    assert_eq!(default_profile.profile.quality, Quality::Balanced);
    assert_eq!(default_profile.profile.tone, Tone::Natural);
}

#[tokio::test]
async fn missing_schema_version_migrates_and_is_saved_once() {
    let backend = MemoryBackend::default();
    backend.state.lock().unwrap().values.insert(
        "settings".into(),
        json!({
            "locale": "en",
            "theme": "dark",
            "launchAtLogin": true
        }),
    );

    let settings = SettingsStore::new(backend.clone()).load().await.unwrap();

    assert_eq!(settings.schema_version, 1);
    assert_eq!(settings.locale, AppLocale::En);
    assert_eq!(settings.theme, Theme::Dark);
    assert!(settings.launch_at_login);
    assert_eq!(backend.state.lock().unwrap().save_count, 1);
}

#[tokio::test]
async fn failed_durable_save_does_not_mutate_the_visible_settings_value() {
    let backend = FailingSaveBackend::default();
    let original = serde_json::to_value(AppSettings::default()).unwrap();
    backend
        .state
        .lock()
        .unwrap()
        .values
        .insert("settings".into(), original.clone());
    let changed = AppSettings {
        locale: AppLocale::En,
        ..AppSettings::default()
    };

    let error = SettingsStore::new(backend.clone())
        .save(&changed)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "settings_persistence_failed");
    assert_eq!(
        backend.state.lock().unwrap().values.get("settings"),
        Some(&original)
    );
}

#[tokio::test]
async fn app_owned_file_backend_replaces_a_complete_settings_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let store = SettingsStore::new(FileSettingsBackend::new(path.clone()));
    let settings = AppSettings {
        locale: AppLocale::En,
        ..AppSettings::default()
    };

    store.save(&settings).await.unwrap();

    assert_eq!(store.load().await.unwrap(), settings);
    let document: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(document["locale"], "en");
    assert!(document.get("settings").is_none());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn app_owned_file_backend_rejects_an_oversized_document_before_parsing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut document = serde_json::to_vec(&AppSettings::default()).unwrap();
    document.extend(std::iter::repeat_n(b' ', 256 * 1_024));
    std::fs::write(&path, document).unwrap();

    let error = SettingsStore::new(FileSettingsBackend::new(path))
        .load()
        .await
        .unwrap_err();

    assert_eq!(error.code(), "settings_persistence_failed");
}

#[test]
fn profiles_can_be_created_renamed_and_deleted_without_losing_the_default() {
    let mut settings = AppSettings::default();
    let id = settings
        .create_profile("기술 번역", profile(Some("en"), "ko"), Field::Technical)
        .unwrap();
    settings.rename_profile(id, "제품 문서").unwrap();
    assert_eq!(settings.profile(id).unwrap().name, "제품 문서");
    settings.delete_profile(id).unwrap();
    assert!(settings.profile(id).is_none());
    assert!(settings.default_profile().is_some());
}

#[test]
fn glossary_rejects_duplicate_source_terms_and_exports_protected_terms() {
    let mut settings = AppSettings::default();
    settings
        .add_glossary_entry(GlossaryEntry::new("en", "ko", "SmartCAT", "", true))
        .unwrap();
    let duplicate = settings
        .add_glossary_entry(GlossaryEntry::new(
            "en",
            "ko",
            " smartcat ",
            "스마트캣",
            false,
        ))
        .unwrap_err();

    assert_eq!(duplicate.code(), "duplicate_glossary_source");
    assert_eq!(settings.protected_terms_for("en", "ko"), vec!["SmartCAT"]);
}

#[test]
fn settings_reject_term_counts_and_aggregate_bytes_that_translation_would_reject() {
    let mut too_many = AppSettings::default();
    too_many.profiles[0].profile.protected_terms = vec!["x".into(); 1_001];
    assert_eq!(
        too_many.validate().unwrap_err().code(),
        "settings_size_limit"
    );

    let mut too_many_bytes = AppSettings::default();
    too_many_bytes.profiles[0].profile.protected_terms = vec!["한".repeat(30); 1_000];
    assert_eq!(
        too_many_bytes.validate().unwrap_err().code(),
        "settings_size_limit"
    );

    let too_many_glossary_rows = AppSettings {
        glossary: (0..1_001)
            .map(|index| GlossaryEntry::new("en", "ko", format!("term-{index}"), "target", false))
            .collect(),
        ..AppSettings::default()
    };
    assert_eq!(
        too_many_glossary_rows.validate().unwrap_err().code(),
        "settings_size_limit"
    );
}

#[test]
fn settings_reject_duplicate_glossary_ids_and_oversized_utf8_terms() {
    let first = GlossaryEntry::new("en", "ko", "one", "하나", false);
    let mut duplicate_id = first.clone();
    duplicate_id.source_term = "two".into();
    let duplicate_settings = AppSettings {
        glossary: vec![first, duplicate_id],
        ..AppSettings::default()
    };
    assert_eq!(
        duplicate_settings.validate().unwrap_err().code(),
        "duplicate_glossary_id"
    );

    let oversized = AppSettings {
        glossary: vec![GlossaryEntry::new(
            "en",
            "ko",
            "🙂".repeat(1_025),
            "target",
            false,
        )],
        ..AppSettings::default()
    };
    assert_eq!(
        oversized.validate().unwrap_err().code(),
        "settings_size_limit"
    );

    let oversized_whitespace = AppSettings {
        glossary: vec![GlossaryEntry::new(
            "en",
            "ko",
            format!("term{}", " ".repeat(4_097)),
            "target",
            false,
        )],
        ..AppSettings::default()
    };
    assert_eq!(
        oversized_whitespace.validate().unwrap_err().code(),
        "settings_size_limit"
    );

    let mut empty_protected_term = AppSettings::default();
    empty_protected_term.profiles[0].profile.protected_terms = vec!["   ".into()];
    assert_eq!(
        empty_protected_term.validate().unwrap_err().code(),
        "invalid_protected_term"
    );
}

#[test]
fn unavailable_specific_model_falls_back_without_discarding_the_saved_id() {
    let saved = ModelChoice::Specific {
        id: "retired-model".into(),
    };
    let models = vec![AvailableModel {
        id: "available-model".into(),
        display_name: "Available".into(),
        supported_reasoning_efforts: vec!["balanced".into()],
        is_default: true,
    }];

    let resolution = resolve_model_choice(&saved, &models);

    assert_eq!(resolution.effective, ModelChoice::Automatic);
    assert_eq!(
        resolution.unavailable_saved_id.as_deref(),
        Some("retired-model")
    );
    assert_eq!(
        saved,
        ModelChoice::Specific {
            id: "retired-model".into()
        }
    );
}

#[test]
fn job_model_resolution_distinguishes_authoritative_empty_unavailable_and_signed_out_catalogs() {
    let saved = ModelChoice::Specific {
        id: "retired-model".into(),
    };

    assert_eq!(
        resolve_model_for_job(&saved, ModelCatalogAuthority::Available(&[])).unwrap(),
        TranslationModel::Automatic
    );
    assert_eq!(
        resolve_model_for_job(&saved, ModelCatalogAuthority::Unavailable)
            .unwrap_err()
            .code(),
        "model_catalog_unavailable"
    );
    assert_eq!(
        resolve_model_for_job(&saved, ModelCatalogAuthority::SignedOut)
            .unwrap_err()
            .code(),
        "model_catalog_signed_out"
    );
}

#[test]
fn matching_source_and_target_suggests_rewrite_instead_of_silently_switching_modes() {
    assert_eq!(
        language_pair_action(Some("ko"), "ko"),
        LanguagePairAction::RewriteSuggested
    );
    assert_eq!(
        language_pair_action(Some("en"), "ko"),
        LanguagePairAction::Translate
    );
    assert_eq!(
        language_pair_action(None, "ko"),
        LanguagePairAction::Translate
    );
}

#[test]
fn invalid_locale_and_retention_are_rejected() {
    assert!(serde_json::from_value::<AppSettings>(json!({ "locale": "fr" })).is_err());
    let mut settings = AppSettings {
        history_retention_days: 0,
        ..AppSettings::default()
    };
    assert_eq!(settings.validate().unwrap_err().code(), "invalid_retention");
    settings.history_retention_days = 366;
    assert_eq!(settings.validate().unwrap_err().code(), "invalid_retention");
}

struct FakeModelTransport {
    response: Value,
    calls: Mutex<Vec<(String, Value)>>,
    events: broadcast::Sender<AppServerNotification>,
}

impl FakeModelTransport {
    fn new(response: Value) -> Self {
        let (events, _) = broadcast::channel(1);
        Self {
            response,
            calls: Mutex::new(Vec::new()),
            events,
        }
    }
}

#[async_trait]
impl AppServerTransport for FakeModelTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError> {
        self.calls.lock().unwrap().push((method.to_owned(), params));
        Ok(self.response.clone())
    }

    async fn terminate(&self) -> Result<(), TransportError> {
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<AppServerNotification> {
        self.events.subscribe()
    }
}

#[tokio::test]
async fn model_catalog_maps_only_account_available_models_and_preserves_effort_order() {
    let transport = Arc::new(FakeModelTransport::new(json!({
        "data": [{
            "id": "gpt-5.6-luna",
            "displayName": "GPT-5.6 Luna",
            "supportedReasoningEfforts": [
                { "reasoningEffort": "low", "description": "Low" },
                { "reasoningEffort": "high", "description": "High" }
            ],
            "isDefault": true
        }],
        "nextCursor": null
    })));
    let models = ModelCatalogService::new(transport.clone())
        .list()
        .await
        .unwrap();

    assert_eq!(
        models,
        vec![AvailableModel {
            id: "gpt-5.6-luna".into(),
            display_name: "GPT-5.6 Luna".into(),
            supported_reasoning_efforts: vec!["low".into(), "high".into()],
            is_default: true,
        }]
    );
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[(
            "model/list".into(),
            json!({ "cursor": null, "limit": 100, "includeHidden": false })
        )]
    );
}

#[tokio::test]
async fn model_catalog_rejects_duplicate_ids_multiple_defaults_and_oversized_fields_or_pages() {
    for response in [
        json!({
            "data": [
                {"id":"same","displayName":"One","supportedReasoningEfforts":[],"isDefault":false},
                {"id":"same","displayName":"Two","supportedReasoningEfforts":[],"isDefault":false}
            ],
            "nextCursor": null
        }),
        json!({
            "data": [
                {"id":"one","displayName":"One","supportedReasoningEfforts":[],"isDefault":true},
                {"id":"two","displayName":"Two","supportedReasoningEfforts":[],"isDefault":true}
            ],
            "nextCursor": null
        }),
        json!({
            "data": [{"id":"one","displayName":"x".repeat(513),"supportedReasoningEfforts":[],"isDefault":false}],
            "nextCursor": null
        }),
        json!({
            "data": (0..101).map(|index| json!({
                "id": format!("model-{index}"), "displayName": "Model", "supportedReasoningEfforts": [], "isDefault": false
            })).collect::<Vec<_>>(),
            "nextCursor": null
        }),
    ] {
        let error = ModelCatalogService::new(Arc::new(FakeModelTransport::new(response)))
            .list()
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_model_catalog");
    }

    let transport = Arc::new(EndlessPagingTransport::new());
    let error = ModelCatalogService::new(transport.clone())
        .list()
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_model_catalog");
    assert_eq!(transport.calls.load(Ordering::Relaxed), 100);

    let oversized_cursor = Arc::new(FakeModelTransport::new(json!({
        "data": [],
        "nextCursor": "x".repeat(513)
    })));
    let error = ModelCatalogService::new(oversized_cursor.clone())
        .list()
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_model_catalog");
    assert_eq!(oversized_cursor.calls.lock().unwrap().len(), 1);
}

struct EndlessPagingTransport {
    calls: AtomicUsize,
    events: broadcast::Sender<AppServerNotification>,
}

impl EndlessPagingTransport {
    fn new() -> Self {
        let (events, _) = broadcast::channel(1);
        Self {
            calls: AtomicUsize::new(0),
            events,
        }
    }
}

#[async_trait]
impl AppServerTransport for EndlessPagingTransport {
    async fn request(&self, _method: &str, _params: Value) -> Result<Value, TransportError> {
        let page = self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(json!({
            "data": [{"id": format!("model-{page}"), "displayName":"Model", "supportedReasoningEfforts":[], "isDefault":false}],
            "nextCursor": format!("cursor-{}", page + 1)
        }))
    }

    async fn terminate(&self) -> Result<(), TransportError> {
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<AppServerNotification> {
        self.events.subscribe()
    }
}
