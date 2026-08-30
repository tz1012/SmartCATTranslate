use std::{fs, path::Path};

use lopdf::{content::Content, Dictionary, Document, Object, ObjectId};
use uuid::Uuid;

use crate::documents::{DocumentError, Segment};

use super::{
    classify_page, PdfPageKind, MAX_PAGE_CONTENT_BYTES, MAX_PDF_BYTES, MAX_PDF_OBJECTS,
    MAX_PDF_PAGES, MAX_PDF_TEXT_CHARS,
};

#[derive(Clone, Debug)]
pub struct PdfBlock {
    pub page: u32,
    pub ordinal: usize,
    pub text: String,
    pub bounds: [f32; 4],
    pub font_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PdfPageInfo {
    pub number: u32,
    pub object_id: ObjectId,
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
    pub kind: PdfPageKind,
    pub blocks: Vec<PdfBlock>,
}

#[derive(Clone, Debug)]
pub struct PdfInspection {
    pub pages: Vec<PdfPageInfo>,
    pub segments: Vec<Segment>,
    pub has_signatures: bool,
    pub has_forms: bool,
    pub has_annotations: bool,
}

pub fn inspect(path: &Path, force_ocr: bool) -> Result<PdfInspection, DocumentError> {
    let metadata = fs::metadata(path).map_err(|_| DocumentError::Io)?;
    if metadata.len() > MAX_PDF_BYTES {
        return Err(DocumentError::LimitExceeded);
    }
    let bytes = fs::read(path).map_err(|_| DocumentError::Io)?;
    let doc = Document::load_mem(&bytes).map_err(|_| DocumentError::InvalidPackage)?;
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        return Err(DocumentError::PasswordRequired);
    }
    if doc.objects.len() > MAX_PDF_OBJECTS {
        return Err(DocumentError::LimitExceeded);
    }
    let page_map = doc.get_pages();
    if page_map.is_empty() || page_map.len() > MAX_PDF_PAGES {
        return Err(DocumentError::LimitExceeded);
    }
    let has_forms = catalog_dict(&doc).is_some_and(|d| d.has(b"AcroForm"));
    let has_signatures = doc.objects.values().any(|o| {
        o.as_dict().is_ok_and(|d| {
            d.get(b"FT")
                .is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"Sig"))
        })
    });
    let mut pages = Vec::with_capacity(page_map.len());
    let mut segments = Vec::new();
    let mut total_chars = 0usize;
    let mut has_annotations = false;
    for (number, object_id) in page_map {
        let page_dict = doc
            .get_object(object_id)
            .and_then(Object::as_dict)
            .map_err(|_| DocumentError::InvalidPackage)?;
        has_annotations |= page_dict.has(b"Annots");
        let (width, height) = page_size(&doc, page_dict).unwrap_or((612.0, 792.0));
        let rotation = inherited_i64(&doc, page_dict, b"Rotate").unwrap_or(0) as i32;
        let content = doc
            .get_page_content(object_id)
            .map_err(|_| DocumentError::InvalidPackage)?;
        if content.len() > MAX_PAGE_CONTENT_BYTES {
            return Err(DocumentError::LimitExceeded);
        }
        let mut blocks = extract_blocks(number, &content, width, height, rotation)?;
        total_chars = total_chars
            .saturating_add(blocks.iter().map(|b| b.text.chars().count()).sum::<usize>());
        if total_chars > MAX_PDF_TEXT_CHARS {
            return Err(DocumentError::LimitExceeded);
        }
        let non_ws = blocks
            .iter()
            .flat_map(|b| b.text.chars())
            .filter(|c| !c.is_whitespace())
            .count();
        let area = blocks
            .iter()
            .map(|b| b.bounds[2] * b.bounds[3])
            .sum::<f32>()
            .clamp(0.0, 1.0);
        let has_image = page_has_image(&doc, page_dict);
        let kind = if force_ocr {
            PdfPageKind::Scanned
        } else {
            classify_page(non_ws, area, has_image)
        };
        if matches!(kind, PdfPageKind::Scanned) {
            blocks.clear();
        }
        for block in &blocks {
            segments.push(Segment {
                id: Uuid::new_v4(),
                part: format!("page:{}", block.page),
                ordinal: block.ordinal,
                location: format!("page:{}/block:{}", block.page, block.ordinal + 1),
                text: block.text.clone(),
            });
        }
        pages.push(PdfPageInfo {
            number,
            object_id,
            width,
            height,
            rotation,
            kind,
            blocks,
        });
    }
    Ok(PdfInspection {
        pages,
        segments,
        has_signatures,
        has_forms,
        has_annotations,
    })
}

