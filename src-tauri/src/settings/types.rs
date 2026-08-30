use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::core::types::Field;
use crate::core::types::{Quality, Tone, TranslationModel, TranslationProfile};
use crate::hotkeys::{BlockedApp, Blocklist, HotkeyBinding, SequenceEngine};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_HISTORY_RETENTION_DAYS: u16 = 30;
const MAX_HISTORY_RETENTION_DAYS: u16 = 365;
const MAX_PROFILES: usize = 64;
const MAX_GLOSSARY_ENTRIES: usize = 1_000;
const MAX_LABEL_CHARS: usize = 120;
const MAX_TERM_CHARS: usize = 1_024;
const MAX_TERM_BYTES: usize = 4_096;
const MAX_AGGREGATE_TERMS: usize = 1_000;
const MAX_AGGREGATE_TERM_BYTES: usize = 64 * 1_024;
const MAX_LANGUAGE_CHARS: usize = 64;
const MAX_LANGUAGE_BYTES: usize = 256;
const MAX_HOTKEY_BINDINGS: usize = 32;
pub const MAX_SETTINGS_DOCUMENT_BYTES: usize = 256 * 1_024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppLocale {
    #[default]
    Ko,
    En,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelChoice {
    #[default]
    Automatic,
    Specific {
        id: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CloseBehavior {
    #[default]
    KeepInTray,
    Quit,
    AskEveryTime,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuickAccessPosition {
    #[default]
    Popup,
    MainWindow,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedProfile {
    pub id: Uuid,
    pub name: String,
    pub field: Field,
    pub profile: TranslationProfile,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub id: Uuid,
    pub source_language: String,
    pub target_language: String,
    pub source_term: String,
    pub target_term: String,
    pub protect_only: bool,
}

impl GlossaryEntry {
    pub fn new(
        source_language: impl Into<String>,
        target_language: impl Into<String>,
        source_term: impl Into<String>,
        target_term: impl Into<String>,
        protect_only: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_language: source_language.into(),
            target_language: target_language.into(),
            source_term: source_term.into(),
            target_term: target_term.into(),
            protect_only,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub locale: AppLocale,
    pub theme: Theme,
    pub default_profile_id: Uuid,
    pub profiles: Vec<SavedProfile>,
    pub glossary: Vec<GlossaryEntry>,
    pub selected_model: ModelChoice,
    pub launch_at_login: bool,
    pub close_behavior: CloseBehavior,
    pub quick_access_position: QuickAccessPosition,
    pub history_retention_days: u16,
    pub hotkeys: Vec<HotkeyBinding>,
    pub blocked_apps: Vec<BlockedApp>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let id = Uuid::new_v4();
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            locale: AppLocale::Ko,
            theme: Theme::System,
            default_profile_id: id,
            profiles: vec![SavedProfile {
                id,
                name: "기본 프로필".to_owned(),
                field: Field::General,
                profile: TranslationProfile {
                    source_language: None,
                    target_language: "ko".to_owned(),
                    quality: Quality::Balanced,
                    tone: Tone::Natural,
                    protected_terms: Vec::new(),
                },
            }],
            glossary: Vec::new(),
            selected_model: ModelChoice::Automatic,
            launch_at_login: false,
            close_behavior: CloseBehavior::KeepInTray,
            quick_access_position: QuickAccessPosition::Popup,
            history_retention_days: DEFAULT_HISTORY_RETENTION_DAYS,
            hotkeys: Vec::new(),
            blocked_apps: Vec::new(),
        }
    }
}

impl AppSettings {
    pub fn default_profile(&self) -> Option<&SavedProfile> {
        self.profile(self.default_profile_id)
    }

    pub fn profile(&self, id: Uuid) -> Option<&SavedProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn create_profile(
        &mut self,
        name: impl Into<String>,
        profile: TranslationProfile,
        field: Field,
    ) -> Result<Uuid, SettingsError> {
        if self.profiles.len() >= MAX_PROFILES {
            return Err(SettingsError::TooManyProfiles);
        }
        let name = normalized_label(name.into())?;
        validate_translation_profile(&profile)?;
        let id = Uuid::new_v4();
        self.profiles.push(SavedProfile {
            id,
            name,
            field,
            profile,
        });
        if let Err(error) = self.validate() {
            self.profiles.pop();
            return Err(error);
        }
        Ok(id)
    }

    pub fn rename_profile(
        &mut self,
        id: Uuid,
        name: impl Into<String>,
    ) -> Result<(), SettingsError> {
        let name = normalized_label(name.into())?;
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == id)
            .ok_or(SettingsError::ProfileNotFound)?;
        profile.name = name;
        Ok(())
    }

    pub fn delete_profile(&mut self, id: Uuid) -> Result<(), SettingsError> {
        if id == self.default_profile_id {
            return Err(SettingsError::CannotDeleteDefaultProfile);
        }
        let before = self.profiles.len();
        self.profiles.retain(|profile| profile.id != id);
        if self.profiles.len() == before {
            return Err(SettingsError::ProfileNotFound);
        }
        Ok(())
    }

    pub fn add_glossary_entry(&mut self, mut entry: GlossaryEntry) -> Result<Uuid, SettingsError> {
        if self.glossary.len() >= MAX_GLOSSARY_ENTRIES {
            return Err(SettingsError::SizeLimit);
        }
        normalize_glossary_entry(&mut entry)?;
        let entry_key = (
            entry.source_language.to_ascii_lowercase(),
            entry.target_language.to_ascii_lowercase(),
            entry.source_term.to_lowercase(),
        );
        let duplicate = self.glossary.iter().any(|existing| {
            (
                existing.source_language.to_ascii_lowercase(),
                existing.target_language.to_ascii_lowercase(),
                existing.source_term.trim().to_lowercase(),
            ) == entry_key
        });
        if duplicate {
            return Err(SettingsError::DuplicateGlossarySource);
        }
        let id = entry.id;
        self.glossary.push(entry);
        if let Err(error) = self.validate() {
            self.glossary.pop();
            return Err(error);
        }
        Ok(id)
    }

    pub fn protected_terms_for(&self, source: &str, target: &str) -> Vec<String> {
        self.glossary
            .iter()
            .filter(|entry| {
                entry.protect_only
                    && entry.source_language.eq_ignore_ascii_case(source)
                    && entry.target_language.eq_ignore_ascii_case(target)
            })
            .map(|entry| entry.source_term.clone())
            .collect()
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedSchemaVersion);
        }
        if !(1..=MAX_HISTORY_RETENTION_DAYS).contains(&self.history_retention_days) {
            return Err(SettingsError::InvalidRetention);
        }
        if self.profiles.is_empty() || self.profiles.len() > MAX_PROFILES {
            return Err(SettingsError::InvalidProfiles);
        }
        if self.glossary.len() > MAX_GLOSSARY_ENTRIES {
            return Err(SettingsError::SizeLimit);
        }
        if self.hotkeys.len() > MAX_HOTKEY_BINDINGS
            || self
                .hotkeys
                .iter()
                .map(|binding| binding.id)
                .collect::<HashSet<_>>()
                .len()
                != self.hotkeys.len()
            || self
                .hotkeys
                .iter()
                .any(|binding| self.profile(binding.profile_id).is_none())
            || SequenceEngine::new(self.hotkeys.clone()).is_err()
        {
            return Err(SettingsError::InvalidHotkeys);
        }
        if self
            .blocked_apps
            .iter()
            .any(|entry| entry.validate().is_err())
            || Blocklist::new(self.blocked_apps.clone()).is_err()
        {
            return Err(SettingsError::InvalidBlockedApps);
        }
        let mut profile_ids = HashSet::new();
        let mut aggregate_term_count = 0_usize;
        let mut aggregate_term_bytes = 0_usize;
        for profile in &self.profiles {
            normalized_label(profile.name.clone())?;
            validate_translation_profile(&profile.profile)?;
            aggregate_term_count = aggregate_term_count
                .checked_add(profile.profile.protected_terms.len())
                .ok_or(SettingsError::SizeLimit)?;
            aggregate_term_bytes = profile.profile.protected_terms.iter().try_fold(
                aggregate_term_bytes,
                |total, term| {
                    total
                        .checked_add(term.len())
                        .ok_or(SettingsError::SizeLimit)
                },
            )?;
            if !profile_ids.insert(profile.id) {
                return Err(SettingsError::InvalidProfiles);
            }
        }
        if !profile_ids.contains(&self.default_profile_id) {
            return Err(SettingsError::InvalidDefaultProfile);
        }
        let mut glossary_keys = HashSet::new();
        let mut glossary_ids = HashSet::new();
        for original in &self.glossary {
            if !glossary_ids.insert(original.id) {
                return Err(SettingsError::DuplicateGlossaryId);
            }
            let mut entry = original.clone();
            normalize_glossary_entry(&mut entry)?;
            aggregate_term_count = aggregate_term_count
                .checked_add(1)
                .ok_or(SettingsError::SizeLimit)?;
            aggregate_term_bytes = aggregate_term_bytes
                .checked_add(entry.source_term.len())
                .and_then(|total| total.checked_add(entry.target_term.len()))
                .ok_or(SettingsError::SizeLimit)?;
            let key = (
                entry.source_language.to_ascii_lowercase(),
                entry.target_language.to_ascii_lowercase(),
                entry.source_term.to_lowercase(),
            );
            if !glossary_keys.insert(key) {
                return Err(SettingsError::DuplicateGlossarySource);
            }
        }
        if let ModelChoice::Specific { id } = &self.selected_model {
            if id.trim().is_empty() || id.chars().count() > MAX_LABEL_CHARS {
                return Err(SettingsError::InvalidModel);
            }
        }
        if aggregate_term_count > MAX_AGGREGATE_TERMS
            || aggregate_term_bytes > MAX_AGGREGATE_TERM_BYTES
            || serde_json::to_vec(self)
                .map_err(|_| SettingsError::InvalidDocument)?
                .len()
                > MAX_SETTINGS_DOCUMENT_BYTES
        {
            return Err(SettingsError::SizeLimit);
        }
        Ok(())
    }
}

fn validate_translation_profile(profile: &TranslationProfile) -> Result<(), SettingsError> {
    if let Some(source) = &profile.source_language {
        validate_language_tag(source)?;
    }
    validate_language_tag(&profile.target_language)?;
    if profile
        .protected_terms
        .iter()
        .any(|term| term.trim().is_empty())
    {
        return Err(SettingsError::InvalidProtectedTerm);
    }
    if profile.protected_terms.len() > MAX_AGGREGATE_TERMS
        || profile
            .protected_terms
            .iter()
            .any(|term| term.chars().count() > MAX_TERM_CHARS || term.len() > MAX_TERM_BYTES)
    {
        return Err(SettingsError::SizeLimit);
    }
    Ok(())
}

fn normalize_glossary_entry(entry: &mut GlossaryEntry) -> Result<(), SettingsError> {
    if entry.source_term.chars().count() > MAX_TERM_CHARS
        || entry.source_term.len() > MAX_TERM_BYTES
        || entry.target_term.chars().count() > MAX_TERM_CHARS
        || entry.target_term.len() > MAX_TERM_BYTES
    {
        return Err(SettingsError::SizeLimit);
    }
    entry.source_language = entry.source_language.trim().to_ascii_lowercase();
    entry.target_language = entry.target_language.trim().to_ascii_lowercase();
    validate_language_tag(&entry.source_language)?;
    validate_language_tag(&entry.target_language)?;
    entry.source_term = entry.source_term.trim().to_owned();
    entry.target_term = entry.target_term.trim().to_owned();
    if entry.source_term.is_empty() || (!entry.protect_only && entry.target_term.is_empty()) {
        return Err(SettingsError::InvalidGlossaryEntry);
    }
    Ok(())
}

fn normalized_label(value: String) -> Result<String, SettingsError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_LABEL_CHARS {
        return Err(SettingsError::InvalidProfileName);
    }
    Ok(value.to_owned())
}

