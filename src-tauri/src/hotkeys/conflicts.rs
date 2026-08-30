use std::collections::BTreeSet;

use chrono::{Months, NaiveDate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::types::{Chord, KeyCode, Modifiers, PhysicalKey, Trigger};

pub const SHORTCUT_CATALOG_SCHEMA_VERSION: u32 = 1;
pub const MAX_CATALOG_BYTES: usize = 256 * 1024;
pub const MAX_CATALOG_ENTRIES: usize = 1_024;
const MAX_CATALOG_VERSION_BYTES: usize = 64;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_PROCESS_NAME_BYTES: usize = 128;
const MAX_PROCESS_NAMES_PER_ENTRY: usize = 16;
const MAX_ALTERNATIVES: usize = 3;
const PRIMARY_SOURCE_HOSTS: [&str; 6] = [
    "learn.microsoft.com",
    "support.microsoft.com",
    "support.apple.com",
    "support.google.com",
    "code.visualstudio.com",
    "support.deepl.com",
];

// This is the SHA-256 of resources/shortcut-catalog.json. Updating the reviewed
// catalog therefore requires an explicit source change as well as new data.
const SHORTCUT_CATALOG_SHA256: &str =
    "5d93551fbecb1928f20fca51a59074874606799d53ae3864c35b971f43ddb355";
const EMBEDDED_CATALOG: &[u8] = include_bytes!("../../resources/shortcut-catalog.json");

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CatalogKind {
    Application,
    OsReserved,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogEntry {
    pub platform: Platform,
    pub kind: CatalogKind,
    pub application: String,
    pub process_names: Vec<String>,
    pub trigger: String,
    pub feature: String,
    pub source_url: String,
    pub verified_at: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogDocument {
    schema_version: u32,
    catalog_version: String,
    entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug)]
pub struct ShortcutCatalog {
    document: CatalogDocument,
    sha256: String,
}

impl ShortcutCatalog {
    pub fn from_embedded(as_of: NaiveDate) -> Result<Self, CatalogError> {
        Self::parse_verified(EMBEDDED_CATALOG, SHORTCUT_CATALOG_SHA256, as_of)
    }

    pub fn parse_verified(
        bytes: &[u8],
        expected_sha256: &str,
        as_of: NaiveDate,
    ) -> Result<Self, CatalogError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge);
        }

        let actual_sha256 = format!("{:x}", Sha256::digest(bytes));
        if expected_sha256.len() != 64
            || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !actual_sha256.eq_ignore_ascii_case(expected_sha256)
        {
            return Err(CatalogError::HashMismatch);
        }

        let document: CatalogDocument =
            serde_json::from_slice(bytes).map_err(|_| CatalogError::InvalidJson)?;
        validate_document(&document, as_of)?;

        Ok(Self {
            document,
            sha256: actual_sha256,
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.document.schema_version
    }

    pub fn catalog_version(&self) -> &str {
        &self.document.catalog_version
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.document.entries
    }
}

fn validate_document(document: &CatalogDocument, as_of: NaiveDate) -> Result<(), CatalogError> {
    if document.schema_version != SHORTCUT_CATALOG_SCHEMA_VERSION {
        return Err(CatalogError::UnsupportedSchema);
    }
    if document.catalog_version.is_empty()
        || document.catalog_version.len() > MAX_CATALOG_VERSION_BYTES
        || !document
            .catalog_version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(CatalogError::InvalidVersion);
    }
    if document.entries.is_empty() {
        return Err(CatalogError::Empty);
    }
    if document.entries.len() > MAX_CATALOG_ENTRIES {
        return Err(CatalogError::TooManyEntries);
    }

    let oldest_allowed = as_of
        .checked_sub_months(Months::new(18))
        .ok_or(CatalogError::InvalidDate)?;
    let mut unique = BTreeSet::new();
    for entry in &document.entries {
        validate_text(&entry.application)?;
        validate_text(&entry.feature)?;
        validate_text(&entry.trigger)?;
        if entry.process_names.len() > MAX_PROCESS_NAMES_PER_ENTRY {
            return Err(CatalogError::InvalidProcessName);
        }
        match entry.kind {
            CatalogKind::Application if entry.process_names.is_empty() => {
                return Err(CatalogError::InvalidProcessName);
            }
            CatalogKind::OsReserved if !entry.process_names.is_empty() => {
                return Err(CatalogError::InvalidProcessName);
            }
            _ => {}
        }
        if entry
            .process_names
            .iter()
            .any(|name| sanitize_process_name(name).is_none())
        {
            return Err(CatalogError::InvalidProcessName);
        }

        let parsed =
            super::parse_trigger(&entry.trigger).map_err(|_| CatalogError::InvalidTrigger)?;
        let normalized_trigger = parsed.to_string();
        let source = Url::parse(&entry.source_url).map_err(|_| CatalogError::InvalidSource)?;
        let source_host = source.host_str().unwrap_or_default();
        if source.scheme() != "https"
            || !PRIMARY_SOURCE_HOSTS.contains(&source_host)
            || !source.username().is_empty()
            || source.password().is_some()
            || source.port().is_some()
            || entry.source_url.len() > MAX_TEXT_BYTES
        {
            return Err(CatalogError::InvalidSource);
        }
        let verified = NaiveDate::parse_from_str(&entry.verified_at, "%Y-%m-%d")
            .map_err(|_| CatalogError::InvalidDate)?;
        if verified < oldest_allowed {
            return Err(CatalogError::StaleEntry);
        }
        if verified > as_of {
            return Err(CatalogError::InvalidDate);
        }

        let key = (
            entry.platform,
            entry.application.to_lowercase(),
            normalized_trigger,
        );
        if !unique.insert(key) {
            return Err(CatalogError::DuplicateEntry);
        }
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), CatalogError> {
    if value.trim().is_empty()
        || value.len() > MAX_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(CatalogError::InvalidText);
    }
    Ok(())
}

