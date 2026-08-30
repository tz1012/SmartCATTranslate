mod classify;
mod extract;
mod rebuild;
mod render;

pub use classify::{classify_page, PdfPageKind};
pub use extract::{
    inspect, ocr_bounds_to_page, page_bounds_to_display, PdfBlock, PdfInspection, PdfPageInfo,
};
pub use rebuild::rebuild;
pub use render::render_page;

use crate::{
    capture::{NativeOcrEngine, OcrEngine},
    documents::{DocumentError, DocumentPlan, Segment},
};
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

pub async fn append_native_ocr(
    plan: &mut DocumentPlan,
    language_hints: &[String],
    force_ocr: bool,
    job_id: Uuid,
    cancelled: &AtomicBool,
    checkpoint: &(dyn Fn(crate::documents::DocumentStage, usize, usize) + Sync),
) -> Result<(), DocumentError> {
    if plan.format != crate::documents::DocumentFormat::Pdf {
        return Ok(());
    }
    let inspection = inspect(&plan.source, force_ocr)?;
    let engine = NativeOcrEngine::default();
    let selected = inspection
        .pages
        .iter()
        .filter(|p| matches!(p.kind, PdfPageKind::Scanned | PdfPageKind::Mixed))
        .collect::<Vec<_>>();
    let total = selected.len();
    for (page_index, page) in selected.into_iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
        checkpoint(crate::documents::DocumentStage::Ocr, page_index, total);
        let image = render_page(
            &plan.source,
            page.number,
            if page.has_large_image { 300 } else { 144 },
        )
        .await?;
        let result = engine
            .recognize(&image, language_hints)
            .await
            .map_err(|_| DocumentError::OcrUnavailable)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
        plan.pdf_rasters.insert(page.number, image);
        let base = if matches!(page.kind, PdfPageKind::Mixed) {
            page.blocks.len()
        } else {
            0
        };
        for (index, line) in result.lines.into_iter().enumerate() {
            if line.text.trim().is_empty() {
                continue;
            }
            let raw = [
                line.bounds.x,
                line.bounds.y,
                line.bounds.width,
                line.bounds.height,
            ];
            let bounds = ocr_bounds_to_page(raw, page.rotation);
            if page
                .blocks
                .iter()
                .any(|native| intersection_over_union(native.bounds, bounds) > 0.55)
            {
                continue;
            }
            let ordinal = base + index;
            let key = format!("page:{}/ocr:{}", page.number, index + 1);
            plan.segments.push(Segment {
                id: Uuid::new_v5(&job_id, key.as_bytes()),
                part: format!("page:{}", page.number),
                ordinal,
                location: format!(
                    "{}@{:.6},{:.6},{:.6},{:.6},{:.3}",
                    key, bounds[0], bounds[1], bounds[2], bounds[3], line.confidence
                ),
                text: line.text,
            });
        }
    }
    checkpoint(crate::documents::DocumentStage::Ocr, total, total);
    plan.manifest.segment_count = plan.segments.len();
    Ok(())
}

fn intersection_over_union(a: [f32; 4], b: [f32; 4]) -> f32 {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    let overlap = (right - left).max(0.) * (bottom - top).max(0.);
    let union = a[2] * a[3] + b[2] * b[3] - overlap;
    if union <= 0. {
        0.
    } else {
        overlap / union
    }
}

pub const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_PDF_PAGES: usize = 2_000;
pub const MAX_PDF_OBJECTS: usize = 200_000;
pub const MAX_PAGE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PDF_TEXT_CHARS: usize = 4_000_000;
