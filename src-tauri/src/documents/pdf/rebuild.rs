use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use image::RgbImage;
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, Stream,
};

use crate::{
    capture::{render::RenderEngine, DecodedImage, TranslatedBlock},
    documents::{
        stable_segment_id, DocumentCheckpoint, DocumentError, DocumentFormat, DocumentOptions,
        DocumentStage, DocumentWarning, PdfRasterSpool, Segment, TranslatedSegment,
    },
};

use super::{inspect, PdfInspection};

const BUNDLED_FONT: &[u8] =
    include_bytes!("../../../../tests/fixtures/fonts/NotoSans-Variable.ttf");

pub fn rebuild(
    source: &Path,
    inspection: &PdfInspection,
    segments: &[Segment],
    translated: &[TranslatedSegment],
    output: &Path,
    options: &DocumentOptions,
    spool: Option<&PdfRasterSpool>,
    translated_result_refs: &[String],
    completed_batch_cursor: usize,
    cancelled: &AtomicBool,
    checkpoint: &(dyn Fn(&DocumentCheckpoint) -> Result<(), DocumentError> + Sync),
) -> Result<Vec<DocumentWarning>, DocumentError> {
    let bytes = fs::read(source).map_err(|_| DocumentError::Io)?;
    let mut doc = Document::load_mem(&bytes).map_err(|_| DocumentError::InvalidPackage)?;
    let by_id = translated
        .iter()
        .map(|v| (v.id, v.text.as_str()))
        .collect::<HashMap<_, _>>();
    let by_location = segments
        .iter()
        .map(|s| {
            (
                (&*s.part, s.ordinal),
                by_id.get(&s.id).copied().unwrap_or(""),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut warnings = Vec::new();
    for (page_index, page) in inspection.pages.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err(DocumentError::Cancelled);
        }
        let spool = spool.ok_or(DocumentError::Io)?;
        super::emit_checkpoint(
            checkpoint,
            &inspection.source_hash,
            DocumentStage::Reflow,
            format!("page:{}", page.number),
            page_index,
            inspection.pages.len(),
            spool,
            completed_batch_cursor,
            translated_result_refs,
        )?;
        let page_part = format!("page:{}", page.number);
        let mut effective_blocks = page.blocks.clone();
        if matches!(page.kind, super::PdfPageKind::Scanned) {
            effective_blocks = segments
                .iter()
                .filter(|s| s.part == page_part)
                .filter_map(|s| parse_ocr_block(s, page.number))
                .collect();
            if effective_blocks.is_empty() {
                return Err(DocumentError::OcrUnavailable);
            }
        } else if matches!(page.kind, super::PdfPageKind::Mixed) {
            effective_blocks.extend(
                segments
                    .iter()
                    .filter(|s| s.part == page_part && s.location.contains("/ocr:"))
                    .filter_map(|s| parse_ocr_block(s, page.number)),
            );
        }
        let mut resources = cloned_resources(&doc, page.object_id);
        let mut xobjects = resources
            .get(b"XObject")
            .ok()
            .and_then(|o| resolve_dict(&doc, o))
            .cloned()
            .unwrap_or_default();
        let mut operations = Vec::new();
        let relative = spool
            .refs
            .get(&page.number)
            .ok_or(DocumentError::OcrUnavailable)?;
        let source_raster = super::load_spooled_page(spool, relative)?;
        let background = estimate_page_background(&source_raster, &effective_blocks, page.rotation);
        let rasterize = page.has_large_image
            || background.complex
            || matches!(page.kind, super::PdfPageKind::Scanned);
        if rasterize {
            let translated_blocks = effective_blocks
                .iter()
                .filter_map(|block| {
                    let text = by_location.get(&(&*page_part, block.ordinal)).copied()?;
                    let display = super::page_bounds_to_display(block.bounds, page.rotation);
                    let stable_location =
                        format!("page:{}/block:{}", page.number, block.ordinal + 1);
                    Some(TranslatedBlock {
                        id: stable_segment_id(
                            &inspection.source_hash,
                            DocumentFormat::Pdf,
                            &page_part,
                            &stable_location,
                            block.ordinal,
                        ),
                        source_ids: Vec::new(),
                        source_text: String::new(),
                        translated_text: text.to_owned(),
                        bounds: crate::capture::NormalizedRect::new(
                            display[0], display[1], display[2], display[3],
                        )
                        .ok()?,
                        confidence: 1.0,
                        direction: None,
                        visible: true,
                    })
                })
                .collect::<Vec<_>>();
            let rendered = RenderEngine::render(&source_raster, &translated_blocks)
                .map_err(|_| DocumentError::ValidationFailed)?;
            let rgb = rendered
                .rgba
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect::<Vec<_>>();
            let mut image = Stream::new(
                dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>rendered.width as i64,"Height"=>rendered.height as i64,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},
                rgb,
            );
            image.compress().map_err(|_| DocumentError::Io)?;
            let image_id = doc.add_object(image);
            xobjects.set("SCPage", Object::Reference(image_id));
            operations.extend([
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        page.width.into(),
                        0.into(),
                        0.into(),
                        page.height.into(),
                        page.crop_x.into(),
                        page.crop_y.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"SCPage".to_vec())]),
                Operation::new("Q", vec![]),
            ]);
            warnings.push(DocumentWarning{code:"rasterizedPage".into(),location:Some(format!("page:{}",page.number)),message:"Complex or scanned page was rebuilt as a bounded raster while links and annotations were preserved.".into()});
            for block in &effective_blocks {
                if confidence_for(segments, page.number, block.ordinal).is_some_and(|v| v < 0.70) {
                    warnings.push(DocumentWarning {
                        code: "lowConfidenceOcr".into(),
                        location: Some(format!("page:{}/ocr:{}", page.number, block.ordinal + 1)),
                        message: "OCR confidence is low; review this translated block.".into(),
                    });
                }
            }
        } else {
            for block in &effective_blocks {
                let part = format!("page:{}", page.number);
                let Some(text) = by_location.get(&(&*part, block.ordinal)).copied() else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                let pixel_w = (block.bounds[2] * page.width * 2.0)
                    .round()
                    .clamp(8.0, 4096.0) as u32;
                let pixel_h = (block.bounds[3] * page.height * 2.0)
                    .round()
                    .clamp(8.0, 2048.0) as u32;
                let rendered =
                    render_text(text, pixel_w, pixel_h, options.pdf_fit, background.color);
                let mut image = Stream::new(
                    dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>pixel_w as i64,"Height"=>pixel_h as i64,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},
                    rendered.rgb,
                );
                image.compress().map_err(|_| DocumentError::Io)?;
                let image_id = doc.add_object(image);
                let name = format!("SC{}", block.ordinal);
                xobjects.set(name.as_bytes().to_vec(), Object::Reference(image_id));
                let x = page.crop_x + block.bounds[0] * page.width;
                let h = block.bounds[3] * page.height;
                let w = block.bounds[2] * page.width;
                let y = page.crop_y + page.height - block.bounds[1] * page.height - h;
                operations.extend([
                    Operation::new("q", vec![]),
                    Operation::new(
                        "cm",
                        vec![w.into(), 0.into(), 0.into(), h.into(), x.into(), y.into()],
                    ),
                    Operation::new("Do", vec![Object::Name(name.into_bytes())]),
                    Operation::new("Q", vec![]),
                ]);
                warnings.push(DocumentWarning{code:"backgroundApproximation".into(),location:Some(format!("page:{}/block:{}",page.number,block.ordinal+1)),message:"Translation is rendered as a non-destructive raster overlay; review complex backgrounds.".into()});
                if rendered.overflow {
                    warnings.push(DocumentWarning {
                        code: "textOverflow".into(),
                        location: Some(format!("page:{}/block:{}", page.number, block.ordinal + 1)),
                        message: "Translated text could not fit at the minimum readable size."
                            .into(),
                    });
                }
                if block.font_hint.is_some() || text.chars().any(|c| c as u32 > 0x2ff) {
                    warnings.push(DocumentWarning {
                        code: "fontSubstitution".into(),
                        location: Some(format!("page:{}/block:{}", page.number, block.ordinal + 1)),
                        message: "Translation used bundled or operating-system fallback glyphs."
                            .into(),
                    });
                }
                if confidence_for(segments, page.number, block.ordinal).is_some_and(|v| v < 0.70) {
                    warnings.push(DocumentWarning {
                        code: "lowConfidenceOcr".into(),
                        location: Some(format!("page:{}/ocr:{}", page.number, block.ordinal + 1)),
                        message: "OCR confidence is low; review this translated block.".into(),
                    });
                }
            }
        }
        if operations.is_empty() {
            continue;
        }
        resources.set("XObject", Object::Dictionary(xobjects));
        let resources_id = doc.add_object(resources);
        let stream = Stream::new(
            Dictionary::new(),
            Content { operations }
                .encode()
                .map_err(|_| DocumentError::Io)?,
        );
        let stream_id = doc.add_object(stream);
        let page_obj = doc
            .get_object_mut(page.object_id)
            .and_then(Object::as_dict_mut)
            .map_err(|_| DocumentError::InvalidPackage)?;
        page_obj.set("Resources", Object::Reference(resources_id));
        let old = page_obj.get(b"Contents").ok().cloned();
        let mut items = if rasterize {
            Vec::new()
        } else {
            match old {
                Some(Object::Array(v)) => v,
                Some(v) => vec![v],
                None => vec![],
            }
        };
        items.push(Object::Reference(stream_id));
        page_obj.set("Contents", Object::Array(items));
        if !options.preserve_annotations {
            page_obj.remove(b"Annots");
        }
    }
    let spool = spool.ok_or(DocumentError::Io)?;
    super::emit_checkpoint(
        checkpoint,
        &inspection.source_hash,
        DocumentStage::Reflow,
        "reflow:completed".into(),
        inspection.pages.len(),
        inspection.pages.len(),
        spool,
        completed_batch_cursor,
        translated_result_refs,
    )?;
    for page in &inspection.pages {
        if let Some(reason) = &page.fallback_reason {
            warnings.push(DocumentWarning {
                code: "unsupportedEncoding".into(),
                location: Some(format!("page:{}", page.number)),
                message: format!(
                    "Native text extraction was unsafe ({reason}); local raster OCR was used."
                ),
            });
        }
    }
    if inspection.has_signatures {
        warnings.push(DocumentWarning{code:"signatureInvalidated".into(),location:None,message:"The source contains a digital signature; the translated copy does not preserve signature validity.".into()});
    }
    if inspection.has_forms {
        warnings.push(DocumentWarning{code:"interactiveFormPreserved".into(),location:None,message:"Interactive form objects are preserved but translated field values are not changed.".into()});
    }
    if !options.preserve_annotations {
        if let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) {
            if let Ok(catalog) = doc.get_object_mut(root).and_then(Object::as_dict_mut) {
                catalog.remove(b"AcroForm");
            }
        }
    }
    doc.prune_objects();
    doc.compress();
    if cancelled.load(Ordering::Acquire) {
        return Err(DocumentError::Cancelled);
    }
    super::emit_checkpoint(
        checkpoint,
        &inspection.source_hash,
        DocumentStage::Reflow,
        "reflow:serialize".into(),
        0,
        1,
        spool,
        completed_batch_cursor,
        translated_result_refs,
    )?;
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .map_err(|_| DocumentError::OutputExists)?;
    let mut writer = BufWriter::new(file);
    doc.save_to(&mut writer).map_err(|_| DocumentError::Io)?;
    writer.flush().map_err(|_| DocumentError::Io)?;
    writer.get_ref().sync_all().map_err(|_| DocumentError::Io)?;
    drop(writer);
    super::emit_checkpoint(
        checkpoint,
        &inspection.source_hash,
        DocumentStage::Validate,
        "validate:package".into(),
        1,
        1,
        spool,
        completed_batch_cursor,
        translated_result_refs,
    )?;
    let reopened = inspect(output, false)?;
    if reopened.pages.len() != inspection.pages.len() {
        return Err(DocumentError::ValidationFailed);
    }
    for (a, b) in reopened.pages.iter().zip(&inspection.pages) {
        if (a.width - b.width).abs() > 0.1
            || (a.height - b.height).abs() > 0.1
            || a.rotation != b.rotation
        {
            return Err(DocumentError::ValidationFailed);
        }
    }
    if options.preserve_annotations
        && (reopened.has_annotations != inspection.has_annotations
            || reopened.has_forms != inspection.has_forms
            || reopened.attachment_count != inspection.attachment_count)
    {
        return Err(DocumentError::ValidationFailed);
    }
    super::emit_checkpoint(
        checkpoint,
        &inspection.source_hash,
        DocumentStage::Validate,
        "validate:completed".into(),
        1,
        1,
        spool,
        completed_batch_cursor,
        translated_result_refs,
    )?;
    Ok(warnings)
}

