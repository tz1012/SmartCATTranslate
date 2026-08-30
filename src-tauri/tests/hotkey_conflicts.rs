use std::collections::BTreeSet;
use std::sync::Mutex;

use chrono::NaiveDate;
use sha2::{Digest, Sha256};
use smartcat_translate::hotkeys::{
    parse_trigger, AppInspector, CatalogError, ConflictAnalyzer, ConflictLevel, ConflictSeverity,
    Platform, RegistrationProbe, ShortcutCatalog,
};

const AS_OF: &str = "2026-08-30";

#[derive(Default)]
struct SelectiveProbe {
    blocked: BTreeSet<String>,
    observed: Mutex<Vec<String>>,
}

impl SelectiveProbe {
    fn blocking(triggers: &[&str]) -> Self {
        Self {
            blocked: triggers.iter().map(|value| (*value).to_owned()).collect(),
            observed: Mutex::new(Vec::new()),
        }
    }
}

impl RegistrationProbe for SelectiveProbe {
    fn can_register(&self, trigger: &smartcat_translate::hotkeys::Trigger) -> bool {
        let normalized = trigger.to_string();
        self.observed.lock().unwrap().push(normalized.clone());
        !self.blocked.contains(&normalized)
    }
}

struct RunningApps(Vec<String>);

impl AppInspector for RunningApps {
    fn running_process_names(&self) -> Vec<String> {
        self.0.clone()
    }
}

fn date() -> NaiveDate {
    NaiveDate::parse_from_str(AS_OF, "%Y-%m-%d").unwrap()
}

fn analyzer<'a>(
    platform: Platform,
    catalog: &'a ShortcutCatalog,
    probe: &'a SelectiveProbe,
    apps: &'a RunningApps,
) -> ConflictAnalyzer<'a> {
    ConflictAnalyzer::new(platform, catalog, probe, apps)
}

#[test]
fn distinguishes_confirmed_registration_failure_and_running_app_causes() {
    // Mutation caught: dropping either the registration layer or running-app/catalog layer.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::blocking(&["Ctrl+L"]);
    let apps = RunningApps(vec!["chrome.exe".to_owned()]);

    let report = analyzer(Platform::Windows, &catalog, &probe, &apps)
        .analyze(&parse_trigger("Ctrl+L").unwrap());

    assert_eq!(report.level, ConflictLevel::Confirmed);
    assert!(report.causes.iter().any(|cause| {
        cause.application.as_deref() == Some("Google Chrome")
            && cause.feature.as_deref() == Some("주소 표시줄로 이동")
            && cause.source_url.as_deref()
                == Some("https://support.google.com/chrome/answer/157179")
            && cause.verified_at.as_deref() == Some(AS_OF)
    }));
    assert!(report.causes.iter().any(|cause| {
        cause.application.is_none()
            && cause.description == "다른 프로그램이 사용 중일 수 있습니다."
            && cause.severity == ConflictSeverity::Blocking
    }));
    assert_eq!(report.alternatives.len(), 3);
    assert_eq!(
        report
            .alternatives
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["Ctrl+Shift+L", "Ctrl+Alt+L", "Ctrl+Alt+Shift+L"]
    );
}

#[test]
fn never_claims_an_unknown_owner() {
    // Mutation caught: guessing an application name from a failed OS probe.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::blocking(&["Ctrl+Alt+9"]);
    let apps = RunningApps(Vec::new());

    let report = analyzer(Platform::Windows, &catalog, &probe, &apps)
        .analyze(&parse_trigger("Ctrl+Alt+9").unwrap());

    assert_eq!(report.level, ConflictLevel::Confirmed);
    assert_eq!(
        report.causes[0].description,
        "다른 프로그램이 사용 중일 수 있습니다."
    );
    assert_eq!(report.causes[0].application, None);
}

