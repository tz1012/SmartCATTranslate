use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use chrono::NaiveDate;
use sha2::{Digest, Sha256};
use smartcat_translate::hotkeys::{
    parse_trigger, AppIdentity, AppInspector, CatalogError, ConflictAnalyzer, ConflictLevel,
    ConflictSeverity, KeyCode, PhysicalKey, Platform, RegistrationProbe, RegistrationProbeStatus,
    ShortcutCatalog, Trigger,
};

const AS_OF: &str = "2026-08-30";

#[derive(Default)]
struct SelectiveProbe {
    statuses: BTreeMap<String, RegistrationProbeStatus>,
    observed: Mutex<Vec<String>>,
}

impl SelectiveProbe {
    fn blocking(triggers: &[&str]) -> Self {
        Self {
            statuses: triggers
                .iter()
                .map(|value| {
                    (
                        (*value).to_owned(),
                        RegistrationProbeStatus::Occupied {
                            observer_available: true,
                        },
                    )
                })
                .collect(),
            observed: Mutex::new(Vec::new()),
        }
    }

    fn with_status(trigger: &str, status: RegistrationProbeStatus) -> Self {
        Self {
            statuses: BTreeMap::from([(trigger.to_owned(), status)]),
            observed: Mutex::new(Vec::new()),
        }
    }
}

impl RegistrationProbe for SelectiveProbe {
    fn probe_and_restore(
        &self,
        trigger: &smartcat_translate::hotkeys::Trigger,
    ) -> RegistrationProbeStatus {
        let normalized = trigger.to_string();
        self.observed.lock().unwrap().push(normalized.clone());
        self.statuses
            .get(&normalized)
            .copied()
            .unwrap_or(RegistrationProbeStatus::Available)
    }
}

#[derive(Default)]
struct RestoringTrialProbe {
    temporarily_registered: Mutex<BTreeSet<String>>,
}

impl RegistrationProbe for RestoringTrialProbe {
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus {
        let normalized = trigger.to_string();
        assert!(self
            .temporarily_registered
            .lock()
            .unwrap()
            .insert(normalized.clone()));
        assert!(self
            .temporarily_registered
            .lock()
            .unwrap()
            .remove(&normalized));
        RegistrationProbeStatus::Available
    }
}

struct RunningApps(Vec<AppIdentity>);

impl AppInspector for RunningApps {
    fn running_apps(&self) -> Vec<AppIdentity> {
        self.0.clone()
    }
}