struct RenderedText {
    rgb: Vec<u8>,
    overflow: bool,
}
fn render_text(
    text: &str,
    width: u32,
    height: u32,
    fit: bool,
    background: [u8; 3],
) -> RenderedText {
    let mut image = RgbImage::from_pixel(width, height, image::Rgb(background));
    let mut fonts = FontSystem::new();
    fonts.db_mut().load_font_data(BUNDLED_FONT.to_vec());
    fonts.db_mut().load_system_fonts();
    let mut cache = SwashCache::new();
    let mut size = (height as f32 * 0.62).clamp(8., 72.);
    if fit {
        let mut low = 8.;
        let mut high = size.max(8.);
        for _ in 0..6 {
            let candidate = (low + high) / 2.;
            let mut probe = Buffer::new(&mut fonts, Metrics::new(candidate, candidate * 1.18));
            probe.set_size(&mut fonts, Some(width as f32), Some(height as f32));
            probe.set_text(
                &mut fonts,
                text,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
            );
            probe.shape_until_scroll(&mut fonts, false);
            if probe
                .layout_runs()
                .all(|r| r.line_top + r.line_height <= height as f32 + 0.5)
            {
                size = candidate;
                low = candidate
            } else {
                high = candidate
            }
        }
    }
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, size * 1.18));
    buffer.set_size(&mut fonts, Some(width as f32), Some(height as f32));
    buffer.set_text(
        &mut fonts,
        text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut fonts, false);
    let overflow = buffer
        .layout_runs()
        .any(|r| r.line_top + r.line_height > height as f32 + 0.5);
    buffer.draw(
        &mut fonts,
        &mut cache,
        Color::rgb(20, 25, 32),
        |x, y, w, h, c| {
            for py in 0..h {
                for px in 0..w {
                    let tx = x + px as i32;
                    let ty = y + py as i32;
                    if tx >= 0 && ty >= 0 && (tx as u32) < width && (ty as u32) < height {
                        let p = image.get_pixel_mut(tx as u32, ty as u32);
                        let a = c.a() as f32 / 255.0;
                        let (color, _, _, _) = c.as_rgba_tuple();
                        for ch in 0..3 {
                            p[ch] = (color as f32 * a + p[ch] as f32 * (1.0 - a)) as u8
                        }
                    }
                }
            }
        },
    );
    RenderedText {
        rgb: image.into_raw(),
        overflow,
    }
}
fn cloned_resources(doc: &Document, page_id: (u32, u16)) -> Dictionary {
    let Ok(page) = doc.get_object(page_id).and_then(Object::as_dict) else {
        return Dictionary::new();
    };
    let Some(o) = inherited(doc, page, b"Resources") else {
        return Dictionary::new();
    };
    resolve_dict(doc, o).cloned().unwrap_or_default()
}
fn inherited<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    if let Ok(v) = d.get(key) {
        return Some(v);
    }
    let id = d.get(b"Parent").ok()?.as_reference().ok()?;
    inherited(doc, doc.get_object(id).ok()?.as_dict().ok()?, key)
}
fn resolve_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok(),
        _ => None,
    }
}
fn parse_ocr_block(segment: &Segment, page: u32) -> Option<super::PdfBlock> {
    let encoded = segment.location.split('@').nth(1)?;
    let values = encoded
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 5
        || values.iter().any(|v| !v.is_finite())
        || !(0.0..=1.0).contains(&values[4])
    {
        return None;
    }
    Some(super::PdfBlock {
        page,
        ordinal: segment.ordinal,
        text: segment.text.clone(),
        bounds: [values[0], values[1], values[2], values[3]],
        font_hint: None,
    })
}

