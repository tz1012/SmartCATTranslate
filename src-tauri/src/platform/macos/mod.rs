mod clipboard;
mod foreground;
mod keyboard;
mod ocr;

pub use clipboard::{MacClipboard, MacCopySynthesizer};
pub use foreground::MacForegroundAppProvider;
pub use keyboard::{MacKeyEventSource, MacRegistrationProbe};
pub use ocr::MacVisionOcr;
