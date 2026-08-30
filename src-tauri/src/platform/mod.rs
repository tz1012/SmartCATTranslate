#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

#[cfg(not(any(windows, target_os = "macos")))]
compile_error!("SmartCAT platform hotkeys currently support Windows and macOS only");