fn executable(name: &str) -> AppIdentity {
    AppIdentity::new(Some(name), None).unwrap()
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
    let apps = RunningApps(vec![executable("chrome.exe")]);

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
    let apps = RunningApps(vec![executable("Code.exe")]);

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
fn app_identity_accepts_only_sanitized_basename_or_bundle_id() {
    // Mutation caught: accepting a full path, title, control character or oversized identity.
    assert!(AppIdentity::new(Some("Code.exe"), Some("com.microsoft.VSCode")).is_some());
    assert!(AppIdentity::new(Some("C:\\Program Files\\Code.exe"), None).is_none());
    assert!(AppIdentity::new(Some("Code.exe\nsecret"), None).is_none());
    assert!(AppIdentity::new(Some(&"x".repeat(129)), None).is_none());
    assert!(AppIdentity::new(None, Some("com.microsoft.VSCode/secret")).is_none());
    assert!(AppIdentity::new(None, None).is_none());
}

#[test]
fn matches_vscode_identity_without_falling_back_after_a_bundle_mismatch() {
    // Mutation caught: OR-matching a generic Electron basename after a bundle mismatch.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::default();
    for (platform, identity, trigger) in [
        (Platform::Windows, executable("Code.exe"), "Ctrl+Shift+P"),
        (Platform::Windows, executable("Code"), "Ctrl+Shift+P"),
        (
            Platform::Macos,
            AppIdentity::new(Some("Electron"), Some("com.microsoft.VSCode")).unwrap(),
            "Shift+Meta+P",
        ),
    ] {
        let report = analyzer(platform, &catalog, &probe, &RunningApps(vec![identity]))
            .analyze(&parse_trigger(trigger).unwrap());
        assert_eq!(report.level, ConflictLevel::Possible, "{platform:?}");
        assert_eq!(
            report.causes[0].application.as_deref(),
            Some("Visual Studio Code")
        );
    }

    let slack = RunningApps(vec![AppIdentity::new(
        Some("Electron"),
        Some("com.tinyspeck.slackmacgap"),
    )
    .unwrap()]);
    let report = analyzer(Platform::Macos, &catalog, &probe, &slack)
        .analyze(&parse_trigger("Shift+Meta+P").unwrap());
    assert_eq!(report.level, ConflictLevel::None);
    assert!(report.causes.is_empty());
}

#[test]
fn matches_macos_deepl_bundle_for_translation_and_improvement_shortcuts() {
    // Mutation caught: leaving the authoritative bundle ID empty or omitting the Mac improve row.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let deepl = RunningApps(vec![AppIdentity::new(
        Some("DeepL"),
        Some("com.linguee.DeepLCopyTranslator"),
    )
    .unwrap()]);

    for (trigger, status, feature) in [
        (
            "Meta+C, C",
            RegistrationProbeStatus::AvailableViaObserver,
            "선택한 텍스트 번역",
        ),
        (
            "Ctrl+Meta+C",
            RegistrationProbeStatus::Available,
            "선택한 텍스트 개선",
        ),
    ] {
        let probe = SelectiveProbe::with_status(trigger, status);
        let report = analyzer(Platform::Macos, &catalog, &probe, &deepl)
            .analyze(&parse_trigger(trigger).unwrap());
        assert_eq!(report.level, ConflictLevel::Possible, "{trigger}");
        assert!(!report.registration_allowed(false), "{trigger}");
        assert!(report.causes.iter().any(|cause| {
            cause.application.as_deref() == Some("DeepL")
                && cause.feature.as_deref() == Some(feature)
                && cause.source_url.as_deref()
                    == Some(
                        "https://support.deepl.com/hc/en-us/articles/360020613059-Use-DeepL-shortcuts-for-desktop-apps",
                    )
                && cause.verified_at.as_deref() == Some(AS_OF)
        }));
    }
}

#[test]
fn sequence_catalog_blocks_only_full_containment_and_advises_on_a_strict_prefix() {
    // Mutation caught: symmetric overlap that blocks one prefix chord or an unrelated lead-in.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe =
        SelectiveProbe::with_status("Ctrl+L, C", RegistrationProbeStatus::AvailableViaObserver);
    let chrome = RunningApps(vec![executable("chrome.exe")]);
    let chrome_report = analyzer(Platform::Windows, &catalog, &probe, &chrome)
        .analyze(&parse_trigger("Ctrl+L, C").unwrap());
    assert_eq!(chrome_report.level, ConflictLevel::Possible);
    assert!(!chrome_report.registration_allowed(false));
    assert!(chrome_report.causes.iter().any(|cause| {
        cause.application.as_deref() == Some("Google Chrome")
            && cause.feature.as_deref() == Some("주소 표시줄로 이동")
    }));

    let deep_l = RunningApps(vec![executable("DeepL.exe")]);
    for trigger in [
        "Ctrl+C, C",
        "Ctrl+C, C, Ctrl+X",
        "Ctrl+X, Ctrl+C, C, Ctrl+V",
    ] {
        let probe =
            SelectiveProbe::with_status(trigger, RegistrationProbeStatus::AvailableViaObserver);
        let report = analyzer(Platform::Windows, &catalog, &probe, &deep_l)
            .analyze(&parse_trigger(trigger).unwrap());
        assert_eq!(report.level, ConflictLevel::Possible, "{trigger}");
        assert!(!report.registration_allowed(false), "{trigger}");
    }

    let strict_prefix = analyzer(
        Platform::Windows,
        &catalog,
        &SelectiveProbe::default(),
        &deep_l,
    )
    .analyze(&parse_trigger("Ctrl+C").unwrap());
    assert_eq!(strict_prefix.level, ConflictLevel::None);
    assert!(strict_prefix.registration_allowed(false));
    assert!(strict_prefix.causes.iter().any(|cause| {
        cause.application.as_deref() == Some("DeepL")
            && cause.severity == ConflictSeverity::Advisory
    }));

    let unrelated_lead_in_probe = SelectiveProbe::with_status(
        "Ctrl+X, Ctrl+C",
        RegistrationProbeStatus::AvailableViaObserver,
    );
    let unrelated_lead_in = analyzer(
        Platform::Windows,
        &catalog,
        &unrelated_lead_in_probe,
        &deep_l,
    )
    .analyze(&parse_trigger("Ctrl+X, Ctrl+C").unwrap());
    assert_eq!(unrelated_lead_in.level, ConflictLevel::None);
    assert!(unrelated_lead_in.registration_allowed(false));
    assert!(unrelated_lead_in.causes.is_empty());
}

