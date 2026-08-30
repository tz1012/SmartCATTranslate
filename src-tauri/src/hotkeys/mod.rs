mod parser;
mod sequence;
mod types;

pub use parser::parse_trigger;
pub use sequence::{
    KeyDevice, KeyEvent, KeyEventPhase, KeyPropagation, SequenceEngine, SequenceEngineError,
    SequenceOutcome, MAX_ACTIVE_DEVICES, MAX_SEQUENCE_BINDINGS,
};
pub use types::{
    Chord, HotkeyAction, HotkeyBinding, HotkeyError, KeyCode, LogicalKey, Modifiers, PhysicalKey,
    Trigger, DEFAULT_SEQUENCE_TIMEOUT_MS, MAX_SEQUENCE_STEPS,
};
