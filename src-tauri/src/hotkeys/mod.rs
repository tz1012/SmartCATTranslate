mod parser;
mod types;

pub use parser::parse_trigger;
pub use types::{
    Chord, HotkeyAction, HotkeyBinding, HotkeyError, KeyCode, LogicalKey, Modifiers, PhysicalKey,
    Trigger, DEFAULT_SEQUENCE_TIMEOUT_MS, MAX_SEQUENCE_STEPS,
};
