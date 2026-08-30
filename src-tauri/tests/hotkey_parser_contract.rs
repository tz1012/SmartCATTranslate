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
fn preserves_reserved_combinations_for_the_conflict_analyzer() {
    for input in [
        "Ctrl+Alt+Delete",
        "Alt+F4",
        "Meta+L",
        "Command+Control+Q",
        "Command+Option+Escape",
    ] {
        let trigger = parse_trigger(input).unwrap();
        let wire = serde_json::to_vec(&trigger).unwrap();
        assert_eq!(serde_json::from_slice::<Trigger>(&wire).unwrap(), trigger);
    }
}

#[test]
fn supports_platform_contract_keys_without_losing_physical_identity() {
    for (input, display) in [
        ("PrintScreen", "PrintScreen"),
        ("Pause", "Pause"),
        ("CapsLock", "CapsLock"),
        ("NumLock", "NumLock"),
        ("ScrollLock", "ScrollLock"),
        ("ContextMenu", "ContextMenu"),
        ("NumpadEnter", "NumpadEnter"),
        ("Ctrl+NumpadEqual", "Ctrl+NumpadEqual"),
        ("Ctrl+IntlBackslash", "Ctrl+IntlBackslash"),
        ("Ctrl+IntlRo", "Ctrl+IntlRo"),
        ("Ctrl+IntlYen", "Ctrl+IntlYen"),
    ] {
        assert_eq!(parse_trigger(input).unwrap().to_string(), display);
    }
}

#[test]
fn plus_requires_the_explicit_shift_equal_chord() {
    assert_eq!(parse_trigger("Ctrl+Plus"), Err(HotkeyError::UnknownKey));
    assert_eq!(
        parse_trigger("Ctrl+Shift+Equal").unwrap().to_string(),
        "Ctrl+Shift+Equal"
    );
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

    let huge = serde_json::json!({
        "type": "sequence",
        "steps": (0..10_000).map(|_| serde_json::json!({
            "modifiers": { "ctrl": true, "alt": false, "shift": false, "meta": false },
            "key": { "kind": "physical", "value": "keyA" }
        })).collect::<Vec<_>>(),
        "timeoutMs": 650
    });
    assert!(serde_json::from_value::<Trigger>(huge).is_err());

    let unknown_top_level = serde_json::json!({
        "type": "chord",
        "chord": {
            "modifiers": { "ctrl": true, "alt": false, "shift": false, "meta": false },
            "key": { "kind": "physical", "value": "keyA" }
        },
        "extra": true
    });
    assert!(serde_json::from_value::<Trigger>(unknown_top_level).is_err());
}