#[test]
fn possible_catalog_conflicts_block_by_default_and_require_explicit_force() {
    // Mutation caught: silently saving a likely collision or force-enabling a reserved shortcut.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::default();
    let apps = RunningApps(vec!["Code.exe".to_owned()]);

    let report = analyzer(Platform::Windows, &catalog, &probe, &apps)
        .analyze(&parse_trigger("Ctrl+Shift+P").unwrap());

    assert_eq!(report.level, ConflictLevel::Possible);
    assert!(report.can_force);
    assert!(!report.registration_allowed(false));
    assert!(report.registration_allowed(true));
    assert_eq!(
        report.causes[0].application.as_deref(),
        Some("Visual Studio Code")
    );
    assert_eq!(
        report.causes[0].feature.as_deref(),
        Some("명령 팔레트 표시")
    );

    let reserved = analyzer(
        Platform::Windows,
        &catalog,
        &probe,
        &RunningApps(Vec::new()),
    )
    .analyze(&parse_trigger("F12").unwrap());
    assert_eq!(reserved.level, ConflictLevel::Confirmed);
    assert!(!reserved.can_force);
    assert!(!reserved.registration_allowed(true));
}

#[test]
fn classifies_windows_and_macos_reserved_combinations_as_non_forceable() {
    // Mutation caught: treating documented OS shortcuts as merely app-level warnings.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::default();
    let none = RunningApps(Vec::new());

    for (platform, trigger, application) in [
        (Platform::Windows, "Meta+L", "Windows"),
        (Platform::Windows, "Ctrl+F12", "Windows"),
        (Platform::Macos, "Meta+Space", "macOS"),
        (Platform::Macos, "Ctrl+Meta+Q", "macOS"),
    ] {
        let report =
            analyzer(platform, &catalog, &probe, &none).analyze(&parse_trigger(trigger).unwrap());
        assert_eq!(report.level, ConflictLevel::Confirmed, "{trigger}");
        assert!(!report.can_force, "{trigger}");
        assert_eq!(report.causes[0].application.as_deref(), Some(application));
        assert!(report.causes[0]
            .source_url
            .as_deref()
            .unwrap()
            .starts_with("https://"));
    }
}

#[test]
fn matches_only_sanitized_process_names_and_never_exposes_titles_or_paths() {
    // Mutation caught: accepting a full path/window title as process identity and echoing it.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::default();
    let apps = RunningApps(vec![
        "C:\\Program Files\\Google\\Chrome\\chrome.exe".to_owned(),
        "Document title - Google Chrome".to_owned(),
        "chrome.exe\nsecret".to_owned(),
    ]);

    let report = analyzer(Platform::Windows, &catalog, &probe, &apps)
        .analyze(&parse_trigger("Ctrl+L").unwrap());

    assert_eq!(report.level, ConflictLevel::None);
    assert!(report.causes.is_empty());
    assert!(format!("{report:?}").find("Program Files").is_none());
    assert!(format!("{report:?}").find("Document title").is_none());
}

#[test]
fn alternatives_are_reanalyzed_and_skip_every_blocked_or_known_collision() {
    // Mutation caught: recommending candidates without running the same layered checks.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::blocking(&["Ctrl+L", "Ctrl+Shift+L", "Ctrl+Alt+L"]);
    let apps = RunningApps(vec!["chrome.exe".to_owned()]);

    let report = analyzer(Platform::Windows, &catalog, &probe, &apps)
        .analyze(&parse_trigger("Ctrl+L").unwrap());
    let alternatives = report
        .alternatives
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    assert_eq!(alternatives.len(), 3);
    assert!(!alternatives.contains(&"Ctrl+Shift+L".to_owned()));
    assert!(!alternatives.contains(&"Ctrl+Alt+L".to_owned()));
    assert!(probe
        .observed
        .lock()
        .unwrap()
        .iter()
        .any(|value| value == "Ctrl+Shift+L"));
}

