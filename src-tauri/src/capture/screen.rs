use std::{
    io::{self, Cursor, Write},
    path::Path,
    sync::Mutex,
    thread,
    time::Duration,
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder, RgbaImage};
use serde::Serialize;
use uuid::Uuid;

use super::{
    image_input::{DecodedImage, SourceFingerprint},
    CapturePermission, CaptureSelection, LogicalRect, MonitorInfo, PixelRect,
};

const MAX_MONITORS: usize = 16;
const MAX_CAPTURE_PIXELS: u64 = 120_000_000;
const MIN_SELECTION: u32 = 8;
const MAX_STORAGE_ATTEMPTS: usize = 3;
const STORAGE_RETRY_DELAY: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScreenCaptureError {
    #[error("screen recording permission is required")]
    PermissionRequired { settings_url: &'static str },
    #[error("screen capture is blocked for the active application")]
    BlockedApplication,
    #[error("screen capture is already active")]
    AlreadyActive,
    #[error("the screen capture session is unavailable")]
    SessionUnavailable,
    #[error("the capture selection is invalid")]
    InvalidSelection,
    #[error("too many or too large displays")]
    DisplayLimitExceeded,
    #[error("screen capture is unavailable")]
    BackendUnavailable,
    #[error("the capture image could not be encoded")]
    EncodingFailed,
    #[error("the capture image file could not be opened")]
    StorageOpenUnavailable,
    #[error("the capture storage root is unavailable")]
    StorageRootUnavailable,
    #[error("the capture image file could not be written")]
    StorageWriteUnavailable,
    #[error("the capture image could not be stored")]
    StorageUnavailable,
}

impl ScreenCaptureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PermissionRequired { .. } => "screen_recording_permission_required",
            Self::BlockedApplication => "screen_capture_blocked_application",
            Self::AlreadyActive => "screen_capture_already_active",
            Self::SessionUnavailable => "screen_capture_session_unavailable",
            Self::InvalidSelection => "invalid_capture_selection",
            Self::DisplayLimitExceeded => "capture_display_limit_exceeded",
            Self::BackendUnavailable => "screen_capture_unavailable",
            Self::EncodingFailed => "capture_encoding_failed",
            Self::StorageOpenUnavailable => "capture_storage_open_failed",
            Self::StorageRootUnavailable => "capture_storage_root_failed",
            Self::StorageWriteUnavailable => "capture_storage_write_failed",
            Self::StorageUnavailable => "capture_storage_unavailable",
        }
    }
}

#[derive(Clone)]
struct MonitorFrame {
    info: MonitorInfo,
    rgba: Vec<u8>,
}

#[async_trait]
pub trait ScreenCapturePort: Send + Sync {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, ScreenCaptureError>;
    async fn capture(
        &self,
        selection: CaptureSelection,
    ) -> Result<DecodedImage, ScreenCaptureError>;
    async fn permission(&self) -> Result<CapturePermission, ScreenCaptureError>;
}

#[derive(Clone, Copy, Default)]
pub struct NativeScreenCapture;

#[async_trait]
impl ScreenCapturePort for NativeScreenCapture {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, ScreenCaptureError> {
        native::monitors()
    }

    async fn capture(
        &self,
        selection: CaptureSelection,
    ) -> Result<DecodedImage, ScreenCaptureError> {
        let frames = native::capture_all()?;
        let image = compose_selection(&frames, selection.global_physical)?;
        Ok(DecodedImage {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
            source: screen_fingerprint(selection.global_physical),
            immutable_copy: Default::default(),
        })
    }

