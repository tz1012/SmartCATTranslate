mod classify;
mod extract;
mod rebuild;
mod render;

pub use classify::{classify_page, PdfPageKind};
pub use extract::{inspect, PdfBlock, PdfInspection, PdfPageInfo};
pub use rebuild::rebuild;
pub use render::render_page;

use crate::{
    capture::{NativeOcrEngine, OcrEngine},
    documents::{DocumentError, DocumentPlan, Segment},
};
use uuid::Uuid;

pub async fn append_native_ocr(
    plan: &mut DocumentPlan,
    language_hints: &[String],
    force_ocr: bool,
) -> Result<(), DocumentError> {
    if plan.format != crate::documents::DocumentFormat::Pdf {
        return Ok(());
    }
    let inspection = inspect(&plan.source, force_ocr)?;
    let engine = NativeOcrEngine::default();
    for page in inspection
        .pages
        .iter()
        .filter(|p| matches!(p.kind, PdfPageKind::Scanned | PdfPageKind::Mixed))
    {
        let image = render_page(&plan.source, page.number, 144).await?;
        let result = engine
            .recognize(&image, language_hints)
            .await
            .map_err(|_| DocumentError::OcrUnavailable)?;
        let base = if matches!(page.kind, PdfPageKind::Mixed) {
            page.blocks.len()
        } else {
            0
        };
        for (index, line) in result.lines.into_iter().enumerate() {
            if line.text.trim().is_empty() {
                continue;
            }
            let ordinal = base + index;
            plan.segments.push(Segment {
                id: Uuid::new_v4(),
                part: format!("page:{}", page.number),
                ordinal,
                location: format!(
                    "page:{}/ocr:{}@{:.6},{:.6},{:.6},{:.6}",
                    page.number,
                    index + 1,
                    line.bounds.x,
                    line.bounds.y,
                    line.bounds.width,
                    line.bounds.height
                ),
                text: line.text,
            });
        }
    }
    plan.manifest.segment_count = plan.segments.len();
    Ok(())
}

pub const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_PDF_PAGES: usize = 2_000;
pub const MAX_PDF_OBJECTS: usize = 200_000;
pub const MAX_PAGE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PDF_TEXT_CHARS: usize = 4_000_000;
