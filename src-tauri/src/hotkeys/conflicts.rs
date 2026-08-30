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
const MAX_BUNDLE_ID_BYTES: usize = 255;
const MAX_PROCESS_NAMES_PER_ENTRY: usize = 16;
const MAX_BUNDLE_IDS_PER_ENTRY: usize = 16;
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
    "7daf4ea4702fce04bc6ed2cff426fa3c6fddd09e3c81852f754a1ee107bd6059";
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
    pub bundle_ids: Vec<String>,
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
        if entry.process_names.len() > MAX_PROCESS_NAMES_PER_ENTRY
            || entry.bundle_ids.len() > MAX_BUNDLE_IDS_PER_ENTRY
        {
            return Err(CatalogError::InvalidProcessName);
        }
        match entry.kind {
            CatalogKind::Application
                if entry.process_names.is_empty() && entry.bundle_ids.is_empty() =>
            {
                return Err(CatalogError::InvalidProcessName);
            }
            CatalogKind::OsReserved
                if !entry.process_names.is_empty() || !entry.bundle_ids.is_empty() =>
            {
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
        if entry
            .bundle_ids
            .iter()
            .any(|identifier| sanitize_bundle_id(identifier).is_none())
        {
            return Err(CatalogError::InvalidProcessName);
        }

        let parsed =
            super::parse_trigger(&entry.trigger).map_err(|_| CatalogError::InvalidTrigger)?;
        let normalized_trigger = parsed.to_string();
        if entry.trigger != normalized_trigger {
            return Err(CatalogError::InvalidTrigger);
        }
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

fn sanitize_bundle_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_BUNDLE_ID_BYTES
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum RegistrationProbeStatus {
    Available,
    AvailableViaObserver,
    Occupied { observer_available: bool },
    UnsupportedSequence { observer_available: bool },
    PermissionDenied,
    Invalid,
    OsReserved,
    BackendError,
}

pub trait RegistrationProbe: Send + Sync {
    /// Performs a non-persistent availability trial and restores the exact
    /// pre-call registration state before returning. A successful direct
    /// trial must unregister immediately; `AvailableViaObserver` means the
    /// non-consuming observer is already usable without a fallback warning.
    /// Task 4 supplies the native implementation and must not place backend
    /// details in this result.
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus;
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AppIdentity {
    executable_basename: Option<String>,
    bundle_id: Option<String>,
}

impl AppIdentity {
    pub fn new(executable_basename: Option<&str>, bundle_id: Option<&str>) -> Option<Self> {
        let executable_basename = match executable_basename {
            Some(value) => Some(sanitize_process_name(value)?),
            None => None,
        };
        let bundle_id = match bundle_id {
            Some(value) => Some(sanitize_bundle_id(value)?),
            None => None,
        };
        if executable_basename.is_none() && bundle_id.is_none() {
            return None;
        }
        Some(Self {
            executable_basename,
            bundle_id,
        })
    }

    pub fn executable_basename(&self) -> Option<&str> {
        self.executable_basename.as_deref()
    }

    pub fn bundle_id(&self) -> Option<&str> {
        self.bundle_id.as_deref()
    }
}

pub trait AppInspector: Send + Sync {
    /// Returns sanitized executable basenames and optional bundle identifiers.
    /// Window titles, full paths and keystrokes are outside this contract.
    fn running_apps(&self) -> Vec<AppIdentity>;
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
    Advisory,
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
        let running_apps = self.running_apps();
        let (level, causes, can_force) = self.classify(trigger, &running_apps);
        let alternatives = if level == ConflictLevel::None {
            Vec::new()
        } else {
            self.suggest_alternatives_with_apps(trigger, &running_apps)
        };
        ConflictReport {
            level,
            causes,
            alternatives,
            can_force,
        }
    }

    pub fn suggest_alternatives(&self, trigger: &Trigger) -> Vec<Trigger> {
        self.suggest_alternatives_with_apps(trigger, &self.running_apps())
    }

    fn running_apps(&self) -> BTreeSet<AppIdentity> {
        self.app_inspector
            .running_apps()
            .into_iter()
            .take(1_024)
            .collect()
    }

    fn classify(
        &self,
        trigger: &Trigger,
        running_apps: &BTreeSet<AppIdentity>,
    ) -> (ConflictLevel, Vec<ConflictCause>, bool) {
        let mut causes = Vec::new();
        let reservations = self.os_reservations(trigger);
        let mut hard_block = !reservations.is_empty();

        for entry in reservations {
            causes.push(cause_from_entry(entry, ConflictSeverity::Blocking));
        }

        let probe_status = if hard_block {
            RegistrationProbeStatus::OsReserved
        } else {
            self.registration_probe.probe_and_restore(trigger)
        };
        let observer_force_available = match probe_status {
            RegistrationProbeStatus::Available | RegistrationProbeStatus::AvailableViaObserver => {
                false
            }
            RegistrationProbeStatus::Occupied { observer_available } => {
                causes.push(fixed_probe_cause("다른 프로그램이 사용 중일 수 있습니다."));
                observer_available
            }
            RegistrationProbeStatus::UnsupportedSequence { observer_available } => {
                causes.push(fixed_probe_cause(
                    "이 연속 단축키는 직접 등록할 수 없어 키 관찰 기능이 필요합니다.",
                ));
                observer_available
            }
            RegistrationProbeStatus::PermissionDenied => {
                causes.push(fixed_probe_cause(
                    "단축키를 확인할 운영체제 권한이 없습니다.",
                ));
                hard_block = true;
                false
            }
            RegistrationProbeStatus::Invalid => {
                causes.push(fixed_probe_cause(
                    "운영체제에서 지원하지 않는 단축키입니다.",
                ));
                hard_block = true;
                false
            }
            RegistrationProbeStatus::OsReserved => {
                if causes.is_empty() {
                    causes.push(self.generic_os_cause());
                }
                hard_block = true;
                false
            }
            RegistrationProbeStatus::BackendError => {
                causes.push(fixed_probe_cause(
                    "단축키 충돌을 확인하는 중 오류가 발생했습니다.",
                ));
                hard_block = true;
                false
            }
        };

        let mut has_application_conflict = false;
        for entry in self.catalog.entries().iter().filter(|entry| {
            entry.platform == self.platform
                && entry.kind == CatalogKind::Application
                && entry_matches_running_app(entry, running_apps)
        }) {
            let Ok(catalog_trigger) = super::parse_trigger(&entry.trigger) else {
                continue;
            };
            match catalog_relationship(trigger, &catalog_trigger) {
                CatalogRelationship::FullConflict => {
                    causes.push(cause_from_entry(entry, ConflictSeverity::Warning));
                    has_application_conflict = true;
                }
                CatalogRelationship::StrictPrefixRisk => {
                    causes.push(cause_from_entry(entry, ConflictSeverity::Advisory));
                }
                CatalogRelationship::None => {}
            }
        }

        let probe_conflict = !matches!(
            probe_status,
            RegistrationProbeStatus::Available | RegistrationProbeStatus::AvailableViaObserver
        );
        let level = if hard_block || probe_conflict {
            ConflictLevel::Confirmed
        } else if has_application_conflict {
            ConflictLevel::Possible
        } else {
            ConflictLevel::None
        };
        let can_force = !hard_block
            && match probe_status {
                RegistrationProbeStatus::Available
                | RegistrationProbeStatus::AvailableViaObserver => has_application_conflict,
                RegistrationProbeStatus::Occupied { .. }
                | RegistrationProbeStatus::UnsupportedSequence { .. } => observer_force_available,
                RegistrationProbeStatus::PermissionDenied
                | RegistrationProbeStatus::Invalid
                | RegistrationProbeStatus::OsReserved
                | RegistrationProbeStatus::BackendError => false,
            };
        (level, causes, can_force)
    }

    fn os_reservations<'b>(&'b self, trigger: &Trigger) -> Vec<&'b CatalogEntry> {
        let mut matches = Vec::new();
        let mut seen = BTreeSet::new();
        if self.platform == Platform::Windows && contains_meta(trigger) {
            if let Some(entry) = self.os_source_entry("Meta+L") {
                seen.insert(entry.trigger.clone());
                matches.push(entry);
            }
        }
        if self.platform == Platform::Windows && contains_f12(trigger) {
            if let Some(entry) = self.os_source_entry("F12") {
                seen.insert(entry.trigger.clone());
                matches.push(entry);
            }
        }
        for entry in self.catalog.entries().iter().filter(|entry| {
            entry.platform == self.platform && entry.kind == CatalogKind::OsReserved
        }) {
            if seen.contains(&entry.trigger) {
                continue;
            }
            if super::parse_trigger(&entry.trigger).is_ok_and(|reserved| {
                catalog_relationship(trigger, &reserved) == CatalogRelationship::FullConflict
            }) {
                seen.insert(entry.trigger.clone());
                matches.push(entry);
            }
        }
        matches
    }

    fn os_source_entry(&self, trigger: &str) -> Option<&CatalogEntry> {
        self.catalog.entries().iter().find(|entry| {
            entry.platform == self.platform
                && entry.kind == CatalogKind::OsReserved
                && entry.trigger == trigger
        })
    }

    fn generic_os_cause(&self) -> ConflictCause {
        let source =
            self.catalog.entries().iter().find(|entry| {
                entry.platform == self.platform && entry.kind == CatalogKind::OsReserved
            });
        ConflictCause {
            severity: ConflictSeverity::Blocking,
            description: "운영체제가 예약한 단축키입니다.".to_owned(),
            application: Some(
                match self.platform {
                    Platform::Windows => "Windows",
                    Platform::Macos => "macOS",
                }
                .to_owned(),
            ),
            feature: None,
            source_url: source.map(|entry| entry.source_url.clone()),
            verified_at: source.map(|entry| entry.verified_at.clone()),
        }
    }

    fn suggest_alternatives_with_apps(
        &self,
        trigger: &Trigger,
        running_apps: &BTreeSet<AppIdentity>,
    ) -> Vec<Trigger> {
        let mut suggestions = Vec::with_capacity(MAX_ALTERNATIVES);
        let mut seen = BTreeSet::new();
        seen.insert(trigger.to_string());

        for candidate in modifier_candidates(trigger, self.platform)
            .into_iter()
            .chain(common_key_candidates())
        {
            if suggestions.len() == MAX_ALTERNATIVES {
                break;
            }
            let normalized = candidate.to_string();
            if !seen.insert(normalized) {
                continue;
            }
            if !is_common_physical_trigger(&candidate) {
                continue;
            }
            if self.classify(&candidate, running_apps).0 == ConflictLevel::None {
                suggestions.push(candidate);
            }
        }
        suggestions
    }
}

