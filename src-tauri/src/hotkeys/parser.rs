use super::types::{
    Chord, HotkeyError, KeyCode, Modifiers, Trigger, DEFAULT_SEQUENCE_TIMEOUT_MS,
    MAX_SEQUENCE_STEPS,
};

const MAX_TRIGGER_CHARS: usize = 256;
const MAX_TRIGGER_BYTES: usize = 1_024;

pub fn parse_trigger(input: &str) -> Result<Trigger, HotkeyError> {
    if input.chars().count() > MAX_TRIGGER_CHARS || input.len() > MAX_TRIGGER_BYTES {
        return Err(HotkeyError::TooLong);
    }
    if input.trim().is_empty() {
        return Err(HotkeyError::Empty);
    }

    let raw_steps: Vec<_> = input.split(',').collect();
    if raw_steps.len() > MAX_SEQUENCE_STEPS {
        return Err(HotkeyError::TooManySteps);
    }
    let steps = raw_steps
        .into_iter()
        .map(parse_chord)
        .collect::<Result<Vec<_>, _>>()?;
    if steps.len() == 1 {
        Trigger::chord(steps[0])
    } else {
        Trigger::sequence(steps, DEFAULT_SEQUENCE_TIMEOUT_MS)
    }
}

fn parse_chord(input: &str) -> Result<Chord, HotkeyError> {
    if input.trim().is_empty() {
        return Err(HotkeyError::EmptyStep);
    }

    let mut modifiers = Modifiers::default();
    let mut key = None;
    for raw_token in input.split('+') {
        let token = raw_token.trim();
        if token.is_empty() {
            return Err(HotkeyError::UnknownKey);
        }
        let normalized = token.to_ascii_uppercase();
        if apply_modifier(&normalized, &mut modifiers) {
            continue;
        }
        let parsed = KeyCode::parse(&normalized).ok_or(HotkeyError::UnknownKey)?;
        if key.replace(parsed).is_some() {
            return Err(HotkeyError::MultipleKeys);
        }
    }
    let key = key.ok_or(HotkeyError::ModifierOnly)?;
    Ok(Chord { modifiers, key })
}

fn apply_modifier(value: &str, modifiers: &mut Modifiers) -> bool {
    match value {
        "CTRL" | "CONTROL" => modifiers.ctrl = true,
        "ALT" | "OPTION" => modifiers.alt = true,
        "SHIFT" => modifiers.shift = true,
        "META" | "CMD" | "COMMAND" | "WIN" | "WINDOWS" | "SUPER" => modifiers.meta = true,
        "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" => {
            #[cfg(target_os = "macos")]
            {
                modifiers.meta = true;
            }
            #[cfg(not(target_os = "macos"))]
            {
                modifiers.ctrl = true;
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::parse_trigger;
    use crate::hotkeys::{HotkeyError, MAX_SEQUENCE_STEPS};

    #[test]
    fn accepts_the_maximum_sequence_and_rejects_one_more_step() {
        assert_eq!(MAX_SEQUENCE_STEPS, 4);
        assert!(parse_trigger("Ctrl+A, B, C, D").is_ok());
        assert_eq!(
            parse_trigger("Ctrl+A, B, C, D, E"),
            Err(HotkeyError::TooManySteps)
        );
    }

    #[test]
    fn accepts_named_keys_and_rejects_out_of_range_function_keys() {
        assert_eq!(
            parse_trigger("Ctrl+Return").unwrap().to_string(),
            "Ctrl+Enter"
        );
        assert_eq!(parse_trigger("Ctrl+F25"), Err(HotkeyError::UnknownKey));
    }
}
