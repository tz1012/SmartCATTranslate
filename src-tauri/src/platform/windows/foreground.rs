use std::path::Path;

use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};

use crate::hotkeys::{AppIdentity, ForegroundAppProvider, PlatformError};

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsForegroundAppProvider;

impl ForegroundAppProvider for WindowsForegroundAppProvider {
    fn current(&self) -> Result<AppIdentity, PlatformError> {
        // SAFETY: handles are checked and the process handle is closed on every path.
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return Err(PlatformError::IdentityUnavailable);
            }
            let mut process_id = 0;
            GetWindowThreadProcessId(window, &mut process_id);
            if process_id == 0 {
                return Err(PlatformError::IdentityUnavailable);
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
            if process.is_null() {
                return Err(PlatformError::PermissionDenied);
            }
            let mut buffer = vec![0_u16; 32_768];
            let mut length = buffer.len() as u32;
            let queried = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut length,
            );
            CloseHandle(process);
            if queried == 0 || length == 0 || length as usize > buffer.len() {
                return Err(PlatformError::IdentityUnavailable);
            }
            let path = String::from_utf16(&buffer[..length as usize])
                .map_err(|_| PlatformError::IdentityUnavailable)?;
            let basename = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(PlatformError::IdentityUnavailable)?;
            AppIdentity::new(Some(basename), None).ok_or(PlatformError::IdentityUnavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::hotkeys::ForegroundAppProvider;

    use super::WindowsForegroundAppProvider;

    #[test]
    fn real_foreground_identity_exposes_only_a_sanitized_basename() {
        let identity = WindowsForegroundAppProvider.current().unwrap();
        let executable = identity.executable_basename().unwrap();

        assert!(!executable.contains('\\'));
        assert!(!executable.contains('/'));
        assert!(identity.bundle_id().is_none());
    }
}
