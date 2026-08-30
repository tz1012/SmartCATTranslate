use std::{mem::size_of, ptr, slice, thread, time::Duration};

use async_trait::async_trait;
use windows_sys::Win32::{
    Foundation::GlobalFree,
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
            GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
    },
};

use crate::hotkeys::{
    CaptureError, ClipboardFormat, ClipboardFormatId, ClipboardLimits, ClipboardPort,
    ClipboardSnapshot, CopySynthesizer,
};

const CLIPBOARD_OPEN_ATTEMPTS: usize = 8;
const CLIPBOARD_RETRY_DELAY: Duration = Duration::from_millis(8);
const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_PALETTE: u32 = 9;
const CF_UNICODETEXT: u32 = 13;
const CF_ENHMETAFILE: u32 = 14;
const CF_DSPBITMAP: u32 = 0x0082;
const CF_DSPMETAFILEPICT: u32 = 0x0083;
const CF_DSPENHMETAFILE: u32 = 0x008e;

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsClipboard;

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsCopySynthesizer;

impl ClipboardPort for WindowsClipboard {
    fn snapshot(&self, limits: ClipboardLimits) -> Result<ClipboardSnapshot, CaptureError> {
        let _open = OpenClipboardLease::acquire()?;
        let generation = unsafe { GetClipboardSequenceNumber() } as u64;
        let mut formats = Vec::new();
        let mut total = 0usize;
        let mut format = 0u32;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if formats.len() >= limits.max_formats || is_non_global_format(format) {
                return Err(CaptureError::ClipboardTooLarge);
            }
            let handle = unsafe { GetClipboardData(format) };
            if handle.is_null() {
                return Err(CaptureError::ClipboardAccessDenied);
            }
            let size = unsafe { GlobalSize(handle) };
            total = total
                .checked_add(size)
                .filter(|value| *value <= limits.max_bytes)
                .ok_or(CaptureError::ClipboardTooLarge)?;
            let locked = unsafe { GlobalLock(handle) };
            if locked.is_null() && size != 0 {
                return Err(CaptureError::ClipboardAccessDenied);
            }
            let data = if size == 0 {
                Vec::new()
            } else {
                unsafe { slice::from_raw_parts(locked.cast::<u8>(), size) }.to_vec()
            };
            if !locked.is_null() {
                unsafe { GlobalUnlock(handle) };
            }
            formats.push(ClipboardFormat::new(
                if format == CF_UNICODETEXT {
                    ClipboardFormatId::Text
                } else {
                    ClipboardFormatId::Native(format)
                },
                data,
            )?);
        }
        Ok(ClipboardSnapshot::new(generation, formats))
    }

    fn generation(&self) -> Result<u64, CaptureError> {
        Ok(unsafe { GetClipboardSequenceNumber() } as u64)
    }

    fn read_plain_text(&self, max_bytes: usize) -> Result<Option<String>, CaptureError> {
        let _open = OpenClipboardLease::acquire()?;
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
        if handle.is_null() {
            return Ok(None);
        }
        let size = unsafe { GlobalSize(handle) };
        if size > max_bytes.saturating_mul(2).saturating_add(2) {
            return Err(CaptureError::SelectionTooLarge);
        }
        let locked = unsafe { GlobalLock(handle) };
        if locked.is_null() {
            return Err(CaptureError::ClipboardAccessDenied);
        }
        let units = unsafe { slice::from_raw_parts(locked.cast::<u16>(), size / 2) };
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let text = String::from_utf16(&units[..end]).map_err(|_| CaptureError::BackendUnavailable);
        unsafe { GlobalUnlock(handle) };
        let text = text?;
        if text.len() > max_bytes {
            Err(CaptureError::SelectionTooLarge)
        } else {
            Ok(Some(text))
        }
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), CaptureError> {
        let mut allocations = Vec::with_capacity(snapshot.items().len());
        for item in snapshot.items() {
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, item.data.len().max(1)) };
            if handle.is_null() {
                free_allocations(&mut allocations);
                return Err(CaptureError::RestoreFailed);
            }
            let locked = unsafe { GlobalLock(handle) };
            if locked.is_null() {
                unsafe { GlobalFree(handle) };
                free_allocations(&mut allocations);
                return Err(CaptureError::RestoreFailed);
            }
            if !item.data.is_empty() {
                unsafe {
                    ptr::copy_nonoverlapping(item.data.as_ptr(), locked.cast(), item.data.len())
                };
            }
            unsafe { GlobalUnlock(handle) };
            let format = match item.id {
                ClipboardFormatId::Text => CF_UNICODETEXT,
                ClipboardFormatId::Native(format) => format,
                ClipboardFormatId::Named(_) => {
                    unsafe { GlobalFree(handle) };
                    free_allocations(&mut allocations);
                    return Err(CaptureError::RestoreFailed);
                }
            };
            allocations.push((format, handle));
        }

        let _open = OpenClipboardLease::acquire().map_err(|_| CaptureError::RestoreFailed)?;
        if unsafe { EmptyClipboard() } == 0 {
            free_allocations(&mut allocations);
            return Err(CaptureError::RestoreFailed);
        }
        let mut allocations = allocations.into_iter();
        while let Some((format, handle)) = allocations.next() {
            if unsafe { SetClipboardData(format, handle) }.is_null() {
                unsafe { GlobalFree(handle) };
                for (_, remaining) in allocations {
                    unsafe { GlobalFree(remaining) };
                }
                return Err(CaptureError::RestoreFailed);
            }
        }
        Ok(())
    }
}

#[async_trait]
impl CopySynthesizer for WindowsCopySynthesizer {
    async fn synthesize_copy(&self) -> Result<(), CaptureError> {
        let mut inputs = [
            keyboard_input(VK_CONTROL, 0),
            keyboard_input(b'C' as u16, 0),
            keyboard_input(b'C' as u16, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_mut_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent == inputs.len() as u32 {
            Ok(())
        } else {
            Err(CaptureError::CopyFailed)
        }
    }
}

fn keyboard_input(key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn is_non_global_format(format: u32) -> bool {
    matches!(
        format,
        CF_BITMAP
            | CF_METAFILEPICT
            | CF_PALETTE
            | CF_ENHMETAFILE
            | CF_DSPBITMAP
            | CF_DSPMETAFILEPICT
            | CF_DSPENHMETAFILE
    )
}

fn free_allocations(allocations: &mut Vec<(u32, *mut core::ffi::c_void)>) {
    for (_, handle) in allocations.drain(..) {
        unsafe { GlobalFree(handle) };
    }
}

struct OpenClipboardLease;

impl OpenClipboardLease {
    fn acquire() -> Result<Self, CaptureError> {
        for _ in 0..CLIPBOARD_OPEN_ATTEMPTS {
            if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                return Ok(Self);
            }
            thread::sleep(CLIPBOARD_RETRY_DELAY);
        }
        Err(CaptureError::ClipboardAccessDenied)
    }
}

impl Drop for OpenClipboardLease {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}
