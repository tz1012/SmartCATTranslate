mod clipboard;
mod foreground;
mod keyboard;

pub use clipboard::{WindowsClipboard, WindowsCopySynthesizer};
pub use foreground::WindowsForegroundAppProvider;
pub use keyboard::{WindowsKeyEventSource, WindowsRegistrationProbe};
