mod clipboard;
mod foreground;
mod keyboard;
mod ocr;

pub use clipboard::{WindowsClipboard, WindowsCopySynthesizer};
pub use foreground::WindowsForegroundAppProvider;
pub use keyboard::{WindowsKeyEventSource, WindowsRegistrationProbe};
pub use ocr::WindowsMediaOcr;
