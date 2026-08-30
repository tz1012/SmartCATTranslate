use super::render::RenderedImage;
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn export_atomic(
    rendered: &RenderedImage,
    destination: &Path,
    replace: bool,
) -> Result<PathBuf, ExportError> {
    let parent = destination
        .parent()
        .ok_or(ExportError::InvalidDestination)?;
    if !parent.is_dir() {
        return Err(ExportError::InvalidDestination);
    }
    let format = match destination
        .extension()
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => ImageFormat::Png,
        Some("jpg" | "jpeg") => ImageFormat::Jpeg,
        Some("webp") => ImageFormat::WebP,
        _ => return Err(ExportError::UnsupportedFormat),
    };
    let destination = available_path(destination, replace);
    let temporary = parent.join(format!(".smartcat-{}.tmp", Uuid::new_v4()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| ExportError::WriteFailed)?;
    let image = RgbaImage::from_raw(rendered.width, rendered.height, rendered.rgba.clone())
        .ok_or(ExportError::InvalidImage)?;
    let mut writer = BufWriter::new(file);
    DynamicImage::ImageRgba8(image)
        .write_to(&mut writer, format)
        .map_err(|_| ExportError::WriteFailed)?;
    writer.flush().map_err(|_| ExportError::WriteFailed)?;
    let file = writer.into_inner().map_err(|_| ExportError::WriteFailed)?;
    file.sync_all().map_err(|_| ExportError::WriteFailed)?;
    let bytes = std::fs::read(&temporary).map_err(|_| ExportError::WriteFailed)?;
    image::load_from_memory_with_format(&bytes, format)
        .map_err(|_| ExportError::VerificationFailed)?;
    if replace && destination.exists() {
        std::fs::remove_file(&destination).map_err(|_| ExportError::WriteFailed)?;
    }
    std::fs::rename(&temporary, &destination).map_err(|_| ExportError::WriteFailed)?;
    Ok(destination)
}

fn available_path(path: &Path, replace: bool) -> PathBuf {
    if replace || !path.exists() {
        return path.to_owned();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("translation");
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("png");
    for index in 2..10_000 {
        let candidate = parent.join(format!("{stem}_{index}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}_{}.{}", Uuid::new_v4(), ext))
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("invalid export destination")]
    InvalidDestination,
    #[error("unsupported export format")]
    UnsupportedFormat,
    #[error("invalid rendered image")]
    InvalidImage,
    #[error("image export failed")]
    WriteFailed,
    #[error("export verification failed")]
    VerificationFailed,
}
