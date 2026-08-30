use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

pub const DEFAULT_SEQUENCE_TIMEOUT_MS: u64 = 650;
pub const MAX_SEQUENCE_STEPS: usize = 4;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn is_empty(self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.meta
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum PhysicalKey {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Backquote,
    Backslash,
    BracketLeft,
    BracketRight,
    Comma,
    Equal,
    Minus,
    Period,
    Quote,
    Semicolon,
    Slash,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadDecimal,
    NumpadDivide,
    NumpadMultiply,
    NumpadSubtract,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
}

impl PhysicalKey {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "A" => Self::KeyA,
            "B" => Self::KeyB,
            "C" => Self::KeyC,
            "D" => Self::KeyD,
            "E" => Self::KeyE,
            "F" => Self::KeyF,
            "G" => Self::KeyG,
            "H" => Self::KeyH,
            "I" => Self::KeyI,
            "J" => Self::KeyJ,
            "K" => Self::KeyK,
            "L" => Self::KeyL,
            "M" => Self::KeyM,
            "N" => Self::KeyN,
            "O" => Self::KeyO,
            "P" => Self::KeyP,
            "Q" => Self::KeyQ,
            "R" => Self::KeyR,
            "S" => Self::KeyS,
            "T" => Self::KeyT,
            "U" => Self::KeyU,
            "V" => Self::KeyV,
            "W" => Self::KeyW,
            "X" => Self::KeyX,
            "Y" => Self::KeyY,
            "Z" => Self::KeyZ,
            "0" => Self::Digit0,
            "1" => Self::Digit1,
            "2" => Self::Digit2,
            "3" => Self::Digit3,
            "4" => Self::Digit4,
            "5" => Self::Digit5,
            "6" => Self::Digit6,
            "7" => Self::Digit7,
            "8" => Self::Digit8,
            "9" => Self::Digit9,
            "BACKQUOTE" | "GRAVE" => Self::Backquote,
            "BACKSLASH" => Self::Backslash,
            "BRACKETLEFT" | "LEFTBRACKET" => Self::BracketLeft,
            "BRACKETRIGHT" | "RIGHTBRACKET" => Self::BracketRight,
            "COMMA" => Self::Comma,
            "EQUAL" | "EQUALS" | "PLUS" => Self::Equal,
            "MINUS" | "HYPHEN" => Self::Minus,
            "PERIOD" | "DOT" => Self::Period,
            "QUOTE" | "APOSTROPHE" => Self::Quote,
            "SEMICOLON" => Self::Semicolon,
            "SLASH" => Self::Slash,
            "NUMPAD0" => Self::Numpad0,
            "NUMPAD1" => Self::Numpad1,
            "NUMPAD2" => Self::Numpad2,
            "NUMPAD3" => Self::Numpad3,
            "NUMPAD4" => Self::Numpad4,
            "NUMPAD5" => Self::Numpad5,
            "NUMPAD6" => Self::Numpad6,
            "NUMPAD7" => Self::Numpad7,
            "NUMPAD8" => Self::Numpad8,
            "NUMPAD9" => Self::Numpad9,
            "NUMPADADD" => Self::NumpadAdd,
            "NUMPADDECIMAL" => Self::NumpadDecimal,
            "NUMPADDIVIDE" => Self::NumpadDivide,
            "NUMPADMULTIPLY" => Self::NumpadMultiply,
            "NUMPADSUBTRACT" => Self::NumpadSubtract,
            "F1" => Self::F1,
            "F2" => Self::F2,
            "F3" => Self::F3,
            "F4" => Self::F4,
            "F5" => Self::F5,
            "F6" => Self::F6,
            "F7" => Self::F7,
            "F8" => Self::F8,
            "F9" => Self::F9,
            "F10" => Self::F10,
            "F11" => Self::F11,
            "F12" => Self::F12,
            "F13" => Self::F13,
            "F14" => Self::F14,
            "F15" => Self::F15,
            "F16" => Self::F16,
            "F17" => Self::F17,
            "F18" => Self::F18,
            "F19" => Self::F19,
            "F20" => Self::F20,
            "F21" => Self::F21,
            "F22" => Self::F22,
            "F23" => Self::F23,
            "F24" => Self::F24,
            _ => return None,
        })
    }

    pub fn is_function(self) -> bool {
        matches!(
            self,
            Self::F1
                | Self::F2
                | Self::F3
                | Self::F4
                | Self::F5
                | Self::F6
                | Self::F7
                | Self::F8
                | Self::F9
                | Self::F10
                | Self::F11
                | Self::F12
                | Self::F13
                | Self::F14
                | Self::F15
                | Self::F16
                | Self::F17
                | Self::F18
                | Self::F19
                | Self::F20
                | Self::F21
                | Self::F22
                | Self::F23
                | Self::F24
        )
    }
}