fn cause_from_entry(entry: &CatalogEntry, severity: ConflictSeverity) -> ConflictCause {
    ConflictCause {
        severity,
        description: match (entry.kind, severity) {
            (CatalogKind::OsReserved, _) => "운영체제가 사용하는 단축키입니다.".to_owned(),
            (CatalogKind::Application, ConflictSeverity::Advisory) => {
                "실행 중인 프로그램의 연속 단축키 시작과 같아 입력이 지연될 수 있습니다.".to_owned()
            }
            (CatalogKind::Application, _) => {
                "실행 중인 프로그램의 알려진 단축키와 겹칠 수 있습니다.".to_owned()
            }
        },
        application: Some(entry.application.clone()),
        feature: Some(entry.feature.clone()),
        source_url: Some(entry.source_url.clone()),
        verified_at: Some(entry.verified_at.clone()),
    }
}

fn fixed_probe_cause(description: &str) -> ConflictCause {
    ConflictCause {
        severity: ConflictSeverity::Blocking,
        description: description.to_owned(),
        application: None,
        feature: None,
        source_url: None,
        verified_at: None,
    }
}

fn entry_matches_running_app(entry: &CatalogEntry, running_apps: &BTreeSet<AppIdentity>) -> bool {
    running_apps.iter().any(|identity| {
        if let Some(running) = identity.bundle_id() {
            entry.bundle_ids.iter().any(|catalog| {
                sanitize_bundle_id(catalog).is_some_and(|catalog| catalog == running)
            })
        } else {
            identity.executable_basename().is_some_and(|running| {
                entry.process_names.iter().any(|catalog| {
                    sanitize_process_name(catalog).is_some_and(|catalog| catalog == running)
                })
            })
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogRelationship {
    None,
    StrictPrefixRisk,
    FullConflict,
}

fn catalog_relationship(candidate: &Trigger, catalog: &Trigger) -> CatalogRelationship {
    let candidate = trigger_steps(candidate);
    let catalog = trigger_steps(catalog);
    if contains_contiguous(candidate, catalog) {
        CatalogRelationship::FullConflict
    } else if candidate.len() < catalog.len() && catalog.starts_with(candidate) {
        CatalogRelationship::StrictPrefixRisk
    } else {
        CatalogRelationship::None
    }
}

fn contains_contiguous(haystack: &[Chord], needle: &[Chord]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn trigger_steps(trigger: &Trigger) -> &[Chord] {
    match trigger {
        Trigger::Chord { chord } => std::slice::from_ref(chord),
        Trigger::Sequence { steps, .. } => steps,
    }
}

fn contains_meta(trigger: &Trigger) -> bool {
    trigger_steps(trigger)
        .iter()
        .any(|chord| chord.modifiers.meta)
}

fn contains_f12(trigger: &Trigger) -> bool {
    trigger_steps(trigger)
        .iter()
        .any(|chord| chord.key == KeyCode::Physical(PhysicalKey::F12))
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

fn common_key_candidates() -> Vec<Trigger> {
    common_physical_keys()
        .into_iter()
        .filter_map(|key| {
            Trigger::chord(Chord {
                modifiers: Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: true,
                    meta: false,
                },
                key: KeyCode::Physical(key),
            })
            .ok()
        })
        .collect()
}

fn is_common_physical_trigger(trigger: &Trigger) -> bool {
    trigger_steps(trigger)
        .iter()
        .all(|chord| matches!(chord.key, KeyCode::Physical(key) if is_common_physical_key(key)))
}

fn is_common_physical_key(key: PhysicalKey) -> bool {
    common_physical_keys().contains(&key)
}

fn common_physical_keys() -> [PhysicalKey; 36] {
    [
        PhysicalKey::KeyT,
        PhysicalKey::KeyY,
        PhysicalKey::KeyU,
        PhysicalKey::KeyI,
        PhysicalKey::KeyO,
        PhysicalKey::KeyP,
        PhysicalKey::KeyA,
        PhysicalKey::KeyS,
        PhysicalKey::KeyD,
        PhysicalKey::KeyF,
        PhysicalKey::KeyG,
        PhysicalKey::KeyH,
        PhysicalKey::KeyJ,
        PhysicalKey::KeyK,
        PhysicalKey::KeyL,
        PhysicalKey::KeyZ,
        PhysicalKey::KeyX,
        PhysicalKey::KeyC,
        PhysicalKey::KeyV,
        PhysicalKey::KeyB,
        PhysicalKey::KeyN,
        PhysicalKey::KeyM,
        PhysicalKey::KeyQ,
        PhysicalKey::KeyW,
        PhysicalKey::KeyE,
        PhysicalKey::KeyR,
        PhysicalKey::Digit1,
        PhysicalKey::Digit2,
        PhysicalKey::Digit3,
        PhysicalKey::Digit4,
        PhysicalKey::Digit5,
        PhysicalKey::Digit6,
        PhysicalKey::Digit7,
        PhysicalKey::Digit8,
        PhysicalKey::Digit9,
        PhysicalKey::Digit0,
    ]
}