#[test]
fn every_reserved_step_is_non_forceable_and_windows_meta_is_a_broad_rule() {
    // Mutation caught: checking only exact catalog rows such as Meta+L.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::default();
    let none = RunningApps(Vec::new());
    for (platform, trigger) in [
        (Platform::Windows, "Ctrl+Meta+C"),
        (Platform::Windows, "Meta+L, C"),
        (Platform::Macos, "Meta+Space, C"),
        (Platform::Macos, "Ctrl+Meta+Q, C"),
    ] {
        let report =
            analyzer(platform, &catalog, &probe, &none).analyze(&parse_trigger(trigger).unwrap());
        assert_eq!(report.level, ConflictLevel::Confirmed, "{trigger}");
        assert!(!report.can_force, "{trigger}");
        assert!(!report.registration_allowed(true), "{trigger}");
    }
}

#[test]
fn force_depends_on_structured_probe_status_and_observer_availability() {
    // Mutation caught: one bool treating every registration failure as forceable.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let none = RunningApps(Vec::new());
    let cases = [
        (
            RegistrationProbeStatus::Occupied {
                observer_available: true,
            },
            true,
        ),
        (
            RegistrationProbeStatus::Occupied {
                observer_available: false,
            },
            false,
        ),
        (
            RegistrationProbeStatus::UnsupportedSequence {
                observer_available: true,
            },
            true,
        ),
        (
            RegistrationProbeStatus::UnsupportedSequence {
                observer_available: false,
            },
            false,
        ),
        (RegistrationProbeStatus::PermissionDenied, false),
        (RegistrationProbeStatus::Invalid, false),
        (RegistrationProbeStatus::OsReserved, false),
        (RegistrationProbeStatus::BackendError, false),
    ];
    for (status, can_force) in cases {
        let probe = SelectiveProbe::with_status("Ctrl+Alt+9", status);
        let report = analyzer(Platform::Windows, &catalog, &probe, &none)
            .analyze(&parse_trigger("Ctrl+Alt+9").unwrap());
        assert_eq!(report.level, ConflictLevel::Confirmed, "{status:?}");
        assert_eq!(report.can_force, can_force, "{status:?}");
        assert_eq!(report.registration_allowed(true), can_force, "{status:?}");
        assert!(!format!("{report:?}").contains("BackendError"));
    }

    let available = analyzer(
        Platform::Windows,
        &catalog,
        &SelectiveProbe::default(),
        &none,
    )
    .analyze(&parse_trigger("Ctrl+Alt+9").unwrap());
    assert_eq!(available.level, ConflictLevel::None);

    let observer_available = analyzer(
        Platform::Windows,
        &catalog,
        &SelectiveProbe::with_status(
            "Ctrl+Alt+C, C",
            RegistrationProbeStatus::AvailableViaObserver,
        ),
        &none,
    )
    .analyze(&parse_trigger("Ctrl+Alt+C, C").unwrap());
    assert_eq!(observer_available.level, ConflictLevel::None);
    assert!(observer_available.registration_allowed(false));
    assert!(!observer_available.can_force);
    assert!(observer_available.causes.is_empty());
}

#[test]
fn a_successful_direct_registration_trial_is_restored_before_probe_return() {
    // Mutation caught: a Task 4 adapter returning Available while retaining registration.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = RestoringTrialProbe::default();
    let none = RunningApps(Vec::new());
    let report = ConflictAnalyzer::new(Platform::Windows, &catalog, &probe, &none)
        .analyze(&parse_trigger("Ctrl+Alt+9").unwrap());
    assert_eq!(report.level, ConflictLevel::None);
    assert!(probe.temporarily_registered.lock().unwrap().is_empty());
}

