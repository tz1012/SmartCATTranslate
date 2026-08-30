use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PdfPageKind {
    Text,
    Scanned,
    Mixed,
}

impl PdfPageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Scanned => "scanned",
            Self::Mixed => "mixed",
        }
    }
}

pub fn classify_page(
    non_whitespace: usize,
    covered_area: f32,
    has_large_image: bool,
) -> PdfPageKind {
    let has_text = non_whitespace >= 20 && covered_area >= 0.005;
    match (has_text, has_large_image) {
        (true, true) => PdfPageKind::Mixed,
        (true, false) => PdfPageKind::Text,
        (false, _) => PdfPageKind::Scanned,
    }
}
