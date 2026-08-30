use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MAX_IMAGE_FILES: usize = 32;

#[derive(Clone, Copy, Debug)]
pub struct ImageLimits {
    pub max_input_bytes: u64,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_files: usize,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 50 * 1024 * 1024,
            max_pixels: 80_000_000,
            max_decoded_bytes: 200 * 1024 * 1024,
            max_files: MAX_IMAGE_FILES,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceFingerprint {
    pub sha256: String,
    pub input_bytes: u64,
    pub original_width: u32,
    pub original_height: u32,
    pub orientation: u16,
    pub color_type: String,
    pub has_embedded_icc: bool,
    pub format: String,
}

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub source: SourceFingerprint,
    pub immutable_copy: PathBuf,
}

impl std::fmt::Debug for DecodedImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_bytes", &self.rgba.len())
            .field("source", &self.source)
            .field("immutable_copy", &"redacted")
            .finish()
    }
}

pub struct ImageInput;

impl ImageInput {
    pub fn open_read_only(
        source: impl AsRef<Path>,
        immutable_root: impl AsRef<Path>,
    ) -> Result<DecodedImage, ImageInputError> {
        Self::open_with_limits(source, immutable_root, ImageLimits::default())
    }

    pub fn open_many_read_only(
        sources: &[PathBuf],
        immutable_root: impl AsRef<Path>,
    ) -> Result<Vec<DecodedImage>, ImageInputError> {
        let limits = ImageLimits::default();
        if sources.is_empty() || sources.len() > limits.max_files {
            return Err(ImageInputError::FileCountExceeded);
        }
        sources
            .iter()
            .map(|source| Self::open_with_limits(source, immutable_root.as_ref(), limits))
            .collect()
    }

    pub fn open_with_limits(
        source: impl AsRef<Path>,
        immutable_root: impl AsRef<Path>,
        limits: ImageLimits,
    ) -> Result<DecodedImage, ImageInputError> {
        let source = source.as_ref();
        let mut file = OpenOptions::new().read(true).open(source)?;
        let file_len = file.metadata()?.len();
        if file_len == 0 {
            return Err(ImageInputError::CorruptImage);
        }
        if file_len > limits.max_input_bytes {
            return Err(ImageInputError::InputLimitExceeded);
        }
        let mut bytes = Vec::with_capacity(file_len as usize);
        std::io::Read::by_ref(&mut file)
            .take(limits.max_input_bytes + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limits.max_input_bytes {
            return Err(ImageInputError::InputLimitExceeded);
        }
        let format = format_from_magic(&bytes).ok_or(ImageInputError::UnsupportedFormat)?;
        let reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        let (original_width, original_height) = reader
            .into_dimensions()
            .map_err(|_| ImageInputError::CorruptImage)?;
        enforce_dimensions(original_width, original_height, limits)?;

        let color_type = image::ImageReader::with_format(Cursor::new(&bytes), format)
            .decode()
            .map_err(|_| ImageInputError::CorruptImage)?;
        let color_name = format!("{:?}", color_type.color());
        let orientation = exif_orientation(&bytes, format).unwrap_or(1);
        let image = apply_orientation(color_type, orientation);
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        enforce_dimensions(width, height, limits)?;

        let root = immutable_root.as_ref();
        fs::create_dir_all(root)?;
        let extension = format_extension(format);
        let immutable_copy = root.join(format!("{}.{}", Uuid::new_v4(), extension));
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&immutable_copy)?;
        output.write_all(&bytes)?;
        output.sync_all()?;

        let source = SourceFingerprint {
            sha256: hex_sha256(&bytes),
            input_bytes: bytes.len() as u64,
            original_width,
            original_height,
            orientation,
            color_type: color_name,
            has_embedded_icc: contains_icc_profile(&bytes, format),
            format: format_extension(format).to_owned(),
        };
        Ok(DecodedImage {
            width,
            height,
            rgba: rgba.into_raw(),
            source,
            immutable_copy,
        })
    }
}

