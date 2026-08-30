use std::path::Path;

use crate::{
    capture::{image_input::SourceFingerprint, DecodedImage},
    documents::DocumentError,
};

#[cfg(windows)]
pub async fn render_page(
    path: &Path,
    page_number: u32,
    dpi: u32,
) -> Result<DecodedImage, DocumentError> {
    use windows::{
        core::HSTRING,
        Data::Pdf::{PdfDocument, PdfPageRenderOptions},
        Storage::{
            StorageFile,
            Streams::{DataReader, InMemoryRandomAccessStream},
        },
    };
    let path_string = path.to_str().ok_or(DocumentError::Io)?.to_owned();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, DocumentError> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path_string))
            .and_then(|v| v.get())
            .map_err(|_| DocumentError::OcrUnavailable)?;
        let pdf = PdfDocument::LoadFromFileAsync(&file)
            .and_then(|v| v.get())
            .map_err(|_| DocumentError::PasswordRequired)?;
        if page_number == 0
            || page_number > pdf.PageCount().map_err(|_| DocumentError::InvalidPackage)?
        {
            return Err(DocumentError::InvalidPackage);
        }
        let page = pdf
            .GetPage(page_number - 1)
            .map_err(|_| DocumentError::InvalidPackage)?;
        let dimensions = page
            .Dimensions()
            .map_err(|_| DocumentError::InvalidPackage)?;
        let scale = (dpi.clamp(72, 300) as f32) / 72.0;
        let crop = dimensions
            .CropBox()
            .map_err(|_| DocumentError::InvalidPackage)?;
        let width = (crop.Width * scale).round().clamp(1.0, 8192.0) as u32;
        let height = (crop.Height * scale).round().clamp(1.0, 8192.0) as u32;
        if u64::from(width) * u64::from(height) > 80_000_000 {
            return Err(DocumentError::LimitExceeded);
        }
        let options = PdfPageRenderOptions::new().map_err(|_| DocumentError::OcrUnavailable)?;
        options
            .SetDestinationWidth(width)
            .map_err(|_| DocumentError::OcrUnavailable)?;
        options
            .SetDestinationHeight(height)
            .map_err(|_| DocumentError::OcrUnavailable)?;
        let stream =
            InMemoryRandomAccessStream::new().map_err(|_| DocumentError::OcrUnavailable)?;
        page.RenderWithOptionsToStreamAsync(&stream, &options)
            .and_then(|v| v.get())
            .map_err(|_| DocumentError::OcrUnavailable)?;
        stream.Seek(0).map_err(|_| DocumentError::OcrUnavailable)?;
        let size = stream.Size().map_err(|_| DocumentError::OcrUnavailable)?;
        if size > 64 * 1024 * 1024 {
            return Err(DocumentError::LimitExceeded);
        }
        let input = stream
            .GetInputStreamAt(0)
            .map_err(|_| DocumentError::OcrUnavailable)?;
        let reader =
            DataReader::CreateDataReader(&input).map_err(|_| DocumentError::OcrUnavailable)?;
        reader
            .LoadAsync(size as u32)
            .and_then(|v| v.get())
            .map_err(|_| DocumentError::OcrUnavailable)?;
        let mut out = vec![0u8; size as usize];
        reader
            .ReadBytes(&mut out)
            .map_err(|_| DocumentError::OcrUnavailable)?;
        Ok(out)
    })
    .await
    .map_err(|_| DocumentError::OcrUnavailable)??;
    decode_rendered(bytes)
}

#[cfg(target_os = "macos")]
pub async fn render_page(
    path: &Path,
    page_number: u32,
    dpi: u32,
) -> Result<DecodedImage, DocumentError> {
    use std::{
        ffi::CString,
        os::raw::{c_char, c_int},
        ptr,
    };
    extern "C" {
        fn smartcat_render_pdf_page(
            path: *const c_char,
            page: u32,
            dpi: u32,
            bytes: *mut *mut u8,
            width: *mut u32,
            height: *mut u32,
            length: *mut usize,
        ) -> c_int;
        fn smartcat_free_pdf_page(bytes: *mut u8);
    }
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let value =
            CString::new(path.to_string_lossy().as_bytes()).map_err(|_| DocumentError::Io)?;
        let mut raw = ptr::null_mut();
        let mut width = 0;
        let mut height = 0;
        let mut length = 0;
        let code = unsafe {
            smartcat_render_pdf_page(
                value.as_ptr(),
                page_number.saturating_sub(1),
                dpi,
                &mut raw,
                &mut width,
                &mut height,
                &mut length,
            )
        };
        if code != 0 || raw.is_null() || length != width as usize * height as usize * 4 {
            return Err(if code == 3 {
                DocumentError::LimitExceeded
            } else {
                DocumentError::OcrUnavailable
            });
        }
        let rgba = unsafe { std::slice::from_raw_parts(raw, length).to_vec() };
        unsafe { smartcat_free_pdf_page(raw) };
        Ok(DecodedImage {
            width,
            height,
            rgba,
            source: SourceFingerprint {
                sha256: String::new(),
                input_bytes: 0,
                original_width: width,
                original_height: height,
                orientation: 1,
                color_type: "RGBA8".into(),
                has_embedded_icc: false,
                format: "pdf-page".into(),
            },
            immutable_copy: std::path::PathBuf::new(),
        })
    })
    .await
    .map_err(|_| DocumentError::OcrUnavailable)?
}

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn render_page(_: &Path, _: u32, _: u32) -> Result<DecodedImage, DocumentError> {
    Err(DocumentError::OcrUnavailable)
}

fn decode_rendered(bytes: Vec<u8>) -> Result<DecodedImage, DocumentError> {
    let image = image::load_from_memory(&bytes)
        .map_err(|_| DocumentError::OcrUnavailable)?
        .to_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 80_000_000 {
        return Err(DocumentError::LimitExceeded);
    }
    Ok(DecodedImage {
        width,
        height,
        rgba: image.into_raw(),
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
        immutable_copy: std::path::PathBuf::new(),
    })
}