fn sanitize_process_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_PROCESS_NAME_BYTES
        || trimmed.chars().any(char::is_control)
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
    {
        return None;
    }
    Some(trimmed.to_lowercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("shortcut catalog is too large")]
    TooLarge,
    #[error("shortcut catalog hash does not match")]
    HashMismatch,
    #[error("shortcut catalog JSON is invalid")]
    InvalidJson,
    #[error("shortcut catalog schema is unsupported")]
    UnsupportedSchema,
    #[error("shortcut catalog version is invalid")]
    InvalidVersion,
    #[error("shortcut catalog is empty")]
    Empty,
    #[error("shortcut catalog has too many entries")]
    TooManyEntries,
    #[error("shortcut catalog text is invalid")]
    InvalidText,
    #[error("shortcut catalog process name is invalid")]
    InvalidProcessName,
    #[error("shortcut catalog trigger is invalid")]
    InvalidTrigger,
    #[error("shortcut catalog source is invalid")]
    InvalidSource,
    #[error("shortcut catalog date is invalid")]
    InvalidDate,
    #[error("shortcut catalog entry is stale")]
    StaleEntry,
    #[error("shortcut catalog contains a duplicate entry")]
    DuplicateEntry,
}

pub trait RegistrationProbe: Send + Sync {
    /// Performs the platform registration trial and immediately releases a
    /// successful trial. Task 4 supplies the native implementation.
    fn can_register(&self, trigger: &Trigger) -> bool;
}