fn extract_blocks(
    page: u32,
    bytes: &[u8],
    width: f32,
    height: f32,
    rotation: i32,
) -> Result<Vec<PdfBlock>, DocumentError> {
    let content = Content::decode(bytes).map_err(|_| DocumentError::InvalidPackage)?;
    let mut x = 0.0f32;
    let mut y = height;
    let mut font_size = 12.0f32;
    let mut font = None;
    let mut blocks = Vec::new();
    for op in content.operations {
        match op.operator.as_str() {
            "Tf" => {
                font = op
                    .operands
                    .first()
                    .and_then(|v| v.as_name().ok())
                    .map(|v| String::from_utf8_lossy(v).into_owned());
                font_size = number(op.operands.get(1))
                    .unwrap_or(12.0)
                    .abs()
                    .clamp(1.0, 300.0);
            }
            "Tm" => {
                x = number(op.operands.get(4)).unwrap_or(x);
                y = number(op.operands.get(5)).unwrap_or(y);
            }
            "Td" | "TD" => {
                x += number(op.operands.first()).unwrap_or(0.0);
                y += number(op.operands.get(1)).unwrap_or(0.0);
            }
            "T*" => y -= font_size * 1.2,
            "Tj" | "'" | "\"" => {
                if let Some(value) = op.operands.last().and_then(text_object) {
                    push_block(
                        &mut blocks,
                        page,
                        value,
                        x,
                        y,
                        font_size,
                        width,
                        height,
                        rotation,
                        font.clone(),
                    );
                }
            }
            "TJ" => {
                if let Some(Object::Array(items)) = op.operands.first() {
                    let text = items.iter().filter_map(text_object).collect::<String>();
                    push_block(
                        &mut blocks,
                        page,
                        text,
                        x,
                        y,
                        font_size,
                        width,
                        height,
                        rotation,
                        font.clone(),
                    );
                }
            }
            _ => {}
        }
    }
    Ok(blocks)
}

fn push_block(
    blocks: &mut Vec<PdfBlock>,
    page: u32,
    text: String,
    x: f32,
    y: f32,
    size: f32,
    width: f32,
    height: f32,
    rotation: i32,
    font: Option<String>,
) {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return;
    }
    let w = (text.chars().count() as f32 * size * 0.55).clamp(size, width.max(1.0));
    let raw = [
        (x / width).clamp(0.0, 1.0),
        ((height - y - size) / height).clamp(0.0, 1.0),
        (w / width).clamp(0.001, 1.0),
        (size * 1.25 / height).clamp(0.001, 1.0),
    ];
    let bounds = rotate_bounds(raw, rotation);
    blocks.push(PdfBlock {
        page,
        ordinal: blocks.len(),
        text,
        bounds,
        font_hint: font,
    });
}

fn rotate_bounds(b: [f32; 4], rotation: i32) -> [f32; 4] {
    match rotation.rem_euclid(360) {
        90 => [1.0 - b[1] - b[3], b[0], b[3], b[2]],
        180 => [1.0 - b[0] - b[2], 1.0 - b[1] - b[3], b[2], b[3]],
        270 => [b[1], 1.0 - b[0] - b[2], b[3], b[2]],
        _ => b,
    }
}
fn text_object(o: &Object) -> Option<String> {
    let b = o.as_str().ok()?;
    if b.starts_with(&[0xfe, 0xff]) {
        Some(
            b[2..]
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
                .collect(),
        )
    } else {
        Some(String::from_utf8_lossy(b).into_owned())
    }
}
fn number(o: Option<&Object>) -> Option<f32> {
    match o? {
        Object::Integer(v) => Some(*v as f32),
        Object::Real(v) => Some(*v),
        _ => None,
    }
}
fn catalog_dict(doc: &Document) -> Option<&Dictionary> {
    let id = doc.trailer.get(b"Root").ok()?.as_reference().ok()?;
    doc.get_object(id).ok()?.as_dict().ok()
}
fn inherited_i64(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    if let Ok(v) = dict.get(key) {
        return v.as_i64().ok();
    }
    let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
    inherited_i64(doc, doc.get_object(parent).ok()?.as_dict().ok()?, key)
}
fn inherited_object<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    if let Ok(v) = dict.get(key) {
        return Some(v);
    }
    let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
    inherited_object(doc, doc.get_object(parent).ok()?.as_dict().ok()?, key)
}
fn page_size(doc: &Document, dict: &Dictionary) -> Option<(f32, f32)> {
    let a = inherited_object(doc, dict, b"CropBox")
        .or_else(|| inherited_object(doc, dict, b"MediaBox"))?
        .as_array()
        .ok()?;
    if a.len() != 4 {
        return None;
    }
    let x0 = number(a.first())?;
    let y0 = number(a.get(1))?;
    let x1 = number(a.get(2))?;
    let y1 = number(a.get(3))?;
    Some(((x1 - x0).abs(), (y1 - y0).abs()))
}
fn page_has_image(doc: &Document, dict: &Dictionary) -> bool {
    inherited_object(doc, dict, b"Resources")
        .and_then(|r| resolve_dict(doc, r))
        .and_then(|r| r.get(b"XObject").ok())
        .and_then(|x| resolve_dict(doc, x))
        .is_some_and(|x| {
            x.iter().any(|(_, v)| {
                v.as_reference()
                    .ok()
                    .and_then(|id| doc.get_object(id).ok())
                    .and_then(|o| o.as_stream().ok())
                    .is_some_and(|s| {
                        s.dict
                            .get(b"Subtype")
                            .is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"Image"))
                    })
            })
        })
}
fn resolve_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok(),
        _ => None,
    }
}
