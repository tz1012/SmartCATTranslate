use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use smartcat_translate::codex::protocol::AppServerNotification;
use smartcat_translate::codex::transport::{AppServerTransport, TransportError};
use smartcat_translate::core::types::{Quality, Tone, TranslationProfile};
use smartcat_translate::settings::store::{SettingsBackend, SettingsStore};
use smartcat_translate::settings::types::{
    language_pair_action, resolve_model_choice, AppLocale, AppSettings, AvailableModel, Field,
    GlossaryEntry, LanguagePairAction, ModelChoice, Theme,
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

#[async_trait]
impl SettingsBackend for MemoryBackend {
    async fn get(&self, key: &str) -> Result<Option<Value>, String> {
        Ok(self.state.lock().unwrap().values.get(key).cloned())
    }

    async fn set(&self, key: &str, value: Value) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .values
            .insert(key.to_owned(), value);
        Ok(())
    }

    async fn save(&self) -> Result<(), String> {
        self.state.lock().unwrap().save_count += 1;
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