impl fmt::Display for PhysicalKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let serialized = serde_json::to_string(self).map_err(|_| fmt::Error)?;
        let value = serialized.trim_matches('"');
        if let Some(letter) = value.strip_prefix("key") {
            formatter.write_str(letter)
        } else if let Some(digit) = value.strip_prefix("digit") {
            formatter.write_str(digit)
        } else if value.starts_with('f') && value[1..].bytes().all(|byte| byte.is_ascii_digit()) {
            formatter.write_str(&value.to_ascii_uppercase())
        } else {
            let mut characters = value.chars();
            let first = characters.next().ok_or(fmt::Error)?.to_ascii_uppercase();
            write!(formatter, "{first}{}", characters.as_str())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum LogicalKey {
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Backspace,
    Delete,
    End,
    Enter,
    Escape,
    Home,
    Insert,
    PageDown,
    PageUp,
    Space,
    Tab,
}

impl LogicalKey {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "ARROWUP" | "UP" => Self::ArrowUp,
            "ARROWDOWN" | "DOWN" => Self::ArrowDown,
            "ARROWLEFT" | "LEFT" => Self::ArrowLeft,
            "ARROWRIGHT" | "RIGHT" => Self::ArrowRight,
            "BACKSPACE" => Self::Backspace,
            "DELETE" | "DEL" => Self::Delete,
            "END" => Self::End,
            "ENTER" | "RETURN" => Self::Enter,
            "ESCAPE" | "ESC" => Self::Escape,
            "HOME" => Self::Home,
            "INSERT" | "INS" => Self::Insert,
            "PAGEDOWN" | "PGDN" => Self::PageDown,
            "PAGEUP" | "PGUP" => Self::PageUp,
            "SPACE" | "SPACEBAR" => Self::Space,
            "TAB" => Self::Tab,
            _ => return None,
        })
    }
}

impl fmt::Display for LogicalKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::ArrowUp => "ArrowUp",
            Self::ArrowDown => "ArrowDown",
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
            Self::Backspace => "Backspace",
            Self::Delete => "Delete",
            Self::End => "End",
            Self::Enter => "Enter",
            Self::Escape => "Escape",
            Self::Home => "Home",
            Self::Insert => "Insert",
            Self::PageDown => "PageDown",
            Self::PageUp => "PageUp",
            Self::Space => "Space",
            Self::Tab => "Tab",
        };
        formatter.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum KeyCode {
    Physical(PhysicalKey),
    Logical(LogicalKey),
}

impl KeyCode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        PhysicalKey::parse(value)
            .map(Self::Physical)
            .or_else(|| LogicalKey::parse(value).map(Self::Logical))
    }

    fn is_plain_typing(self) -> bool {
        match self {
            Self::Physical(key) => !key.is_function(),
            Self::Logical(key) => key == LogicalKey::Space,
        }
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Physical(key) => key.fmt(formatter),
            Self::Logical(key) => key.fmt(formatter),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: KeyCode,
}

impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::with_capacity(5);
        if self.modifiers.ctrl {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.alt {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.shift {
            parts.push("Shift".to_owned());
        }
        if self.modifiers.meta {
            parts.push("Meta".to_owned());
        }
        parts.push(self.key.to_string());
        formatter.write_str(&parts.join("+"))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    Chord {
        chord: Chord,
    },
    Sequence {
        steps: Vec<Chord>,
        #[serde(rename = "timeoutMs")]
        timeout_ms: u64,
    },
}

impl Trigger {
    pub(crate) fn chord(chord: Chord) -> Result<Self, HotkeyError> {
        validate_steps(std::slice::from_ref(&chord))?;
        Ok(Self::Chord { chord })
    }

    pub(crate) fn sequence(steps: Vec<Chord>, timeout_ms: u64) -> Result<Self, HotkeyError> {
        if !(2..=MAX_SEQUENCE_STEPS).contains(&steps.len()) {
            return Err(if steps.len() > MAX_SEQUENCE_STEPS {
                HotkeyError::TooManySteps
            } else {
                HotkeyError::TooFewSteps
            });
        }
        if timeout_ms != DEFAULT_SEQUENCE_TIMEOUT_MS {
            return Err(HotkeyError::InvalidTimeout);
        }
        validate_steps(&steps)?;
        Ok(Self::Sequence { steps, timeout_ms })
    }
}

fn validate_steps(steps: &[Chord]) -> Result<(), HotkeyError> {
    let first = steps.first().ok_or(HotkeyError::Empty)?;
    if first.modifiers.is_empty() && first.key.is_plain_typing() {
        return Err(HotkeyError::PlainTyping);
    }
    if steps.iter().any(is_forbidden_system_shortcut) {
        return Err(HotkeyError::ForbiddenSystemShortcut);
    }
    Ok(())
}

fn is_forbidden_system_shortcut(chord: &Chord) -> bool {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let only = |ctrl, alt, shift, meta| {
        chord.modifiers
            == Modifiers {
                ctrl,
                alt,
                shift,
                meta,
            }
    };
    #[cfg(target_os = "windows")]
    {
        (only(true, true, false, false) && chord.key == KeyCode::Logical(LogicalKey::Delete))
            || (only(false, true, false, false) && chord.key == KeyCode::Physical(PhysicalKey::F4))
            || (only(false, false, false, true)
                && chord.key == KeyCode::Physical(PhysicalKey::KeyL))
    }
    #[cfg(target_os = "macos")]
    {
        (only(true, false, false, true) && chord.key == KeyCode::Physical(PhysicalKey::KeyQ))
            || (only(false, true, false, true) && chord.key == KeyCode::Logical(LogicalKey::Escape))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

impl<'de> Deserialize<'de> for Trigger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
        enum RawTrigger {
            Chord {
                chord: Chord,
            },
            Sequence {
                steps: Vec<Chord>,
                #[serde(rename = "timeoutMs")]
                timeout_ms: u64,
            },
        }

        match RawTrigger::deserialize(deserializer)? {
            RawTrigger::Chord { chord } => Self::chord(chord),
            RawTrigger::Sequence { steps, timeout_ms } => Self::sequence(steps, timeout_ms),
        }
        .map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Trigger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chord { chord } => chord.fmt(formatter),
            Self::Sequence { steps, .. } => formatter.write_str(
                &steps
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyAction {
    TranslateSelection,
    CaptureScreen,
    TranslateImage,
    OpenMainWindow,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HotkeyBinding {
    pub id: Uuid,
    pub trigger: Trigger,
    pub action: HotkeyAction,
    pub profile_id: Uuid,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    #[error("hotkey is empty")]
    Empty,
    #[error("hotkey exceeds the input limit")]
    TooLong,
    #[error("hotkey contains an empty sequence step")]
    EmptyStep,
    #[error("hotkey has too few sequence steps")]
    TooFewSteps,
    #[error("hotkey has too many sequence steps")]
    TooManySteps,
    #[error("hotkey contains only modifiers")]
    ModifierOnly,
    #[error("hotkey contains more than one key")]
    MultipleKeys,
    #[error("hotkey key is not supported")]
    UnknownKey,
    #[error("plain typing cannot start a global hotkey")]
    PlainTyping,
    #[error("system shortcut cannot be registered")]
    ForbiddenSystemShortcut,
    #[error("sequence timeout is invalid")]
    InvalidTimeout,
}
