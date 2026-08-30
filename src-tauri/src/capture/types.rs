use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_OCR_LINES: usize = 20_000;
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoordinateSpace {
    PhysicalPixels,
    LogicalPoints,
    Normalized,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn validate_local(self, width: u32, height: u32) -> Result<(), ContractError> {
        if self.x < 0 || self.y < 0 || self.width == 0 || self.height == 0 {
            return Err(ContractError::InvalidRectangle);
        }
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        if right > i64::from(width) || bottom > i64::from(height) {
            return Err(ContractError::InvalidRectangle);
        }
        Ok(())
    }

    pub fn normalize(self, width: u32, height: u32) -> Result<NormalizedRect, ContractError> {
        self.validate_local(width, height)?;
        NormalizedRect::new(
            self.x as f32 / width as f32,
            self.y as f32 / height as f32,
            self.width as f32 / width as f32,
            self.height as f32 / height as f32,
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl LogicalRect {
    pub fn validate(self) -> Result<(), ContractError> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|value| !value.is_finite())
            || self.width <= 0.0
            || self.height <= 0.0
        {
            return Err(ContractError::InvalidRectangle);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl NormalizedRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, ContractError> {
        let value = Self {
            x,
            y,
            width,
            height,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(self) -> Result<(), ContractError> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|value| !value.is_finite())
            || self.x < 0.0
            || self.y < 0.0
            || self.width <= 0.0
            || self.height <= 0.0
            || self.x + self.width > 1.000_001
            || self.y + self.height > 1.000_001
        {
            return Err(ContractError::InvalidRectangle);
        }
        Ok(())
    }

    pub fn denormalize(self, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x: (self.x * width as f32).round() as i32,
            y: (self.y * height as f32).round() as i32,
            width: (self.width * width as f32).round().max(1.0) as u32,
            height: (self.height * height as f32).round().max(1.0) as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TextDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub id: Uuid,
    pub text: String,
    pub bounds: NormalizedRect,
    pub confidence: f32,
    pub angle_degrees: f32,
    pub direction: TextDirection,
    #[serde(default)]
    pub polygon: Vec<NormalizedPoint>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedPoint {
    pub x: f32,
    pub y: f32,
}

impl OcrLine {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.text.len() > MAX_TEXT_BYTES
            || self.confidence.is_nan()
            || !(0.0..=1.0).contains(&self.confidence)
            || !self.angle_degrees.is_finite()
        {
            return Err(ContractError::InvalidOcrData);
        }
        self.bounds.validate().and_then(|_| {
            if self.polygon.len() > 16
                || self.polygon.iter().any(|point| {
                    !point.x.is_finite()
                        || !point.y.is_finite()
                        || !(0.0..=1.0).contains(&point.x)
                        || !(0.0..=1.0).contains(&point.y)
                })
                || self.language.as_ref().is_some_and(|value| value.len() > 64)
            {
                Err(ContractError::InvalidOcrData)
            } else {
                Ok(())
            }
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrDocument {
    pub image_width: u32,
    pub image_height: u32,
    pub lines: Vec<OcrLine>,
    pub source_language: Option<String>,
}

impl OcrDocument {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.image_width == 0 || self.image_height == 0 || self.lines.len() > MAX_OCR_LINES {
            return Err(ContractError::InvalidOcrData);
        }
        self.lines.iter().try_for_each(OcrLine::validate)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedBlock {
    pub id: Uuid,
    pub source_ids: Vec<Uuid>,
    pub source_text: String,
    pub translated_text: String,
    pub bounds: NormalizedRect,
    pub confidence: f32,
    #[serde(default)]
    pub direction: Option<TextDirection>,
    #[serde(default = "default_visible")]
    pub visible: bool,
}

fn default_visible() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureJobResult {
    pub job_id: Uuid,
    pub status: CaptureJobStatus,
    pub image_width: u32,
    pub image_height: u32,
    pub ocr: Option<OcrDocument>,
    pub translated_blocks: Vec<TranslatedBlock>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub source_preview: Option<String>,
    #[serde(default)]
    pub translated_preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CaptureJobStatus {
    SourceReady,
    OcrReady,
    Translated,
    Rendered,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub id: String,
    pub name: String,
    pub physical_bounds: PixelRect,
    pub logical_bounds: LogicalRect,
    pub scale_factor: f64,
    pub primary: bool,
}

impl MonitorInfo {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.id.is_empty()
            || self.id.len() > 128
            || self.name.len() > 256
            || self.id.chars().any(char::is_control)
            || self.name.chars().any(char::is_control)
            || !self.scale_factor.is_finite()
            || self.scale_factor <= 0.0
            || self.scale_factor > 8.0
            || self.physical_bounds.width == 0
            || self.physical_bounds.height == 0
        {
            return Err(ContractError::InvalidMonitor);
        }
        self.logical_bounds.validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSelection {
    pub global_physical: PixelRect,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CapturePermission {
    Granted,
    PermissionRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("invalid rectangle")]
    InvalidRectangle,
    #[error("invalid monitor")]
    InvalidMonitor,
    #[error("invalid OCR data")]
    InvalidOcrData,
}
