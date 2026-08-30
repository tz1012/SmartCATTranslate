mod blocklist;
mod clipboard;
mod conflicts;
mod native;
mod parser;
mod sequence;
mod types;

pub use blocklist::{BlockedApp, Blocklist, BlocklistError, MAX_BLOCKED_APPS};
pub use clipboard::{
    CaptureError, CapturedSelection, ClipboardFormat, ClipboardFormatId, ClipboardGuard,
    ClipboardLimits, ClipboardPort, ClipboardSnapshot, CopySynthesizer, RestoreStatus,
    SelectedTextAcquirer, MAX_CLIPBOARD_FORMATS, MAX_CLIPBOARD_SNAPSHOT_BYTES, MAX_SELECTION_BYTES,
};
pub use conflicts::{
    AppIdentity, AppInspector, CatalogEntry, CatalogError, CatalogKind, ConflictAnalyzer,
    ConflictCause, ConflictLevel, ConflictReport, ConflictSeverity, Platform, RegistrationProbe,
    RegistrationProbeStatus, ShortcutCatalog, MAX_CATALOG_BYTES, MAX_CATALOG_ENTRIES,
    SHORTCUT_CATALOG_SCHEMA_VERSION,
};
pub use native::{
    ForegroundAppProvider, HotkeyObserver, KeyEventSource, NativeController, NativeControllerError,
    NativeEventReceiver, ObserverAvailability, PlatformError,
};
pub(crate) use native::{ObserverActivationGuard, ObserverExitHandshake};
pub use parser::parse_trigger;
pub use sequence::{
    KeyDevice, KeyEvent, KeyEventPhase, KeyPropagation, SequenceEngine, SequenceEngineError,
    SequenceOutcome, MAX_ACTIVE_DEVICES, MAX_SEQUENCE_BINDINGS,
};
pub use types::{
    Chord, HotkeyAction, HotkeyBinding, HotkeyError, KeyCode, LogicalKey, Modifiers, PhysicalKey,
    Trigger, DEFAULT_SEQUENCE_TIMEOUT_MS, MAX_SEQUENCE_STEPS,
};
