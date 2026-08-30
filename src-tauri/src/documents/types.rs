use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    Docx,
    Pptx,
    Xlsx,
}

impl DocumentFormat {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "docx" => Some(Self::Docx),
            "pptx" => Some(Self::Pptx),
            "xlsx" => Some(Self::Xlsx),
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
}
impl Default for DocumentOptions {
    fn default() -> Self {
        Self {
            include_comments: true,
            include_notes: true,
            include_hidden: false,
            wrap_text: true,
            target_language: "ko".into(),
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
}

pub struct DocumentPlan {
    pub source: PathBuf,
    pub format: DocumentFormat,
    pub manifest: DocumentManifest,
    pub segments: Vec<Segment>,
}
