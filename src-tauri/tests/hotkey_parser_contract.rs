use smartcat_translate::hotkeys::{
    parse_trigger, HotkeyAction, HotkeyBinding, HotkeyError, KeyCode, LogicalKey, Modifiers,
    PhysicalKey, Trigger, DEFAULT_SEQUENCE_TIMEOUT_MS,
};
use uuid::Uuid;

#[test]
fn parses_and_displays_normalized_chords_and_sequences() {
    let chord = parse_trigger("  Control + Shift + c  ").unwrap();
    assert_eq!(chord.to_string(), "Ctrl+Shift+C");

    let sequence = parse_trigger("Ctrl+C, C").unwrap();
    assert_eq!(sequence.to_string(), "Ctrl+C, C");
    assert_eq!(
        sequence,
        Trigger::Sequence {
            steps: vec![
                smartcat_translate::hotkeys::Chord {
                    modifiers: Modifiers {
                        ctrl: true,
                        ..Modifiers::default()
                    },
                    key: KeyCode::Physical(PhysicalKey::KeyC),
                },
                smartcat_translate::hotkeys::Chord {
                    modifiers: Modifiers::default(),
                    key: KeyCode::Physical(PhysicalKey::KeyC),
                },
            ],
            timeout_ms: DEFAULT_SEQUENCE_TIMEOUT_MS,
        }
    );
}

#[test]
fn normalizes_aliases_duplicate_modifiers_and_modifier_order() {
    assert_eq!(
        parse_trigger("shift+ctrl+control+option+c")
            .unwrap()
            .to_string(),
        "Ctrl+Alt+Shift+C"
    );
    assert_eq!(
        parse_trigger("Command+Shift+C").unwrap().to_string(),
        "Shift+Meta+C"
    );
    assert_eq!(
        parse_trigger("Cmd+ArrowUp").unwrap(),
        Trigger::Chord {
            chord: smartcat_translate::hotkeys::Chord {
                modifiers: Modifiers {
                    meta: true,
                    ..Modifiers::default()
                },
                key: KeyCode::Logical(LogicalKey::ArrowUp),
            },
        }
    );
}

#[test]
fn command_or_control_maps_to_the_current_platform() {
    let display = parse_trigger("CommandOrControl+C").unwrap().to_string();
    #[cfg(target_os = "macos")]
    assert_eq!(display, "Meta+C");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(display, "Ctrl+C");
}

#[test]
fn rejects_modifier_only_unknown_multiple_key_and_oversized_triggers() {
    assert_eq!(parse_trigger("Ctrl"), Err(HotkeyError::ModifierOnly));
    assert_eq!(parse_trigger("Ctrl+Banana"), Err(HotkeyError::UnknownKey));
    assert_eq!(parse_trigger("Ctrl+C+V"), Err(HotkeyError::MultipleKeys));
    assert_eq!(
        parse_trigger("Ctrl+A, Ctrl+B, Ctrl+C, Ctrl+D, Ctrl+E"),
        Err(HotkeyError::TooManySteps)
    );
    assert_eq!(parse_trigger("Ctrl+C,"), Err(HotkeyError::EmptyStep));
    assert_eq!(
        parse_trigger(&format!("Ctrl+{}", "A".repeat(1_025))),
        Err(HotkeyError::TooLong)
    );
}

#[test]
fn rejects_plain_typing_as_the_first_step_but_allows_sequence_followups() {
    assert_eq!(parse_trigger("C"), Err(HotkeyError::PlainTyping));
    assert_eq!(parse_trigger("C, Ctrl+C"), Err(HotkeyError::PlainTyping));
    assert!(parse_trigger("Ctrl+C, C").is_ok());
    assert!(parse_trigger("F8").is_ok());
    assert_eq!(parse_trigger("ArrowUp").unwrap().to_string(), "ArrowUp");
}

#[test]
fn supports_common_punctuation_and_numpad_physical_keys() {
    assert_eq!(
        parse_trigger("Ctrl+Comma").unwrap().to_string(),
        "Ctrl+Comma"
    );
    assert_eq!(
        parse_trigger("Alt+Numpad1").unwrap().to_string(),
        "Alt+Numpad1"
    );
    assert_eq!(parse_trigger("Numpad1"), Err(HotkeyError::PlainTyping));
}

#[test]
fn rejects_dangerous_system_combinations() {
    #[cfg(target_os = "windows")]
    {
        assert_eq!(
            parse_trigger("Ctrl+Alt+Delete"),
            Err(HotkeyError::ForbiddenSystemShortcut)
        );
        assert_eq!(
            parse_trigger("Alt+F4"),
            Err(HotkeyError::ForbiddenSystemShortcut)
        );
        assert_eq!(
            parse_trigger("Meta+L"),
            Err(HotkeyError::ForbiddenSystemShortcut)
        );
    }
    #[cfg(target_os = "macos")]
    {
        assert_eq!(
            parse_trigger("Command+Control+Q"),
            Err(HotkeyError::ForbiddenSystemShortcut)
        );
        assert_eq!(
            parse_trigger("Command+Option+Escape"),
            Err(HotkeyError::ForbiddenSystemShortcut)
        );
        assert!(parse_trigger("Command+L").is_ok());
    }
}

#[test]
fn trigger_and_binding_serde_round_trip_with_the_typescript_shape() {
    let trigger = parse_trigger("Ctrl+C, C").unwrap();
    let encoded = serde_json::to_value(&trigger).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({
            "type": "sequence",
            "steps": [
                {
                    "modifiers": { "ctrl": true, "alt": false, "shift": false, "meta": false },
                    "key": { "kind": "physical", "value": "keyC" }
                },
                {
                    "modifiers": { "ctrl": false, "alt": false, "shift": false, "meta": false },
                    "key": { "kind": "physical", "value": "keyC" }
                }
            ],
            "timeoutMs": 650
        })
    );
    assert_eq!(serde_json::from_value::<Trigger>(encoded).unwrap(), trigger);

    let binding = HotkeyBinding {
        id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
        trigger,
        action: HotkeyAction::TranslateSelection,
        profile_id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
        force: false,
    };
    let serialized = serde_json::to_string(&binding).unwrap();
    assert!(serialized.contains("\"action\":\"translateSelection\""));
    assert_eq!(
        serde_json::from_str::<HotkeyBinding>(&serialized).unwrap(),
        binding
    );
}

#[test]
fn deserialization_rejects_bypassing_trigger_validation() {
    let plain = serde_json::json!({
        "type": "chord",
        "chord": {
            "modifiers": { "ctrl": false, "alt": false, "shift": false, "meta": false },
            "key": { "kind": "physical", "value": "keyA" }
        }
    });
    assert!(serde_json::from_value::<Trigger>(plain).is_err());

    let too_many = serde_json::json!({
        "type": "sequence",
        "steps": (0..5).map(|_| serde_json::json!({
            "modifiers": { "ctrl": true, "alt": false, "shift": false, "meta": false },
            "key": { "kind": "physical", "value": "keyA" }
        })).collect::<Vec<_>>(),
        "timeoutMs": 650
    });
    assert!(serde_json::from_value::<Trigger>(too_many).is_err());
}