pub trait AppInspector: Send + Sync {
    /// Returns executable/process names only. Window titles, paths and
    /// keystrokes are outside this contract.
    fn running_process_names(&self) -> Vec<String>;
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConflictLevel {
    None,
    Possible,
    Confirmed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConflictSeverity {
    Warning,
    Blocking,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictCause {
    pub severity: ConflictSeverity,
    pub description: String,
    pub application: Option<String>,
    pub feature: Option<String>,
    pub source_url: Option<String>,
    pub verified_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictReport {
    pub level: ConflictLevel,
    pub causes: Vec<ConflictCause>,
    pub alternatives: Vec<Trigger>,
    pub can_force: bool,
}

impl ConflictReport {
    pub fn registration_allowed(&self, force_requested: bool) -> bool {
        self.level == ConflictLevel::None || (force_requested && self.can_force)
    }
}

pub struct ConflictAnalyzer<'a> {
    platform: Platform,
    catalog: &'a ShortcutCatalog,
    registration_probe: &'a dyn RegistrationProbe,
    app_inspector: &'a dyn AppInspector,
}

impl<'a> ConflictAnalyzer<'a> {
    pub fn new(
        platform: Platform,
        catalog: &'a ShortcutCatalog,
        registration_probe: &'a dyn RegistrationProbe,
        app_inspector: &'a dyn AppInspector,
    ) -> Self {
        Self {
            platform,
            catalog,
            registration_probe,
            app_inspector,
        }
    }

    pub fn analyze(&self, trigger: &Trigger) -> ConflictReport {
        let running_processes = self.running_processes();
        let (level, causes, can_force) = self.classify(trigger, &running_processes);
        let alternatives = if level == ConflictLevel::None {
            Vec::new()
        } else {
            self.suggest_alternatives_with_apps(trigger, &running_processes)
        };
        ConflictReport {
            level,
            causes,
            alternatives,
            can_force,
        }
    }

    pub fn suggest_alternatives(&self, trigger: &Trigger) -> Vec<Trigger> {
        self.suggest_alternatives_with_apps(trigger, &self.running_processes())
    }

    fn running_processes(&self) -> BTreeSet<String> {
        self.app_inspector
            .running_process_names()
            .into_iter()
            .filter_map(|name| sanitize_process_name(&name))
            .take(1_024)
            .collect()
    }

    fn classify(
        &self,
        trigger: &Trigger,
        running_processes: &BTreeSet<String>,
    ) -> (ConflictLevel, Vec<ConflictCause>, bool) {
        let normalized = trigger.to_string();
        let mut causes = Vec::new();
        let mut has_non_forceable_reservation = false;

        if let Some(entry) = self.os_reservation(trigger, &normalized) {
            causes.push(cause_from_entry(entry, ConflictSeverity::Blocking));
            has_non_forceable_reservation = true;
        }

        if !has_non_forceable_reservation && !self.registration_probe.can_register(trigger) {
            causes.push(ConflictCause {
                severity: ConflictSeverity::Blocking,
                description: "다른 프로그램이 사용 중일 수 있습니다.".to_owned(),
                application: None,
                feature: None,
                source_url: None,
                verified_at: None,
            });
        }

        for entry in self.catalog.entries().iter().filter(|entry| {
            entry.platform == self.platform
                && entry.kind == CatalogKind::Application
                && entry.trigger == normalized
                && entry.process_names.iter().any(|process| {
                    sanitize_process_name(process)
                        .is_some_and(|process| running_processes.contains(&process))
                })
        }) {
            causes.push(cause_from_entry(entry, ConflictSeverity::Warning));
        }

        let level = if has_non_forceable_reservation
            || causes
                .iter()
                .any(|cause| cause.severity == ConflictSeverity::Blocking)
        {
            ConflictLevel::Confirmed
        } else if causes.is_empty() {
            ConflictLevel::None
        } else {
            ConflictLevel::Possible
        };
        let can_force = level != ConflictLevel::None && !has_non_forceable_reservation;
        (level, causes, can_force)
    }

    fn os_reservation<'b>(
        &'b self,
        trigger: &Trigger,
        normalized: &str,
    ) -> Option<&'b CatalogEntry> {
        self.catalog.entries().iter().find(|entry| {
            if entry.platform != self.platform || entry.kind != CatalogKind::OsReserved {
                return false;
            }
            if self.platform == Platform::Windows && contains_f12(trigger) {
                return entry.trigger == "F12";
            }
            entry.trigger == normalized
        })
    }

    fn suggest_alternatives_with_apps(
        &self,
        trigger: &Trigger,
        running_processes: &BTreeSet<String>,
    ) -> Vec<Trigger> {
        let mut suggestions = Vec::with_capacity(MAX_ALTERNATIVES);
        let mut seen = BTreeSet::new();
        seen.insert(trigger.to_string());

        for candidate in modifier_candidates(trigger, self.platform)
            .into_iter()
            .chain(function_key_candidates(trigger, self.platform))
        {
            if suggestions.len() == MAX_ALTERNATIVES {
                break;
            }
            let normalized = candidate.to_string();
            if !seen.insert(normalized) {
                continue;
            }
            if self.classify(&candidate, running_processes).0 == ConflictLevel::None {
                suggestions.push(candidate);
            }
        }
        suggestions
    }
}

fn cause_from_entry(entry: &CatalogEntry, severity: ConflictSeverity) -> ConflictCause {
    ConflictCause {
        severity,
        description: if entry.kind == CatalogKind::OsReserved {
            "운영체제가 사용하는 단축키입니다.".to_owned()
        } else {
            "실행 중인 프로그램의 알려진 단축키와 겹칠 수 있습니다.".to_owned()
        },
        application: Some(entry.application.clone()),
        feature: Some(entry.feature.clone()),
        source_url: Some(entry.source_url.clone()),
        verified_at: Some(entry.verified_at.clone()),
    }
}

fn contains_f12(trigger: &Trigger) -> bool {
    let is_f12 = |chord: &Chord| chord.key == KeyCode::Physical(PhysicalKey::F12);
    match trigger {
        Trigger::Chord { chord } => is_f12(chord),
        Trigger::Sequence { steps, .. } => steps.iter().any(is_f12),
    }
}

fn modifier_candidates(trigger: &Trigger, platform: Platform) -> Vec<Trigger> {
    const SHIFT: u8 = 1;
    const ALT: u8 = 2;
    const CTRL: u8 = 4;
    const META: u8 = 8;
    let mut additions = vec![
        SHIFT,
        ALT,
        SHIFT | ALT,
        CTRL,
        SHIFT | CTRL,
        ALT | CTRL,
        SHIFT | ALT | CTRL,
    ];
    if platform == Platform::Macos {
        additions.extend([
            META,
            SHIFT | META,
            ALT | META,
            SHIFT | ALT | META,
            CTRL | META,
            SHIFT | CTRL | META,
            ALT | CTRL | META,
            SHIFT | ALT | CTRL | META,
        ]);
    }

    additions
        .into_iter()
        .filter_map(|addition| with_added_modifiers(trigger, addition))
        .collect()
}

fn with_added_modifiers(trigger: &Trigger, addition: u8) -> Option<Trigger> {
    let mut steps = match trigger {
        Trigger::Chord { chord } => vec![*chord],
        Trigger::Sequence { steps, .. } => steps.clone(),
    };
    let first = steps.first_mut()?;
    let original = first.modifiers;
    first.modifiers.shift |= addition & 1 != 0;
    first.modifiers.alt |= addition & 2 != 0;
    first.modifiers.ctrl |= addition & 4 != 0;
    first.modifiers.meta |= addition & 8 != 0;
    if first.modifiers == original {
        return None;
    }
    match trigger {
        Trigger::Chord { .. } => Trigger::chord(steps[0]).ok(),
        Trigger::Sequence { timeout_ms, .. } => Trigger::sequence(steps, *timeout_ms).ok(),
    }
}

fn function_key_candidates(trigger: &Trigger, platform: Platform) -> Vec<Trigger> {
    let mut keys = Vec::new();
    if let Some(number) = primary_function_number(trigger) {
        for distance in 1..=12 {
            if number > distance {
                keys.push(number - distance);
            }
            if number + distance <= 24 {
                keys.push(number + distance);
            }
        }
    } else {
        keys.extend([8, 9, 10, 11, 13, 14, 15, 16]);
    }
    keys.into_iter()
        .filter(|number| !(platform == Platform::Windows && *number == 12))
        .filter_map(function_key)
        .filter_map(|key| {
            Trigger::chord(Chord {
                modifiers: Modifiers::default(),
                key: KeyCode::Physical(key),
            })
            .ok()
        })
        .collect()
}

fn primary_function_number(trigger: &Trigger) -> Option<u8> {
    let chord = match trigger {
        Trigger::Chord { chord } => chord,
        Trigger::Sequence { steps, .. } => steps.first()?,
    };
    match chord.key {
        KeyCode::Physical(key) => function_number(key),
        KeyCode::Logical(_) => None,
    }
}

fn function_number(key: PhysicalKey) -> Option<u8> {
    Some(match key {
        PhysicalKey::F1 => 1,
        PhysicalKey::F2 => 2,
        PhysicalKey::F3 => 3,
        PhysicalKey::F4 => 4,
        PhysicalKey::F5 => 5,
        PhysicalKey::F6 => 6,
        PhysicalKey::F7 => 7,
        PhysicalKey::F8 => 8,
        PhysicalKey::F9 => 9,
        PhysicalKey::F10 => 10,
        PhysicalKey::F11 => 11,
        PhysicalKey::F12 => 12,
        PhysicalKey::F13 => 13,
        PhysicalKey::F14 => 14,
        PhysicalKey::F15 => 15,
        PhysicalKey::F16 => 16,
        PhysicalKey::F17 => 17,
        PhysicalKey::F18 => 18,
        PhysicalKey::F19 => 19,
        PhysicalKey::F20 => 20,
        PhysicalKey::F21 => 21,
        PhysicalKey::F22 => 22,
        PhysicalKey::F23 => 23,
        PhysicalKey::F24 => 24,
        _ => return None,
    })
}

fn function_key(number: u8) -> Option<PhysicalKey> {
    Some(match number {
        1 => PhysicalKey::F1,
        2 => PhysicalKey::F2,
        3 => PhysicalKey::F3,
        4 => PhysicalKey::F4,
        5 => PhysicalKey::F5,
        6 => PhysicalKey::F6,
        7 => PhysicalKey::F7,
        8 => PhysicalKey::F8,
        9 => PhysicalKey::F9,
        10 => PhysicalKey::F10,
        11 => PhysicalKey::F11,
        12 => PhysicalKey::F12,
        13 => PhysicalKey::F13,
        14 => PhysicalKey::F14,
        15 => PhysicalKey::F15,
        16 => PhysicalKey::F16,
        17 => PhysicalKey::F17,
        18 => PhysicalKey::F18,
        19 => PhysicalKey::F19,
        20 => PhysicalKey::F20,
        21 => PhysicalKey::F21,
        22 => PhysicalKey::F22,
        23 => PhysicalKey::F23,
        24 => PhysicalKey::F24,
        _ => return None,
    })
}
