use std::{
    ffi::{c_char, c_void, CStr},
    path::Path,
};

use crate::hotkeys::{AppIdentity, ForegroundAppProvider, PlatformError};

type ObjcId = *mut c_void;
type Sel = *mut c_void;

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> ObjcId;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[derive(Clone, Copy, Debug, Default)]
pub struct MacForegroundAppProvider;

impl ForegroundAppProvider for MacForegroundAppProvider {
    fn current(&self) -> Result<AppIdentity, PlatformError> {
        unsafe {
            let pool_class = class(b"NSAutoreleasePool\0")?;
            let pool = send_id(pool_class, selector(b"new\0")?);
            let result = current_identity();
            if !pool.is_null() {
                send_void(pool, selector(b"drain\0")?);
            }
            result
        }
    }
}

unsafe fn current_identity() -> Result<AppIdentity, PlatformError> {
    let workspace_class = class(b"NSWorkspace\0")?;
    let workspace = send_id(workspace_class, selector(b"sharedWorkspace\0")?);
    let application = send_id(workspace, selector(b"frontmostApplication\0")?);
    if application.is_null() {
        return Err(PlatformError::IdentityUnavailable);
    }
    let bundle = nsstring(send_id(application, selector(b"bundleIdentifier\0")?));
    let executable_url = send_id(application, selector(b"executableURL\0")?);
    let path_value = if executable_url.is_null() {
        None
    } else {
        nsstring(send_id(executable_url, selector(b"path\0")?))
    };
    let basename = path_value.as_deref().and_then(|value| {
        Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
    });
    AppIdentity::new(basename.as_deref(), bundle.as_deref())
        .ok_or(PlatformError::IdentityUnavailable)
}

unsafe fn class(name: &'static [u8]) -> Result<ObjcId, PlatformError> {
    let value = objc_getClass(name.as_ptr().cast());
    if value.is_null() {
        Err(PlatformError::BackendUnavailable)
    } else {
        Ok(value)
    }
}

unsafe fn selector(name: &'static [u8]) -> Result<Sel, PlatformError> {
    let value = sel_registerName(name.as_ptr().cast());
    if value.is_null() {
        Err(PlatformError::BackendUnavailable)
    } else {
        Ok(value)
    }
}

unsafe fn send_id(receiver: ObjcId, selector: Sel) -> ObjcId {
    let function: unsafe extern "C" fn(ObjcId, Sel) -> ObjcId =
        std::mem::transmute(objc_msgSend as *const ());
    function(receiver, selector)
}

unsafe fn nsstring(value: ObjcId) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let utf8 = send_cstr(value, selector(b"UTF8String\0").ok()?);
    if utf8.is_null() {
        return None;
    }
    CStr::from_ptr(utf8).to_str().ok().map(str::to_owned)
}

unsafe fn send_cstr(receiver: ObjcId, selector: Sel) -> *const c_char {
    let function: unsafe extern "C" fn(ObjcId, Sel) -> *const c_char =
        std::mem::transmute(objc_msgSend as *const ());
    function(receiver, selector)
}

unsafe fn send_void(receiver: ObjcId, selector: Sel) {
    let function: unsafe extern "C" fn(ObjcId, Sel) =
        std::mem::transmute(objc_msgSend as *const ());
    function(receiver, selector)
}