struct BackgroundEstimate {
    color: [u8; 3],
    complex: bool,
}

fn estimate_page_background(
    image: &DecodedImage,
    blocks: &[super::PdfBlock],
    rotation: i32,
) -> BackgroundEstimate {
    let mut samples = Vec::new();
    for block in blocks.iter().take(512) {
        let display = super::page_bounds_to_display(block.bounds, rotation);
        let left = (display[0].clamp(0.0, 1.0) * image.width as f32) as u32;
        let top = (display[1].clamp(0.0, 1.0) * image.height as f32) as u32;
        let right = ((display[0] + display[2]).clamp(0.0, 1.0) * image.width as f32) as u32;
        let bottom = ((display[1] + display[3]).clamp(0.0, 1.0) * image.height as f32) as u32;
        let step_x = ((right.saturating_sub(left)) / 12).max(1);
        let step_y = ((bottom.saturating_sub(top)) / 8).max(1);
        let mut y = top;
        while y < bottom.min(image.height) {
            let mut x = left;
            while x < right.min(image.width) {
                let offset = (u64::from(y) * u64::from(image.width) + u64::from(x)) * 4;
                let offset = offset as usize;
                if offset + 2 < image.rgba.len() {
                    samples.push([
                        image.rgba[offset],
                        image.rgba[offset + 1],
                        image.rgba[offset + 2],
                    ]);
                }
                x = x.saturating_add(step_x);
            }
            y = y.saturating_add(step_y);
        }
    }
    if samples.is_empty() {
        return BackgroundEstimate {
            color: [255, 255, 255],
            complex: true,
        };
    }
    let mut channels = [Vec::new(), Vec::new(), Vec::new()];
    for sample in &samples {
        for channel in 0..3 {
            channels[channel].push(sample[channel]);
        }
    }
    for values in &mut channels {
        values.sort_unstable();
    }
    let color = [
        channels[0][channels[0].len() / 2],
        channels[1][channels[1].len() / 2],
        channels[2][channels[2].len() / 2],
    ];
    let differing = samples
        .iter()
        .filter(|sample| {
            sample
                .iter()
                .zip(color)
                .any(|(value, median)| value.abs_diff(median) > 32)
        })
        .count();
    BackgroundEstimate {
        color,
        complex: differing * 5 > samples.len(),
    }
}
fn confidence_for(segments: &[Segment], page: u32, ordinal: usize) -> Option<f32> {
    let part = format!("page:{page}");
    let s = segments
        .iter()
        .find(|s| s.part == part && s.ordinal == ordinal && s.location.contains("/ocr:"))?;
    s.location
        .split('@')
        .nth(1)?
        .split(',')
        .nth(4)?
        .parse()
        .ok()
}
