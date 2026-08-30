use async_trait::async_trait;

use crate::capture::{DecodedImage, OcrDocument, OcrEngine, OcrError};

/// macOS Vision OCR entry point. The native bridge is isolated here so capture
/// pipeline code never depends on AppKit coordinate or permission semantics.
#[derive(Default)]
pub struct MacVisionOcr;

#[async_trait]
impl OcrEngine for MacVisionOcr {
    async fn recognize(
        &self,
        image: &DecodedImage,
        hints: &[String],
    ) -> Result<OcrDocument, OcrError> {
        vision_bridge::recognize(image, hints)
    }
}

#[cfg(target_os = "macos")]
mod vision_bridge {
    use std::{
        ffi::{CStr, CString},
        os::raw::{c_char, c_int},
    };

    use serde::Deserialize;

    use crate::capture::{
        ocr::{normalize_lines, NativeOcrLine},
        DecodedImage, OcrDocument, OcrError, TextDirection,
    };

    unsafe extern "C" {
        fn smartcat_vision_ocr(
            rgba: *const u8,
            width: c_int,
            height: c_int,
            hints_json: *const c_char,
            output_json: *mut *mut c_char,
            error_code: *mut *mut c_char,
        ) -> c_int;
        fn smartcat_free_string(value: *mut c_char);
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VisionLine {
        text: String,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        confidence: f32,
        angle_degrees: f32,
    }

    pub fn recognize(image: &DecodedImage, hints: &[String]) -> Result<OcrDocument, OcrError> {
        if hints
            .iter()
            .any(|tag| tag.len() > 64 || tag.chars().any(char::is_control))
        {
            return Err(OcrError::LanguagePackMissing {
                requested: hints.iter().take(8).cloned().collect(),
            });
        }
        let hints = hints.iter().take(8).cloned().collect::<Vec<_>>();
        let encoded =
            CString::new(serde_json::to_string(&hints).map_err(|_| OcrError::InvalidResult)?)
                .map_err(|_| OcrError::InvalidResult)?;
        let mut output = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let ok = unsafe {
            smartcat_vision_ocr(
                image.rgba.as_ptr(),
                image.width as c_int,
                image.height as c_int,
                encoded.as_ptr(),
                &mut output,
                &mut error,
            )
        };
        if ok == 0 {
            let code = if error.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() }
            };
            unsafe { smartcat_free_string(error) };
            return Err(match code.as_str() {
                "unsupported_os_version" => OcrError::UnsupportedOsVersion,
                "language_pack_missing" => OcrError::LanguagePackMissing { requested: hints },
                "invalid_result" => OcrError::InvalidResult,
                _ => OcrError::NativeFailure,
            });
        }
        if output.is_null() {
            return Err(OcrError::InvalidResult);
        }
        let json = unsafe { CStr::from_ptr(output).to_bytes().to_vec() };
        unsafe { smartcat_free_string(output) };
        let parsed: Vec<VisionLine> =
            serde_json::from_slice(&json).map_err(|_| OcrError::InvalidResult)?;
        let native = parsed
            .into_iter()
            .map(|line| NativeOcrLine {
                text: line.text,
                x: line.x,
                y: line.y,
                width: line.width,
                height: line.height,
                confidence: line.confidence,
                angle_degrees: line.angle_degrees,
                direction: TextDirection::LeftToRight,
                polygon: vec![
                    (line.x, line.y),
                    (line.x + line.width, line.y),
                    (line.x + line.width, line.y + line.height),
                    (line.x, line.y + line.height),
                ],
                language: hints.first().cloned(),
            })
            .collect();
        normalize_lines(image.width, image.height, hints.first().cloned(), native)
    }
}
