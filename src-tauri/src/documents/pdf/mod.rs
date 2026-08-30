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
        stable_segment_id, DocumentCheckpoint, DocumentError, DocumentFormat, DocumentPlan,
        DocumentStage, PdfRasterSpool, Segment,
    },
};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, fs::OpenOptions, io::BufWriter, path::Path};

pub async fn append_native_ocr(
    plan: &mut DocumentPlan,
    language_hints: &[String],
    force_ocr: bool,
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
    set_spool_private(&spool.root.join("pages"), true);
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
            0,
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
                id: stable_segment_id(
                    &plan.manifest.source_hash,
                    DocumentFormat::Pdf,
                    &format!("page:{}", page.number),
                    &key,
                    ordinal,
                ),
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
        0,
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
    let initial_dpi = if page.has_large_image { 300 } else { 144 };
    let mut image = render_page(source, page.number, initial_dpi).await?;
    if initial_dpi == 144
        && matches!(page.kind, PdfPageKind::Text)
        && rendered_background_is_complex(&image, &page.blocks, page.rotation)
    {
        image = render_page(source, page.number, 300).await?;
    }
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
        .open(&path)
        .map_err(|_| DocumentError::Io)?;
    set_spool_private(&path, false);
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
    writer.get_ref().sync_all().map_err(|_| DocumentError::Io)?;
    drop(writer);
    let page_bytes = fs::metadata(&path).map_err(|_| DocumentError::Io)?.len();
    if page_bytes > MAX_SPOOL_PAGE_BYTES || spool_size(&spool.root)? > MAX_SPOOL_TOTAL_BYTES {
        let _ = fs::remove_file(&path);
        return Err(DocumentError::LimitExceeded);
    }
    Ok(())
}

fn spool_size(root: &Path) -> Result<u64, DocumentError> {
    let pages = root.join("pages");
    if !pages.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in fs::read_dir(pages).map_err(|_| DocumentError::Io)? {
        let entry = entry.map_err(|_| DocumentError::Io)?;
        if entry.file_type().map_err(|_| DocumentError::Io)?.is_file() {
            total = total.saturating_add(entry.metadata().map_err(|_| DocumentError::Io)?.len());
            if total > MAX_SPOOL_TOTAL_BYTES {
                return Ok(total);
            }
        }
    }
    Ok(total)
}

fn rendered_background_is_complex(
    image: &DecodedImage,
    blocks: &[PdfBlock],
    rotation: i32,
) -> bool {
    let mut sampled = 0usize;
    let mut changed = 0usize;
    for block in blocks.iter().take(512) {
        let display = page_bounds_to_display(block.bounds, rotation);
        let left = (display[0].clamp(0.0, 1.0) * image.width as f32) as u32;
        let top = (display[1].clamp(0.0, 1.0) * image.height as f32) as u32;
        let right = ((display[0] + display[2]).clamp(0.0, 1.0) * image.width as f32) as u32;
        let bottom = ((display[1] + display[3]).clamp(0.0, 1.0) * image.height as f32) as u32;
        let mut baseline = None;
        let step_x = ((right.saturating_sub(left)) / 8).max(1);
        let step_y = ((bottom.saturating_sub(top)) / 6).max(1);
        let mut y = top;
        while y < bottom.min(image.height) {
            let mut x = left;
            while x < right.min(image.width) {
                let offset = ((u64::from(y) * u64::from(image.width) + u64::from(x)) * 4) as usize;
                if offset + 2 < image.rgba.len() {
                    let pixel = [
                        image.rgba[offset],
                        image.rgba[offset + 1],
                        image.rgba[offset + 2],
                    ];
                    if let Some(base) = baseline {
                        if pixel.iter().zip(base).any(|(a, b)| a.abs_diff(b) > 32) {
                            changed += 1;
                        }
                    } else {
                        baseline = Some(pixel);
                    }
                    sampled += 1;
                }
                x = x.saturating_add(step_x);
            }
            y = y.saturating_add(step_y);
        }
    }
    sampled == 0 || changed * 5 > sampled
}

pub fn preflight_spool(root: &Path, page_count: usize) -> Result<(), DocumentError> {
    if page_count == 0 || page_count > MAX_PDF_PAGES {
        return Err(DocumentError::LimitExceeded);
    }
    fs::create_dir_all(root).map_err(|_| DocumentError::Io)?;
    let estimated = (page_count as u64)
        .saturating_mul(4 * 1024 * 1024)
        .min(MAX_SPOOL_TOTAL_BYTES);
    if estimated > MAX_SPOOL_TOTAL_BYTES
        || available_space(root).is_some_and(|free| free < estimated + 256 * 1024 * 1024)
    {
        return Err(DocumentError::LimitExceeded);
    }
    Ok(())
}

#[cfg(windows)]
fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    let mut available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(target_os = "macos")]
fn available_space(path: &Path) -> Option<u64> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
        return None;
    }
    let value = unsafe { value.assume_init() };
    Some((value.f_bavail as u64).saturating_mul(value.f_frsize as u64))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn available_space(_: &Path) -> Option<u64> {
    None
}

pub fn emit_checkpoint(
    callback: &(dyn Fn(&DocumentCheckpoint) + Sync),
    source_fingerprint: &str,
    stage: DocumentStage,
    stable_unit_id: String,
    completed: usize,
    total: usize,
    spool: &PdfRasterSpool,
    completed_batch_cursor: usize,
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
        completed_batch_cursor,
        raster_refs,
        translated_result_refs: translated_result_refs.to_vec(),
    });
}

pub async fn validate_rendered_output(
    source: &Path,
    path: &Path,
    inspection: &PdfInspection,
    cancelled: &AtomicBool,
    checkpoint: &(dyn Fn(&DocumentCheckpoint) + Sync),
    source_fingerprint: &str,
    completed_batch_cursor: usize,
    translated_result_refs: &[String],
) -> Result<(), DocumentError> {
    let empty_spool = PdfRasterSpool {
        root: std::path::PathBuf::new(),
        refs: Default::default(),
    };
    for (index, expected) in inspection.pages.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
        let source_image = render_page(source, expected.number, 72).await?;
        let image = render_page(path, expected.number, 72).await?;
        if image.width.abs_diff(source_image.width) > 2
            || image.height.abs_diff(source_image.height) > 2
        {
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
            completed_batch_cursor,
            translated_result_refs,
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

#[cfg(unix)]
fn set_spool_private(path: &Path, directory: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(windows)]
fn set_spool_private(path: &Path, directory: bool) {
    use std::{ffi::OsString, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };
    let text = if directory {
        "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    } else {
        "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)"
    };
    let text: Vec<u16> = OsString::from(text).encode_wide().chain(Some(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return;
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        != 0
        && present != 0
    {
        let mut path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let _ = unsafe {
            SetNamedSecurityInfoW(
                path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        };
    }
    unsafe {
        LocalFree(descriptor);
    }
}

pub const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_PDF_PAGES: usize = 2_000;
pub const MAX_PDF_OBJECTS: usize = 200_000;
pub const MAX_PAGE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PDF_TEXT_CHARS: usize = 4_000_000;
pub const MAX_SPOOL_PAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SPOOL_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
