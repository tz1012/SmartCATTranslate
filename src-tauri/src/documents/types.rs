use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    Docx,
    Pptx,
    Xlsx,
    Pdf,
}

impl DocumentFormat {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "docx" => Some(Self::Docx),
            "pptx" => Some(Self::Pptx),
            "xlsx" => Some(Self::Xlsx),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocumentOptions {
    pub include_comments: bool,
    pub include_notes: bool,
    pub include_hidden: bool,
    pub wrap_text: bool,
    pub target_language: String,
    pub source_language: Option<String>,
    pub profile_id: Option<Uuid>,
    pub model: Option<String>,
    pub quality: Option<crate::core::types::Quality>,
    pub pdf_force_ocr: bool,
    pub pdf_fit: bool,
    pub preserve_annotations: bool,
    pub secret: bool,
    pub output_directory: Option<String>,
}
impl Default for DocumentOptions {
    fn default() -> Self {
        Self {
            include_comments: true,
            include_notes: true,
            include_hidden: false,
            wrap_text: true,
            target_language: "ko".into(),
            source_language: None,
            profile_id: None,
            model: None,
            quality: None,
            pdf_force_ocr: false,
            pdf_fit: true,
            preserve_annotations: true,
            secret: false,
            output_directory: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub id: Uuid,
    pub part: String,
    pub ordinal: usize,
    pub location: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct TranslatedSegment {
    pub id: Uuid,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentManifest {
    pub format: DocumentFormat,
    pub file_name: String,
    pub segment_count: usize,
    pub part_count: usize,
    pub source_hash: String,
    #[serde(default)]
    pub page_count: usize,
    #[serde(default)]
    pub page_kinds: Vec<String>,
    #[serde(default)]
    pub has_signatures: bool,
    #[serde(default)]
    pub has_forms: bool,
    #[serde(default)]
    pub has_annotations: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentWarning {
    pub code: String,
    pub location: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentReport {
    pub job_id: Uuid,
    pub format: DocumentFormat,
    pub output_path: String,
    pub output_name: String,
    pub translated_segments: usize,
    pub warnings: Vec<DocumentWarning>,
    pub publishable: bool,
    pub source_hash: String,
    pub output_hash: String,
    #[serde(default)]
    pub resumed_from_stage: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DocumentStage {
    Inspect,
    Extract,
    Ocr,
    Translate,
    Reflow,
    Save,
    Validate,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentCheckpoint {
    pub source_fingerprint: String,
    pub stage: DocumentStage,
    pub stable_unit_id: String,
    pub completed: usize,
    pub total: usize,
    #[serde(default)]
    pub raster_refs: Vec<String>,
    #[serde(default)]
    pub translated_result_refs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PdfRasterSpool {
    pub root: PathBuf,
    pub refs: std::collections::HashMap<u32, String>,
}

#[derive(Clone, Debug)]
pub struct DocumentResumeState {
    pub batch_cursor: usize,
    pub translated: Vec<TranslatedSegment>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("unsupported document format")]
    Unsupported,
    #[error("encrypted or macro-enabled documents are not supported")]
    UnsafePackage,
    #[error("invalid or oversized document package")]
    InvalidPackage,
    #[error("document source changed during translation")]
    SourceChanged,
    #[error("document output already exists")]
    OutputExists,
    #[error("document validation failed")]
    ValidationFailed,
    #[error("document translation cancelled")]
    Cancelled,
    #[error("document I/O failed")]
    Io,
    #[error("PDF password is required")]
    PasswordRequired,
    #[error("PDF limits exceeded")]
    LimitExceeded,
    #[error("PDF OCR is unavailable")]
    OcrUnavailable,
}

pub struct DocumentPlan {
    pub source: PathBuf,
    pub format: DocumentFormat,
    pub manifest: DocumentManifest,
    pub segments: Vec<Segment>,
    pub pdf_spool: Option<PdfRasterSpool>,
    pub resumed_from_stage: Option<String>,
}
