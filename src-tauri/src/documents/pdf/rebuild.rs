use std::{collections::HashMap, fs, path::Path};

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use image::RgbImage;
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, Stream,
};

use crate::documents::{DocumentError, DocumentWarning, Segment, TranslatedSegment};

use super::{inspect, PdfInspection};

const BUNDLED_FONT: &[u8] =
    include_bytes!("../../../../tests/fixtures/fonts/NotoSans-Variable.ttf");

pub fn rebuild(
    source: &Path,
    inspection: &PdfInspection,
    segments: &[Segment],
    translated: &[TranslatedSegment],
    output: &Path,
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
    for page in &inspection.pages {
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
            let rgb = render_text(text, pixel_w, pixel_h);
            let mut image = Stream::new(
                dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>pixel_w as i64,"Height"=>pixel_h as i64,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},
                rgb,
            );
            image.compress().map_err(|_| DocumentError::Io)?;
            let image_id = doc.add_object(image);
            let name = format!("SC{}", block.ordinal);
            xobjects.set(name.as_bytes().to_vec(), Object::Reference(image_id));
            let x = block.bounds[0] * page.width;
            let h = block.bounds[3] * page.height;
            let w = block.bounds[2] * page.width;
            let y = page.height - block.bounds[1] * page.height - h;
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
        let mut items = match old {
            Some(Object::Array(v)) => v,
            Some(v) => vec![v],
            None => vec![],
        };
        items.push(Object::Reference(stream_id));
        page_obj.set("Contents", Object::Array(items));
    }
    if inspection.has_signatures {
        warnings.push(DocumentWarning{code:"signatureInvalidated".into(),location:None,message:"The source contains a digital signature; the translated copy does not preserve signature validity.".into()});
    }
    if inspection.has_forms {
        warnings.push(DocumentWarning{code:"interactiveFormPreserved".into(),location:None,message:"Interactive form objects are preserved but translated field values are not changed.".into()});
    }
    doc.prune_objects();
    doc.compress();
    doc.save(output).map_err(|_| DocumentError::Io)?;
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
    Ok(warnings)
}

fn render_text(text: &str, width: u32, height: u32) -> Vec<u8> {
    let mut image = RgbImage::from_pixel(width, height, image::Rgb([255, 255, 255]));
    let mut fonts = FontSystem::new();
    fonts.db_mut().load_font_data(BUNDLED_FONT.to_vec());
    fonts.db_mut().load_system_fonts();
    let mut cache = SwashCache::new();
    let size = (height as f32 * 0.62).clamp(8.0, 72.0);
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, size * 1.18));
    buffer.set_size(&mut fonts, Some(width as f32), Some(height as f32));
    buffer.set_text(
        &mut fonts,
        text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut fonts, false);
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
    image.into_raw()
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
    if values.len() != 4 || values.iter().any(|v| !v.is_finite()) {
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