#[test]
fn every_seeded_application_collision_has_three_available_alternatives() {
    // Mutation caught: a catalog addition whose trigger cannot be escaped by the recommender.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    for entry in catalog
        .entries()
        .iter()
        .filter(|entry| entry.kind == smartcat_translate::hotkeys::CatalogKind::Application)
    {
        let probe = SelectiveProbe::blocking(&[&entry.trigger]);
        let apps = RunningApps(vec![entry.process_names[0].clone()]);
        let report = analyzer(entry.platform, &catalog, &probe, &apps)
            .analyze(&parse_trigger(&entry.trigger).unwrap());
        assert_eq!(
            report.alternatives.len(),
            3,
            "{} {}",
            entry.application,
            entry.trigger
        );
        for alternative in &report.alternatives {
            assert_eq!(
                analyzer(entry.platform, &catalog, &probe, &apps)
                    .analyze(alternative)
                    .level,
                ConflictLevel::None,
                "{} recommended {}",
                entry.application,
                alternative
            );
        }
    }
}

#[test]
fn embedded_catalog_has_a_bounded_schema_version_and_verified_hash() {
    // Mutation caught: shipping malformed/unversioned/unhashed catalog bytes.
    let bytes = include_bytes!("../resources/shortcut-catalog.json");
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../resources/shortcut-catalog.schema.json")).unwrap();
    let instance: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&instance)
        .unwrap();

    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    assert_eq!(catalog.schema_version(), 1);
    assert_eq!(catalog.catalog_version(), "2026.08.30.1");
    assert_eq!(catalog.sha256(), format!("{:x}", Sha256::digest(bytes)));
    assert!(catalog.entries().len() >= 13);
}

#[test]
fn catalog_validation_rejects_missing_sources_stale_dates_duplicates_and_bad_hashes() {
    // Mutation caught: accepting untraceable, stale, duplicate or tampered catalog content.
    let bytes = include_bytes!("../resources/shortcut-catalog.json");
    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();

    value["entries"][0]["sourceUrl"] = serde_json::Value::String(String::new());
    assert_verified_catalog_error(&value, CatalogError::InvalidSource);

    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    value["entries"][0]["sourceUrl"] =
        serde_json::Value::String("https://example.com/shortcut-list".to_owned());
    assert_verified_catalog_error(&value, CatalogError::InvalidSource);

    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    value["entries"][0]["verifiedAt"] = serde_json::Value::String("2024-12-29".to_owned());
    assert_verified_catalog_error(&value, CatalogError::StaleEntry);

    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let duplicate = value["entries"][0].clone();
    value["entries"].as_array_mut().unwrap().push(duplicate);
    assert_verified_catalog_error(&value, CatalogError::DuplicateEntry);

    assert_eq!(
        ShortcutCatalog::parse_verified(bytes, "00", date()).unwrap_err(),
        CatalogError::HashMismatch
    );
}

#[test]
fn catalog_rejects_oversized_documents_and_entry_collections_before_use() {
    // Mutation caught: removing resource size/entry-count bounds from an untrusted update.
    let oversized = vec![b' '; 256 * 1024 + 1];
    assert_eq!(
        ShortcutCatalog::parse_verified(
            &oversized,
            &format!("{:x}", Sha256::digest(&oversized)),
            date(),
        )
        .unwrap_err(),
        CatalogError::TooLarge
    );

    let sample = serde_json::json!({
        "platform": "windows",
        "kind": "application",
        "application": "x",
        "processNames": ["x"],
        "trigger": "F8",
        "feature": "x",
        "sourceUrl": "https://x.io",
        "verifiedAt": AS_OF
    });
    let value = serde_json::json!({
        "schemaVersion": 1,
        "catalogVersion": "1",
        "entries": vec![sample; 1_025]
    });
    assert_verified_catalog_error(&value, CatalogError::TooManyEntries);
}

fn assert_verified_catalog_error(value: &serde_json::Value, expected: CatalogError) {
    let bytes = serde_json::to_vec(value).unwrap();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        ShortcutCatalog::parse_verified(&bytes, &hash, date()).unwrap_err(),
        expected
    );
}