fn enforce_dimensions(width: u32, height: u32, limits: ImageLimits) -> Result<(), ImageInputError> {
    if width == 0 || height == 0 {
        return Err(ImageInputError::CorruptImage);
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ImageInputError::PixelLimitExceeded)?;
    if pixels > limits.max_pixels {
        return Err(ImageInputError::PixelLimitExceeded);
    }
    if pixels.saturating_mul(4) > limits.max_decoded_bytes {
        return Err(ImageInputError::DecodedMemoryLimitExceeded);
    }
    Ok(())
}

fn format_from_magic(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some(ImageFormat::Tiff)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

fn format_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::WebP => "webp",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Bmp => "bmp",
        _ => "image",
    }
}

fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}

fn contains_icc_profile(bytes: &[u8], format: ImageFormat) -> bool {
    match format {
        ImageFormat::Jpeg => bytes.windows(11).any(|window| window == b"ICC_PROFILE"),
        ImageFormat::Png => bytes.windows(4).any(|window| window == b"iCCP"),
        ImageFormat::WebP => bytes.windows(4).any(|window| window == b"ICCP"),
        ImageFormat::Tiff => bytes
            .windows(2)
            .any(|window| window == [0x87, 0x73] || window == [0x73, 0x87]),
        _ => false,
    }
}

fn exif_orientation(bytes: &[u8], format: ImageFormat) -> Option<u16> {
    match format {
        ImageFormat::Jpeg => jpeg_exif_tiff(bytes).and_then(parse_tiff_orientation),
        ImageFormat::Tiff => parse_tiff_orientation(bytes),
        _ => None,
    }
}

fn jpeg_exif_tiff(bytes: &[u8]) -> Option<&[u8]> {
    let mut offset = 2usize;
    while offset.checked_add(4)? <= bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let length = u16::from_be_bytes([*bytes.get(offset)?, *bytes.get(offset + 1)?]) as usize;
        if length < 2 || offset.checked_add(length)? > bytes.len() {
            return None;
        }
        let payload = &bytes[offset + 2..offset + length];
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            return Some(&payload[6..]);
        }
        offset += length;
    }
    None
}

fn parse_tiff_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let read_u16 = |slice: &[u8]| -> Option<u16> {
        let value: [u8; 2] = slice.get(..2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(value)
        } else {
            u16::from_be_bytes(value)
        })
    };
    let read_u32 = |slice: &[u8]| -> Option<u32> {
        let value: [u8; 4] = slice.get(..4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        })
    };
    let ifd = read_u32(tiff.get(4..)?)? as usize;
    let count = read_u16(tiff.get(ifd..)?)? as usize;
    if count > 1024 {
        return None;
    }
    for index in 0..count {
        let entry = tiff.get(ifd + 2 + index * 12..)?;
        if read_u16(entry)? == 0x0112 && read_u16(&entry[2..])? == 3 && read_u32(&entry[4..])? == 1
        {
            return read_u16(&entry[8..]).filter(|value| (1..=8).contains(value));
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
pub enum ImageInputError {
    #[error("image input or output could not be read")]
    Io(#[from] std::io::Error),
    #[error("unsupported image format")]
    UnsupportedFormat,
    #[error("corrupt image")]
    CorruptImage,
    #[error("image input exceeded the file size limit")]
    InputLimitExceeded,
    #[error("image dimensions exceeded the pixel limit")]
    PixelLimitExceeded,
    #[error("decoded image exceeded the memory limit")]
    DecodedMemoryLimitExceeded,
    #[error("too many image files")]
    FileCountExceeded,
}

impl ImageInputError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "image_io_failed",
            Self::UnsupportedFormat => "unsupported_image_format",
            Self::CorruptImage => "corrupt_image",
            Self::InputLimitExceeded => "image_file_too_large",
            Self::PixelLimitExceeded => "image_dimensions_too_large",
            Self::DecodedMemoryLimitExceeded => "image_memory_limit_exceeded",
            Self::FileCountExceeded => "too_many_images",
        }
    }
}
