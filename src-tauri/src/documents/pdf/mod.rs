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
    capture::{image_input::SourceFingerprint, DecodedImage, NativeOcrEngine, OcrEngine},
    documents::{
        DocumentCheckpoint, DocumentError, DocumentPlan, DocumentStage, PdfRasterSpool, Segment,
    },
};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, fs::OpenOptions, io::BufWriter, path::Path};
use uuid::Uuid;

pub async fn append_native_ocr(
    plan: &mut DocumentPlan,
    language_hints: &[String],
    force_ocr: bool,
    job_id: Uuid,
    cancelled: &AtomicBool,
    checkpoint: &(dyn Fn(&DocumentCheckpoint) + Sync),
) -> Result<(), DocumentError> {
    if plan.format != crate::documents::DocumentFormat::Pdf {
        return Ok(());
    }
    let inspection = inspect(&plan.source, force_ocr)?;
    let engine = NativeOcrEngine::default();
    let spool = plan.pdf_spool.as_mut().ok_or(DocumentError::Io)?;
    fs::create_dir_all(spool.root.join("pages")).map_err(|_| DocumentError::Io)?;
    let total = inspection.pages.len();
    for (page_index, page) in inspection.pages.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
        emit_checkpoint(
            checkpoint,
            &plan.manifest.source_hash,
            DocumentStage::Ocr,
            format!("page:{}", page.number),
            page_index,
            total,
            spool,
            &[],
        );
        let image = load_or_render_page(&plan.source, page, spool).await?;
        if !matches!(page.kind, PdfPageKind::Scanned | PdfPageKind::Mixed) {
            continue;
        }
        let result = engine
            .recognize(&image, language_hints)
            .await
            .map_err(|_| DocumentError::OcrUnavailable)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
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
    emit_checkpoint(
        checkpoint,
        &plan.manifest.source_hash,
        DocumentStage::Ocr,
        "ocr:completed".into(),
        total,
        total,
        spool,
        &[],
    );
    plan.manifest.segment_count = plan.segments.len();
    Ok(())
}

async fn load_or_render_page(
    source: &Path,
    page: &PdfPageInfo,
    spool: &mut PdfRasterSpool,
) -> Result<DecodedImage, DocumentError> {
    if let Some(relative) = spool.refs.get(&page.number) {
        return load_spooled_page(spool, relative);
    }
    let image = render_page(
        source,
        page.number,
        if page.has_large_image { 300 } else { 144 },
    )
    .await?;
    let relative = format!("pages/page-{:05}.png", page.number);
    write_spooled_page(spool, &relative, &image)?;
    spool.refs.insert(page.number, relative);
    Ok(image)
}

pub fn load_spooled_page(
    spool: &PdfRasterSpool,
    relative: &str,
) -> Result<DecodedImage, DocumentError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(DocumentError::InvalidPackage);
    }
    let path = spool.root.join(relative_path);
    let bytes = fs::read(&path).map_err(|_| DocumentError::Io)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(DocumentError::LimitExceeded);
    }
    let rgba = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|_| DocumentError::InvalidPackage)?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 80_000_000 {
        return Err(DocumentError::LimitExceeded);
    }
    Ok(DecodedImage {
        width,
        height,
        rgba: rgba.into_raw(),
        source: SourceFingerprint {
            sha256: String::new(),
            input_bytes: bytes.len() as u64,
            original_width: width,
            original_height: height,
            orientation: 1,
            color_type: "RGBA8".into(),
            has_embedded_icc: false,
            format: "pdf-page".into(),
        },
        immutable_copy: path,
    })
}

fn write_spooled_page(
    spool: &PdfRasterSpool,
    relative: &str,
    image: &DecodedImage,
) -> Result<(), DocumentError> {
    let path = spool.root.join(relative);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| DocumentError::Io)?;
    let mut writer = BufWriter::new(file);
    PngEncoder::new(&mut writer)
        .write_image(
            &image.rgba,
            image.width,
            image.height,
            ColorType::Rgba8.into(),
        )
        .map_err(|_| DocumentError::Io)?;
    use std::io::Write;
    writer.flush().map_err(|_| DocumentError::Io)?;
    writer.get_ref().sync_all().map_err(|_| DocumentError::Io)
}

pub fn emit_checkpoint(
    callback: &(dyn Fn(&DocumentCheckpoint) + Sync),
    source_fingerprint: &str,
    stage: DocumentStage,
    stable_unit_id: String,
    completed: usize,
    total: usize,
    spool: &PdfRasterSpool,
    translated_result_refs: &[String],
) {
    let mut raster_refs = spool.refs.values().cloned().collect::<Vec<_>>();
    raster_refs.sort();
    callback(&DocumentCheckpoint {
        source_fingerprint: source_fingerprint.to_owned(),
        stage,
        stable_unit_id,
        completed,
        total,
        raster_refs,
        translated_result_refs: translated_result_refs.to_vec(),
    });
}

pub async fn validate_rendered_output(
    path: &Path,
    inspection: &PdfInspection,
    cancelled: &AtomicBool,
    checkpoint: &(dyn Fn(&DocumentCheckpoint) + Sync),
    source_fingerprint: &str,
) -> Result<(), DocumentError> {
    let empty_spool = PdfRasterSpool {
        root: std::path::PathBuf::new(),
        refs: Default::default(),
    };
    for (index, expected) in inspection.pages.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
        let image = render_page(path, expected.number, 72).await?;
        let expected_width = expected.width.round().max(1.0) as u32;
        let expected_height = expected.height.round().max(1.0) as u32;
        let (expected_width, expected_height) = if expected.rotation.rem_euclid(180) == 90 {
            (expected_height, expected_width)
        } else {
            (expected_width, expected_height)
        };
        if image.width.abs_diff(expected_width) > 2 || image.height.abs_diff(expected_height) > 2 {
            return Err(DocumentError::ValidationFailed);
        }
        emit_checkpoint(
            checkpoint,
            source_fingerprint,
            DocumentStage::Validate,
            format!("page:{}", expected.number),
            index + 1,
            inspection.pages.len(),
            &empty_spool,
            &[],
        );
    }
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