#[test]
fn alternatives_use_only_common_physical_keys_and_avoid_known_running_app_shortcuts() {
    // Mutation caught: falling back to F13-F24 or Word's Ctrl+Shift+L.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();

    let vscode_probe = SelectiveProbe::blocking(&["Ctrl+Shift+P"]);
    let vscode = RunningApps(vec![executable("Code.exe")]);
    let vscode_report = analyzer(Platform::Windows, &catalog, &vscode_probe, &vscode)
        .analyze(&parse_trigger("Ctrl+Shift+P").unwrap());
    let vscode_alternatives = vscode_report
        .alternatives
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(!vscode_alternatives.contains(&"F8".to_owned()));
    assert!(!vscode_alternatives.contains(&"F9".to_owned()));
    assert!(vscode_report
        .alternatives
        .iter()
        .all(is_common_physical_trigger));
    assert!(vscode_alternatives
        .iter()
        .all(|value| !value.contains("F13") && !value.contains("F24")));

    let word_probe = SelectiveProbe::blocking(&["Ctrl+L"]);
    let word = RunningApps(vec![executable("WINWORD.EXE")]);
    let word_report = analyzer(Platform::Windows, &catalog, &word_probe, &word)
        .analyze(&parse_trigger("Ctrl+L").unwrap());
    assert!(!word_report
        .alternatives
        .iter()
        .any(|trigger| trigger.to_string() == "Ctrl+Shift+L"));
}

fn is_common_physical_trigger(trigger: &Trigger) -> bool {
    let is_common = |key: PhysicalKey| {
        matches!(
            key,
            PhysicalKey::KeyA
                | PhysicalKey::KeyB
                | PhysicalKey::KeyC
                | PhysicalKey::KeyD
                | PhysicalKey::KeyE
                | PhysicalKey::KeyF
                | PhysicalKey::KeyG
                | PhysicalKey::KeyH
                | PhysicalKey::KeyI
                | PhysicalKey::KeyJ
                | PhysicalKey::KeyK
                | PhysicalKey::KeyL
                | PhysicalKey::KeyM
                | PhysicalKey::KeyN
                | PhysicalKey::KeyO
                | PhysicalKey::KeyP
                | PhysicalKey::KeyQ
                | PhysicalKey::KeyR
                | PhysicalKey::KeyS
                | PhysicalKey::KeyT
                | PhysicalKey::KeyU
                | PhysicalKey::KeyV
                | PhysicalKey::KeyW
                | PhysicalKey::KeyX
                | PhysicalKey::KeyY
                | PhysicalKey::KeyZ
                | PhysicalKey::Digit0
                | PhysicalKey::Digit1
                | PhysicalKey::Digit2
                | PhysicalKey::Digit3
                | PhysicalKey::Digit4
                | PhysicalKey::Digit5
                | PhysicalKey::Digit6
                | PhysicalKey::Digit7
                | PhysicalKey::Digit8
                | PhysicalKey::Digit9
        )
    };
    match trigger {
        Trigger::Chord { chord } => matches!(chord.key, KeyCode::Physical(key) if is_common(key)),
        Trigger::Sequence { steps, .. } => steps
            .iter()
            .all(|chord| matches!(chord.key, KeyCode::Physical(key) if is_common(key))),
    }
}

#[test]
fn alternatives_are_reanalyzed_and_skip_every_blocked_or_known_collision() {
    // Mutation caught: recommending candidates without running the same layered checks.
    let catalog = ShortcutCatalog::from_embedded(date()).unwrap();
    let probe = SelectiveProbe::blocking(&["Ctrl+L", "Ctrl+Shift+L", "Ctrl+Alt+L"]);
    let apps = RunningApps(vec![executable("chrome.exe")]);

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
        let apps = RunningApps(vec![executable(&entry.process_names[0])]);
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
    assert_eq!(catalog.catalog_version(), "2026.08.30.4");
    assert_eq!(catalog.sha256(), format!("{:x}", Sha256::digest(bytes)));
    assert!(catalog.entries().len() >= 13);
    let powerpoint = catalog
        .entries()
        .iter()
        .find(|entry| entry.application == "Microsoft PowerPoint")
        .unwrap();
    assert_eq!(
        powerpoint.source_url,
        "https://support.microsoft.com/en-us/accessibility/powerpoint/use-keyboard-shortcuts-to-create-powerpoint-presentations"
    );
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
    value["entries"][4]["trigger"] = serde_json::Value::String(" ctrl+l ".to_owned());
    assert_verified_catalog_error(&value, CatalogError::InvalidTrigger);

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
        "bundleIds": [],
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