fn validate_language_tag(value: &str) -> Result<(), SettingsError> {
    if value.chars().count() > MAX_LANGUAGE_CHARS || value.len() > MAX_LANGUAGE_BYTES {
        return Err(SettingsError::SizeLimit);
    }
    let mut parts = value.split('-');
    let first = parts.next().unwrap_or_default();
    if !(2..=8).contains(&first.len()) || !first.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(SettingsError::InvalidLanguage);
    }
    if parts.any(|part| {
        part.is_empty() || part.len() > 8 || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }) {
        return Err(SettingsError::InvalidLanguage);
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvailableModel {
    pub id: String,
    pub display_name: String,
    pub supported_reasoning_efforts: Vec<String>,
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelResolution {
    pub effective: ModelChoice,
    pub unavailable_saved_id: Option<String>,
}

pub fn resolve_model_choice(choice: &ModelChoice, models: &[AvailableModel]) -> ModelResolution {
    match choice {
        ModelChoice::Automatic => ModelResolution {
            effective: ModelChoice::Automatic,
            unavailable_saved_id: None,
        },
        ModelChoice::Specific { id } if models.iter().any(|model| model.id == *id) => {
            ModelResolution {
                effective: choice.clone(),
                unavailable_saved_id: None,
            }
        }
        ModelChoice::Specific { id } => ModelResolution {
            effective: ModelChoice::Automatic,
            unavailable_saved_id: Some(id.clone()),
        },
    }
}

pub enum ModelCatalogAuthority<'a> {
    Available(&'a [AvailableModel]),
    Unavailable,
    SignedOut,
}

pub fn resolve_model_for_job(
    choice: &ModelChoice,
    authority: ModelCatalogAuthority<'_>,
) -> Result<TranslationModel, ModelSelectionError> {
    if matches!(authority, ModelCatalogAuthority::SignedOut) {
        return Err(ModelSelectionError::SignedOut);
    }
    match choice {
        ModelChoice::Automatic => Ok(TranslationModel::Automatic),
        ModelChoice::Specific { .. } if matches!(authority, ModelCatalogAuthority::Unavailable) => {
            Err(ModelSelectionError::CatalogUnavailable)
        }
        ModelChoice::Specific { id } => {
            let ModelCatalogAuthority::Available(models) = authority else {
                unreachable!("handled unavailable catalog above")
            };
            Ok(if models.iter().any(|model| model.id == *id) {
                TranslationModel::Specific(id.clone())
            } else {
                TranslationModel::Automatic
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelSelectionError {
    #[error("the account is signed out")]
    SignedOut,
    #[error("the authoritative model catalog is unavailable")]
    CatalogUnavailable,
}

impl ModelSelectionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::SignedOut => "model_catalog_signed_out",
            Self::CatalogUnavailable => "model_catalog_unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguagePairAction {
    Translate,
    RewriteSuggested,
}

pub fn language_pair_action(source: Option<&str>, target: &str) -> LanguagePairAction {
    if source.is_some_and(|source| source.eq_ignore_ascii_case(target)) {
        LanguagePairAction::RewriteSuggested
    } else {
        LanguagePairAction::Translate
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("unsupported settings schema")]
    UnsupportedSchemaVersion,
    #[error("invalid history retention")]
    InvalidRetention,
    #[error("invalid translation profiles")]
    InvalidProfiles,
    #[error("invalid default profile")]
    InvalidDefaultProfile,
    #[error("too many profiles")]
    TooManyProfiles,
    #[error("profile not found")]
    ProfileNotFound,
    #[error("default profile cannot be deleted")]
    CannotDeleteDefaultProfile,
    #[error("invalid profile name")]
    InvalidProfileName,
    #[error("invalid language")]
    InvalidLanguage,
    #[error("invalid protected term")]
    InvalidProtectedTerm,
    #[error("invalid glossary entry")]
    InvalidGlossaryEntry,
    #[error("duplicate glossary source")]
    DuplicateGlossarySource,
    #[error("too many glossary entries")]
    TooManyGlossaryEntries,
    #[error("duplicate glossary identifier")]
    DuplicateGlossaryId,
    #[error("settings size limit exceeded")]
    SizeLimit,
    #[error("invalid model selection")]
    InvalidModel,
    #[error("invalid blocked applications")]
    InvalidBlockedApps,
    #[error("invalid hotkey bindings")]
    InvalidHotkeys,
    #[error("settings persistence failed")]
    Persistence,
    #[error("settings document is invalid")]
    InvalidDocument,
}

impl SettingsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedSchemaVersion => "unsupported_schema_version",
            Self::InvalidRetention => "invalid_retention",
            Self::InvalidProfiles => "invalid_profiles",
            Self::InvalidDefaultProfile => "invalid_default_profile",
            Self::TooManyProfiles => "too_many_profiles",
            Self::ProfileNotFound => "profile_not_found",
            Self::CannotDeleteDefaultProfile => "cannot_delete_default_profile",
            Self::InvalidProfileName => "invalid_profile_name",
            Self::InvalidLanguage => "invalid_language",
            Self::InvalidProtectedTerm => "invalid_protected_term",
            Self::InvalidGlossaryEntry => "invalid_glossary_entry",
            Self::DuplicateGlossarySource => "duplicate_glossary_source",
            Self::TooManyGlossaryEntries => "too_many_glossary_entries",
            Self::DuplicateGlossaryId => "duplicate_glossary_id",
            Self::SizeLimit => "settings_size_limit",
            Self::InvalidModel => "invalid_model",
            Self::InvalidBlockedApps => "invalid_blocked_apps",
            Self::InvalidHotkeys => "invalid_hotkeys",
            Self::Persistence => "settings_persistence_failed",
            Self::InvalidDocument => "invalid_settings_document",
        }
    }
}
