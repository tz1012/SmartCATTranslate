use async_trait::async_trait;
use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine as WinOcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
};

use crate::capture::{
    ocr::{normalize_lines, NativeOcrLine},
    DecodedImage, OcrDocument, OcrEngine, OcrError, TextDirection,
};

#[derive(Default)]
pub struct WindowsMediaOcr;

#[async_trait]
impl OcrEngine for WindowsMediaOcr {
    async fn recognize(
        &self,
        image: &DecodedImage,
        hints: &[String],
    ) -> Result<OcrDocument, OcrError> {
        let image = image.clone();
        let hints = hints.iter().take(8).cloned().collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || recognize_blocking(&image, &hints))
            .await
            .map_err(|_| OcrError::NativeFailure)?
    }
}

fn recognize_blocking(image: &DecodedImage, hints: &[String]) -> Result<OcrDocument, OcrError> {
    let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    if let Err(error) = initialized {
        if error.code().0 != 0x80010106u32 as i32 {
            return Err(OcrError::NativeFailure);
        }
    }
    let engine = hints
        .iter()
        .filter_map(|tag| Language::CreateLanguage(&tag.into()).ok())
        .find_map(|language| WinOcrEngine::TryCreateFromLanguage(&language).ok())
        .or_else(|| WinOcrEngine::TryCreateFromUserProfileLanguages().ok())
        .ok_or_else(|| OcrError::LanguagePackMissing {
            requested: hints.to_vec(),
        })?;

    let bitmap = SoftwareBitmap::CreateWithAlpha(
        BitmapPixelFormat::Bgra8,
        image.width as i32,
        image.height as i32,
        BitmapAlphaMode::Premultiplied,
    )
    .map_err(|_| OcrError::NativeFailure)?;
    let mut bgra = image.rgba.clone();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(|_| OcrError::NativeFailure)?;
    writer
        .WriteBytes(&bgra)
        .map_err(|_| OcrError::NativeFailure)?;
    let buffer = writer.DetachBuffer().map_err(|_| OcrError::NativeFailure)?;
    bitmap
        .CopyFromBuffer(&buffer)
        .map_err(|_| OcrError::NativeFailure)?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|op| op.get())
        .map_err(|_| OcrError::NativeFailure)?;
    let angle = result
        .TextAngle()
        .ok()
        .and_then(|value| value.Value().ok())
        .unwrap_or(0.0) as f32;
    let collection = result.Lines().map_err(|_| OcrError::NativeFailure)?;
    let mut lines = Vec::new();
    for index in 0..collection.Size().map_err(|_| OcrError::NativeFailure)? {
        let line = collection
            .GetAt(index)
            .map_err(|_| OcrError::NativeFailure)?;
        let words = line.Words().map_err(|_| OcrError::NativeFailure)?;
        let mut left = f32::MAX;
        let mut top = f32::MAX;
        let mut right = 0f32;
        let mut bottom = 0f32;
        for word_index in 0..words.Size().map_err(|_| OcrError::NativeFailure)? {
            let rect = words
                .GetAt(word_index)
                .and_then(|word| word.BoundingRect())
                .map_err(|_| OcrError::NativeFailure)?;
            left = left.min(rect.X);
            top = top.min(rect.Y);
            right = right.max(rect.X + rect.Width);
            bottom = bottom.max(rect.Y + rect.Height);
        }
        if left == f32::MAX || right <= left || bottom <= top {
            continue;
        }
        lines.push(NativeOcrLine {
            text: line
                .Text()
                .map_err(|_| OcrError::NativeFailure)?
                .to_string(),
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
            confidence: 1.0,
            angle_degrees: angle,
            direction: TextDirection::LeftToRight,
            polygon: vec![(left, top), (right, top), (right, bottom), (left, bottom)],
            language: hints.first().cloned(),
        });
    }
    normalize_lines(image.width, image.height, hints.first().cloned(), lines)
}
