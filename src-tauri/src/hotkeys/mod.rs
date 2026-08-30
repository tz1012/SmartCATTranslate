mod conflicts;
mod parser;
mod sequence;
mod types;

pub use conflicts::{
    AppIdentity, AppInspector, CatalogEntry, CatalogError, CatalogKind, ConflictAnalyzer,
    ConflictCause, ConflictLevel, ConflictReport, ConflictSeverity, Platform, RegistrationProbe,
    RegistrationProbeStatus, ShortcutCatalog, MAX_CATALOG_BYTES, MAX_CATALOG_ENTRIES,
    SHORTCUT_CATALOG_SCHEMA_VERSION,
};
pub use parser::parse_trigger;
pub use sequence::{
    KeyDevice, KeyEvent, KeyEventPhase, KeyPropagation, SequenceEngine, SequenceEngineError,
    SequenceOutcome, MAX_ACTIVE_DEVICES, MAX_SEQUENCE_BINDINGS,
};
pub use types::{
    Chord, HotkeyAction, HotkeyBinding, HotkeyError, KeyCode, LogicalKey, Modifiers, PhysicalKey,
    Trigger, DEFAULT_SEQUENCE_TIMEOUT_MS, MAX_SEQUENCE_STEPS,
};
