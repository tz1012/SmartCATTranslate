use async_trait::async_trait;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use super::{DecodedImage, NormalizedPoint, NormalizedRect, OcrDocument, OcrLine, TextDirection};

#[async_trait]
pub trait OcrEngine: Send + Sync {
    async fn recognize(
        &self,
        image: &DecodedImage,
        language_hints: &[String],
    ) -> Result<OcrDocument, OcrError>;
}

#[cfg(windows)]
pub type NativeOcrEngine = crate::platform::windows::WindowsMediaOcr;
#[cfg(target_os = "macos")]
pub type NativeOcrEngine = crate::platform::macos::MacVisionOcr;
#[cfg(not(any(windows, target_os = "macos")))]
pub type NativeOcrEngine = UnsupportedOcr;

#[cfg(not(any(windows, target_os = "macos")))]
#[derive(Default)]
pub struct UnsupportedOcr;

#[cfg(not(any(windows, target_os = "macos")))]
#[async_trait]
impl OcrEngine for UnsupportedOcr {
    async fn recognize(&self, _: &DecodedImage, _: &[String]) -> Result<OcrDocument, OcrError> {
        Err(OcrError::UnsupportedOsVersion)
    }
}

pub(crate) fn normalize_lines(
    width: u32,
    height: u32,
    source_language: Option<String>,
    native: Vec<NativeOcrLine>,
) -> Result<OcrDocument, OcrError> {
    if width == 0 || height == 0 || native.len() > super::types::MAX_OCR_LINES {
        return Err(OcrError::InvalidResult);
    }
    let mut lines = Vec::with_capacity(native.len());
    for line in native {
        let bounds = NormalizedRect::new(
            (line.x / width as f32).clamp(0.0, 1.0),
            (line.y / height as f32).clamp(0.0, 1.0),
            (line.width / width as f32).clamp(f32::EPSILON, 1.0),
            (line.height / height as f32).clamp(f32::EPSILON, 1.0),
        )
        .map_err(|_| OcrError::InvalidResult)?;
        let right = (bounds.x + bounds.width).min(1.0);
        let bottom = (bounds.y + bounds.height).min(1.0);
        let bounds = NormalizedRect::new(bounds.x, bounds.y, right - bounds.x, bottom - bounds.y)
            .map_err(|_| OcrError::InvalidResult)?;
        let polygon = line
            .polygon
            .into_iter()
            .take(16)
            .map(|(x, y)| NormalizedPoint {
                x: (x / width as f32).clamp(0.0, 1.0),
                y: (y / height as f32).clamp(0.0, 1.0),
            })
            .collect();
        lines.push(OcrLine {
            id: Uuid::new_v4(),
            text: line.text.nfc().collect(),
            bounds,
            confidence: line.confidence.clamp(0.0, 1.0),
            angle_degrees: if line.angle_degrees.is_finite() {
                line.angle_degrees
            } else {
                0.0
            },
            direction: line.direction,
            polygon,
            language: line.language.filter(|value| value.len() <= 64),
        });
    }
    lines.sort_by(|left, right| {
        let row_tolerance = left.bounds.height.min(right.bounds.height) * 0.45;
        if (left.bounds.y - right.bounds.y).abs() <= row_tolerance {
            match left.direction {
                TextDirection::RightToLeft => right.bounds.x.total_cmp(&left.bounds.x),
                _ => left.bounds.x.total_cmp(&right.bounds.x),
            }
        } else {
            left.bounds.y.total_cmp(&right.bounds.y)
        }
    });
    let document = OcrDocument {
        image_width: width,
        image_height: height,
        lines,
        source_language,
    };
    document.validate().map_err(|_| OcrError::InvalidResult)?;
    Ok(document)
}

pub(crate) struct NativeOcrLine {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
    pub angle_degrees: f32,
    pub direction: TextDirection,
    pub polygon: Vec<(f32, f32)>,
    pub language: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("OCR permission is required")]
    PermissionRequired { settings_url: String },
    #[error("the requested OCR language pack is not installed")]
    LanguagePackMissing { requested: Vec<String> },
    #[error("OCR is unavailable on this operating system version")]
    UnsupportedOsVersion,
    #[error("the OCR result was invalid")]
    InvalidResult,
    #[error("the native OCR service failed")]
    NativeFailure,
}

impl OcrError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PermissionRequired { .. } => "ocr_permission_required",
            Self::LanguagePackMissing { .. } => "ocr_language_pack_missing",
            Self::UnsupportedOsVersion => "ocr_unsupported_os_version",
            Self::InvalidResult => "ocr_invalid_result",
            Self::NativeFailure => "ocr_native_failure",
        }
    }
}