    async fn permission(&self) -> Result<CapturePermission, ScreenCaptureError> {
        native::permission()
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayDescriptor {
    pub session_id: Uuid,
    pub monitor: MonitorInfo,
    pub background_data_url: String,
}

struct CaptureSession {
    id: Uuid,
    frames: Vec<MonitorFrame>,
}

#[derive(Default)]
pub struct CaptureCoordinator {
    session: Mutex<Option<CaptureSession>>,
}

impl CaptureCoordinator {
    pub fn begin(&self) -> Result<(Uuid, Vec<MonitorInfo>), ScreenCaptureError> {
        if native::permission()? != CapturePermission::Granted {
            return Err(ScreenCaptureError::PermissionRequired {
                settings_url: permission_settings_url(),
            });
        }
        // Hold the short-lived session lock across the native snapshot so two
        // UI requests cannot both allocate a full display set.
        let mut current = self
            .session
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        if current.is_some() {
            return Err(ScreenCaptureError::AlreadyActive);
        }
        let frames = native::capture_all()?;
        validate_frames(&frames)?;
        let id = Uuid::new_v4();
        let monitors = frames.iter().map(|frame| frame.info.clone()).collect();
        *current = Some(CaptureSession { id, frames });
        Ok((id, monitors))
    }

    pub fn overlay(
        &self,
        session_id: Uuid,
        monitor_id: &str,
    ) -> Result<OverlayDescriptor, ScreenCaptureError> {
        let current = self
            .session
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        let session = current
            .as_ref()
            .filter(|value| value.id == session_id)
            .ok_or(ScreenCaptureError::SessionUnavailable)?;
        let frame = session
            .frames
            .iter()
            .find(|frame| frame.info.id == monitor_id)
            .ok_or(ScreenCaptureError::SessionUnavailable)?;
        let png = encode_png(
            frame.info.physical_bounds.width,
            frame.info.physical_bounds.height,
            &frame.rgba,
        )?;
        Ok(OverlayDescriptor {
            session_id,
            monitor: frame.info.clone(),
            background_data_url: format!("data:image/png;base64,{}", STANDARD.encode(png)),
        })
    }

    pub fn complete(
        &self,
        session_id: Uuid,
        selection: CaptureSelection,
        immutable_root: &Path,
    ) -> Result<DecodedImage, ScreenCaptureError> {
        let mut current = self
            .session
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        let session = current
            .as_ref()
            .filter(|value| value.id == session_id)
            .ok_or(ScreenCaptureError::SessionUnavailable)?;
        let image = compose_selection(&session.frames, selection.global_physical)?;
        std::fs::create_dir_all(immutable_root)
            .map_err(|_| ScreenCaptureError::StorageRootUnavailable)?;
        let path = immutable_root.join(format!("{}.png", Uuid::new_v4()));
        let png = encode_png(image.width(), image.height(), image.as_raw())?;
        let mut output = retry_transient_io(|| {
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
        })
        .map_err(|_| ScreenCaptureError::StorageOpenUnavailable)?;
        output
            .write_all(&png)
            .and_then(|_| output.flush())
            .map_err(|_| ScreenCaptureError::StorageWriteUnavailable)?;
        let _completed = current.take();
        Ok(DecodedImage {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
            source: SourceFingerprint {
                sha256: sha256_hex(&png),
                input_bytes: png.len() as u64,
                original_width: selection.global_physical.width,
                original_height: selection.global_physical.height,
                orientation: 1,
                color_type: "Rgba8".to_owned(),
                has_embedded_icc: false,
                format: "png".to_owned(),
            },
            immutable_copy: path,
        })
    }

    pub fn cancel(&self, session_id: Uuid) -> Result<(), ScreenCaptureError> {
        let mut current = self
            .session
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        match current.as_ref() {
            Some(value) if value.id == session_id => {
                *current = None;
                Ok(())
            }
            _ => Err(ScreenCaptureError::SessionUnavailable),
        }
    }
}

fn validate_frames(frames: &[MonitorFrame]) -> Result<(), ScreenCaptureError> {
    if frames.is_empty() || frames.len() > MAX_MONITORS {
        return Err(ScreenCaptureError::DisplayLimitExceeded);
    }
    let mut pixels = 0_u64;
    for frame in frames {
        frame
            .info
            .validate()
            .map_err(|_| ScreenCaptureError::BackendUnavailable)?;
        pixels = pixels.saturating_add(
            u64::from(frame.info.physical_bounds.width)
                * u64::from(frame.info.physical_bounds.height),
        );
        let expected = usize::try_from(
            u64::from(frame.info.physical_bounds.width)
                * u64::from(frame.info.physical_bounds.height)
                * 4,
        )
        .map_err(|_| ScreenCaptureError::DisplayLimitExceeded)?;
        if frame.rgba.len() != expected {
            return Err(ScreenCaptureError::BackendUnavailable);
        }
    }
    if pixels > MAX_CAPTURE_PIXELS {
        return Err(ScreenCaptureError::DisplayLimitExceeded);
    }
    Ok(())
}

fn compose_selection(
    frames: &[MonitorFrame],
    selection: PixelRect,
) -> Result<RgbaImage, ScreenCaptureError> {
    if selection.width < MIN_SELECTION
        || selection.height < MIN_SELECTION
        || u64::from(selection.width) * u64::from(selection.height) > MAX_CAPTURE_PIXELS
    {
        return Err(ScreenCaptureError::InvalidSelection);
    }
    let mut output = RgbaImage::new(selection.width, selection.height);
    let mut covered = 0_u64;
    for frame in frames {
        let bounds = frame.info.physical_bounds;
        let left = selection.x.max(bounds.x);
        let top = selection.y.max(bounds.y);
        let right = (i64::from(selection.x) + i64::from(selection.width))
            .min(i64::from(bounds.x) + i64::from(bounds.width)) as i32;
        let bottom = (i64::from(selection.y) + i64::from(selection.height))
            .min(i64::from(bounds.y) + i64::from(bounds.height)) as i32;
        if right <= left || bottom <= top {
            continue;
        }
        for global_y in top..bottom {
            for global_x in left..right {
                let source_x = (global_x - bounds.x) as u32;
                let source_y = (global_y - bounds.y) as u32;
                let source_index = ((source_y * bounds.width + source_x) * 4) as usize;
                let target_x = (global_x - selection.x) as u32;
                let target_y = (global_y - selection.y) as u32;
                output.put_pixel(
                    target_x,
                    target_y,
                    image::Rgba(
                        frame.rgba[source_index..source_index + 4]
                            .try_into()
                            .unwrap(),
                    ),
                );
                covered += 1;
            }
        }
    }
    if covered == 0 {
        return Err(ScreenCaptureError::InvalidSelection);
    }
    Ok(output)
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ScreenCaptureError> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(ScreenCaptureError::EncodingFailed)?;
    if rgba.len() != expected {
        return Err(ScreenCaptureError::EncodingFailed);
    }
    let mut bytes = Cursor::new(Vec::new());
    PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, ColorType::Rgba8.into())
        .map_err(|_| ScreenCaptureError::EncodingFailed)?;
    Ok(bytes.into_inner())
}

fn retry_transient_io<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for attempt in 1..=MAX_STORAGE_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if attempt < MAX_STORAGE_ATTEMPTS
                    && matches!(
                        error.kind(),
                        io::ErrorKind::PermissionDenied | io::ErrorKind::WouldBlock
                    ) =>
            {
                thread::sleep(STORAGE_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded retry loop always returns")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}

fn screen_fingerprint(bounds: PixelRect) -> SourceFingerprint {
    SourceFingerprint {
        sha256: String::new(),
        input_bytes: 0,
        original_width: bounds.width,
        original_height: bounds.height,
        orientation: 1,
        color_type: "Rgba8".to_owned(),
        has_embedded_icc: false,
        format: "screen".to_owned(),
    }
}

#[cfg(windows)]
fn permission_settings_url() -> &'static str {
    "ms-settings:privacy-screenshotborders"
}
#[cfg(target_os = "macos")]
fn permission_settings_url() -> &'static str {
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
            EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HDC, HMONITOR,
            MONITORINFOEXW, SRCCOPY,
        },
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        UI::WindowsAndMessaging::MONITORINFOF_PRIMARY,
    };

    pub fn permission() -> Result<CapturePermission, ScreenCaptureError> {
        Ok(CapturePermission::Granted)
    }

    pub fn monitors() -> Result<Vec<MonitorInfo>, ScreenCaptureError> {
        let mut monitors = Vec::<(HMONITOR, MonitorInfo)>::new();
        unsafe extern "system" fn callback(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            let values = &mut *(data as *mut Vec<(HMONITOR, MonitorInfo)>);
            let mut raw = MONITORINFOEXW::default();
            raw.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(monitor, &mut raw.monitorInfo) == 0 {
                return 1;
            }
            let rect = raw.monitorInfo.rcMonitor;
            let mut dpi_x = 96_u32;
            let mut dpi_y = 96_u32;
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let scale = f64::from(dpi_x.max(1)) / 96.0;
            let end = raw
                .szDevice
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(raw.szDevice.len());
            let name = String::from_utf16_lossy(&raw.szDevice[..end]);
            let physical = PixelRect {
                x: rect.left,
                y: rect.top,
                width: (rect.right - rect.left).max(0) as u32,
                height: (rect.bottom - rect.top).max(0) as u32,
            };
            values.push((
                monitor,
                MonitorInfo {
                    id: format!("display-{:x}", monitor as usize),
                    name,
                    physical_bounds: physical,
                    logical_bounds: LogicalRect {
                        x: physical.x as f64 / scale,
                        y: physical.y as f64 / scale,
                        width: physical.width as f64 / scale,
                        height: physical.height as f64 / scale,
                    },
                    scale_factor: scale,
                    primary: raw.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                },
            ));
            1
        }
        let ok = unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(callback),
                &mut monitors as *mut _ as LPARAM,
            )
        };
        if ok == 0 {
            return Err(ScreenCaptureError::BackendUnavailable);
        }
        Ok(monitors.into_iter().map(|(_, info)| info).collect())
    }

    pub fn capture_all() -> Result<Vec<MonitorFrame>, ScreenCaptureError> {
        let infos = monitors()?;
        infos
            .into_iter()
            .map(|info| capture_monitor(info))
            .collect()
    }

    fn capture_monitor(info: MonitorInfo) -> Result<MonitorFrame, ScreenCaptureError> {
        let bounds = info.physical_bounds;
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return Err(ScreenCaptureError::BackendUnavailable);
            }
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, bounds.width as i32, bounds.height as i32);
            if memory.is_null() || bitmap.is_null() {
                if !memory.is_null() {
                    DeleteDC(memory);
                }
                ReleaseDC(std::ptr::null_mut(), screen);
                return Err(ScreenCaptureError::BackendUnavailable);
            }
            let old = SelectObject(memory, bitmap);
            let copied = BitBlt(
                memory,
                0,
                0,
                bounds.width as i32,
                bounds.height as i32,
                screen,
                bounds.x,
                bounds.y,
                SRCCOPY | CAPTUREBLT,
            );
            let mut bitmap_info = BITMAPINFO::default();
            bitmap_info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bounds.width as i32,
                biHeight: -(bounds.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            };
            let mut bgra = vec![0_u8; bounds.width as usize * bounds.height as usize * 4];
            let read = if copied != 0 {
                GetDIBits(
                    memory,
                    bitmap,
                    0,
                    bounds.height,
                    bgra.as_mut_ptr().cast(),
                    &mut bitmap_info,
                    DIB_RGB_COLORS,
                )
            } else {
                0
            };
            SelectObject(memory, old);
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
            if read == 0 {
                return Err(ScreenCaptureError::BackendUnavailable);
            }
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            Ok(MonitorFrame { info, rgba: bgra })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, ErrorKind};

    #[test]
    fn png_parameter_failure_has_safe_encoding_code() {
        let error = encode_png(1, 1, &[]).expect_err("invalid pixel data must fail");

        assert_eq!(error.code(), "capture_encoding_failed");
        assert_eq!(error.to_string(), "the capture image could not be encoded");
    }

    #[test]
    fn storage_open_failure_has_safe_stage_code() {
        assert_eq!(
            ScreenCaptureError::StorageOpenUnavailable.code(),
            "capture_storage_open_failed"
        );
    }

    #[test]
    fn storage_root_and_write_failures_have_safe_stage_codes() {
        assert_eq!(
            ScreenCaptureError::StorageRootUnavailable.code(),
            "capture_storage_root_failed"
        );
        assert_eq!(
            ScreenCaptureError::StorageWriteUnavailable.code(),
            "capture_storage_write_failed"
        );
    }

    #[test]
    fn transient_io_retry_succeeds_on_a_later_attempt() {
        let mut attempts = 0;

        let result = retry_transient_io(|| {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::from(ErrorKind::WouldBlock))
            } else {
                Ok("stored")
            }
        });

        assert_eq!(result.expect("second attempt must succeed"), "stored");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn transient_io_retry_stops_after_three_attempts() {
        let mut attempts = 0;

        let error = retry_transient_io(|| -> io::Result<()> {
            attempts += 1;
            Err(io::Error::from(ErrorKind::PermissionDenied))
        })
        .expect_err("persistent transient errors must stop");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn transient_io_retry_does_not_retry_other_errors() {
        let mut attempts = 0;

        let error = retry_transient_io(|| -> io::Result<()> {
            attempts += 1;
            Err(io::Error::from(ErrorKind::AlreadyExists))
        })
        .expect_err("non-retryable errors must be returned");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(attempts, 1);
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::{ffi::c_void, process::Command};
    type CGDirectDisplayID = u32;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGGetActiveDisplayList(
            max: u32,
            displays: *mut CGDirectDisplayID,
            count: *mut u32,
        ) -> i32;
        fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
        fn CGDisplayPixelsWide(display: CGDirectDisplayID) -> usize;
        fn CGDisplayPixelsHigh(display: CGDirectDisplayID) -> usize;
        fn CGMainDisplayID() -> CGDirectDisplayID;
    }
    pub fn permission() -> Result<CapturePermission, ScreenCaptureError> {
        Ok(if unsafe { CGPreflightScreenCaptureAccess() } {
            CapturePermission::Granted
        } else {
            CapturePermission::PermissionRequired
        })
    }
    pub fn monitors() -> Result<Vec<MonitorInfo>, ScreenCaptureError> {
        let mut ids = [0_u32; MAX_MONITORS];
        let mut count = 0_u32;
        if unsafe { CGGetActiveDisplayList(MAX_MONITORS as u32, ids.as_mut_ptr(), &mut count) } != 0
        {
            return Err(ScreenCaptureError::BackendUnavailable);
        }
        ids[..count as usize]
            .iter()
            .map(|id| {
                let bounds = unsafe { CGDisplayBounds(*id) };
                let width = unsafe { CGDisplayPixelsWide(*id) } as u32;
                let height = unsafe { CGDisplayPixelsHigh(*id) } as u32;
                let scale = if bounds.size.width > 0.0 {
                    width as f64 / bounds.size.width
                } else {
                    1.0
                };
                Ok(MonitorInfo {
                    id: format!("display-{id}"),
                    name: format!("Display {id}"),
                    physical_bounds: PixelRect {
                        x: (bounds.origin.x * scale).round() as i32,
                        y: (bounds.origin.y * scale).round() as i32,
                        width,
                        height,
                    },
                    logical_bounds: LogicalRect {
                        x: bounds.origin.x,
                        y: bounds.origin.y,
                        width: bounds.size.width,
                        height: bounds.size.height,
                    },
                    scale_factor: scale,
                    primary: *id == unsafe { CGMainDisplayID() },
                })
            })
            .collect()
    }
    pub fn capture_all() -> Result<Vec<MonitorFrame>, ScreenCaptureError> {
        if permission()? != CapturePermission::Granted {
            return Err(ScreenCaptureError::PermissionRequired {
                settings_url: permission_settings_url(),
            });
        }
        monitors()?
            .into_iter()
            .map(|info| {
                let temp = tempfile::Builder::new()
                    .suffix(".png")
                    .tempfile()
                    .map_err(|_| ScreenCaptureError::StorageUnavailable)?;
                let rect = info.logical_bounds;
                let region = format!(
                    "{},{},{},{}",
                    rect.x.round() as i64,
                    rect.y.round() as i64,
                    rect.width.round() as u64,
                    rect.height.round() as u64
                );
                let status = Command::new("/usr/sbin/screencapture")
                    .args(["-x", "-R", &region])
                    .arg(temp.path())
                    .status()
                    .map_err(|_| ScreenCaptureError::BackendUnavailable)?;
                if !status.success() {
                    return Err(ScreenCaptureError::PermissionRequired {
                        settings_url: permission_settings_url(),
                    });
                }
                let image = image::open(temp.path())
                    .map_err(|_| ScreenCaptureError::BackendUnavailable)?
                    .to_rgba8();
                Ok(MonitorFrame {
                    info,
                    rgba: image.into_raw(),
                })
            })
            .collect()
    }
}
