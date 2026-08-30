use std::{collections::BTreeMap, fs, path::Path};

use crate::documents::{stable_segment_id, DocumentError, DocumentFormat, Segment};
use lopdf::{content::Content, Dictionary, Document, Object, ObjectId};
use sha2::{Digest, Sha256};

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
    pub crop_x: f32,
    pub crop_y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
    pub kind: PdfPageKind,
    pub blocks: Vec<PdfBlock>,
    pub fallback_reason: Option<String>,
    pub has_large_image: bool,
}
#[derive(Clone, Debug)]
pub struct PdfInspection {
    pub source_hash: String,
    pub pages: Vec<PdfPageInfo>,
    pub segments: Vec<Segment>,
    pub has_signatures: bool,
    pub has_forms: bool,
    pub has_annotations: bool,
    pub attachment_count: usize,
}

#[derive(Clone, Copy)]
struct Matrix([f32; 6]);
impl Matrix {
    const ID: Self = Self([1., 0., 0., 1., 0., 0.]);
    fn then(self, r: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = r.0;
        Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }
    fn point(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
    fn area(self) -> f32 {
        let [a, b, c, d, _, _] = self.0;
        (a * d - b * c).abs()
    }
}
#[derive(Clone)]
struct TextState {
    ctm: Matrix,
    tm: Matrix,
    tlm: Matrix,
    font: Vec<u8>,
    size: f32,
    leading: f32,
    char_space: f32,
    word_space: f32,
    scale: f32,
    rise: f32,
}
impl Default for TextState {
    fn default() -> Self {
        Self {
            ctm: Matrix::ID,
            tm: Matrix::ID,
            tlm: Matrix::ID,
            font: Vec::new(),
            size: 12.,
            leading: 14.4,
            char_space: 0.,
            word_space: 0.,
            scale: 1.,
            rise: 0.,
        }
    }
}
struct Extracted {
    blocks: Vec<PdfBlock>,
    large_image: bool,
    unsafe_reason: Option<String>,
}

pub fn inspect(path: &Path, force_ocr: bool) -> Result<PdfInspection, DocumentError> {
    let metadata = fs::metadata(path).map_err(|_| DocumentError::Io)?;
    if metadata.len() > MAX_PDF_BYTES {
        return Err(DocumentError::LimitExceeded);
    }
    let bytes = fs::read(path).map_err(|_| DocumentError::Io)?;
    let digest = Sha256::digest(&bytes);
    let source_hash = format!("{digest:x}");
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
    let attachment_count = catalog_dict(&doc)
        .and_then(|d| d.get(b"Names").ok())
        .and_then(|v| resolve_dict(&doc, v))
        .and_then(|d| d.get(b"EmbeddedFiles").ok())
        .map(|value| name_tree_count(&doc, value, 0))
        .unwrap_or(0);
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
        let crop = page_crop(&doc, page_dict).unwrap_or([0., 0., 612., 792.]);
        let rotation = inherited_i64(&doc, page_dict, b"Rotate").unwrap_or(0) as i32;
        let content = doc
            .get_page_content(object_id)
            .map_err(|_| DocumentError::InvalidPackage)?;
        if content.len() > MAX_PAGE_CONTENT_BYTES {
            return Err(DocumentError::LimitExceeded);
        }
        let mut extracted = extract_blocks(&doc, number, object_id, &content, crop)?;
        total_chars = total_chars.saturating_add(
            extracted
                .blocks
                .iter()
                .map(|b| b.text.chars().count())
                .sum::<usize>(),
        );
        if total_chars > MAX_PDF_TEXT_CHARS {
            return Err(DocumentError::LimitExceeded);
        }
        let non_ws = extracted
            .blocks
            .iter()
            .flat_map(|b| b.text.chars())
            .filter(|c| !c.is_whitespace())
            .count();
        let area = extracted
            .blocks
            .iter()
            .map(|b| b.bounds[2] * b.bounds[3])
            .sum::<f32>()
            .clamp(0., 1.);
        let mut kind = classify_page(non_ws, area, extracted.large_image);
        if force_ocr || extracted.unsafe_reason.is_some() {
            kind = PdfPageKind::Scanned;
            extracted.blocks.clear()
        }
        for block in &extracted.blocks {
            let key = format!("page:{number}/block:{}", block.ordinal + 1);
            segments.push(Segment {
                id: stable_segment_id(
                    &source_hash,
                    DocumentFormat::Pdf,
                    &format!("page:{number}"),
                    &key,
                    block.ordinal,
                ),
                part: format!("page:{number}"),
                ordinal: block.ordinal,
                location: format!(
                    "{key}@{:.6},{:.6},{:.6},{:.6}",
                    block.bounds[0], block.bounds[1], block.bounds[2], block.bounds[3]
                ),
                text: block.text.clone(),
            });
        }
        pages.push(PdfPageInfo {
            number,
            object_id,
            crop_x: crop[0],
            crop_y: crop[1],
            width: crop[2],
            height: crop[3],
            rotation,
            kind,
            blocks: extracted.blocks,
            fallback_reason: extracted.unsafe_reason,
            has_large_image: extracted.large_image,
        });
    }
    Ok(PdfInspection {
        source_hash,
        pages,
        segments,
        has_signatures,
        has_forms,
        has_annotations,
        attachment_count,
    })
}

fn name_tree_count(doc: &Document, value: &Object, depth: usize) -> usize {
    if depth > 32 {
        return 0;
    }
    let Some(dictionary) = resolve_dict(doc, value) else {
        return 0;
    };
    let direct = dictionary
        .get(b"Names")
        .ok()
        .and_then(|value| value.as_array().ok())
        .map(|values| values.len() / 2)
        .unwrap_or(0);
    let nested = dictionary
        .get(b"Kids")
        .ok()
        .and_then(|value| value.as_array().ok())
        .map(|kids| {
            kids.iter()
                .map(|kid| name_tree_count(doc, kid, depth + 1))
                .sum()
        })
        .unwrap_or(0);
    direct.saturating_add(nested)
}

fn extract_blocks(
    doc: &Document,
    page: u32,
    page_id: ObjectId,
    bytes: &[u8],
    crop: [f32; 4],
) -> Result<Extracted, DocumentError> {
    let content = Content::decode(bytes).map_err(|_| DocumentError::InvalidPackage)?;
    let fonts = doc.get_page_fonts(page_id).unwrap_or_default();
    let xobjects = page_xobjects(doc, page_id);
    let mut state = TextState::default();
    let mut stack = Vec::new();
    let mut blocks = Vec::new();
    let mut large = false;
    let mut unsafe_reason = None;
    for op in content.operations {
        match op.operator.as_str() {
            "q" => stack.push(state.clone()),
            "Q" => {
                if let Some(v) = stack.pop() {
                    state = v
                }
            }
            "cm" => {
                if let Some(m) = matrix(&op.operands) {
                    state.ctm = state.ctm.then(m)
                }
            }
            "BT" => {
                state.tm = Matrix::ID;
                state.tlm = Matrix::ID
            }
            "Tf" => {
                state.font = op
                    .operands
                    .first()
                    .and_then(|v| v.as_name().ok())
                    .unwrap_or_default()
                    .to_vec();
                state.size = number(op.operands.get(1))
                    .unwrap_or(12.)
                    .abs()
                    .clamp(1., 300.);
                if vertical_font(&fonts, &state.font) {
                    unsafe_reason = Some("verticalText".into())
                }
            }
            "Tm" => {
                if let Some(m) = matrix(&op.operands) {
                    state.tm = m;
                    state.tlm = m
                }
            }
            "Td" | "TD" => {
                let tx = number(op.operands.first()).unwrap_or(0.);
                let ty = number(op.operands.get(1)).unwrap_or(0.);
                state.tlm = state.tlm.then(Matrix([1., 0., 0., 1., tx, ty]));
                state.tm = state.tlm;
                if op.operator == "TD" {
                    state.leading = -ty
                }
            }
            "T*" => {
                state.tlm = state.tlm.then(Matrix([1., 0., 0., 1., 0., -state.leading]));
                state.tm = state.tlm
            }
            "Tc" => state.char_space = number(op.operands.first()).unwrap_or(0.),
            "Tw" => state.word_space = number(op.operands.first()).unwrap_or(0.),
            "Tz" => state.scale = number(op.operands.first()).unwrap_or(100.) / 100.,
            "TL" => state.leading = number(op.operands.first()).unwrap_or(state.leading),
            "Ts" => state.rise = number(op.operands.first()).unwrap_or(0.),
            "Tj" => show(
                doc,
                &fonts,
                page,
                &mut state,
                op.operands.first(),
                crop,
                &mut blocks,
                &mut unsafe_reason,
            ),
            "TJ" => {
                if let Some(Object::Array(items)) = op.operands.first() {
                    for item in items {
                        match item {
                            Object::String(_, _) => show(
                                doc,
                                &fonts,
                                page,
                                &mut state,
                                Some(item),
                                crop,
                                &mut blocks,
                                &mut unsafe_reason,
                            ),
                            Object::Integer(v) => {
                                let delta = -(*v as f32) / 1000. * state.size * state.scale;
                                advance(&mut state, delta)
                            }
                            Object::Real(v) => {
                                let delta = -*v / 1000. * state.size * state.scale;
                                advance(&mut state, delta)
                            }
                            _ => {}
                        }
                    }
                }
            }
            "'" => {
                state.tlm = state.tlm.then(Matrix([1., 0., 0., 1., 0., -state.leading]));
                state.tm = state.tlm;
                show(
                    doc,
                    &fonts,
                    page,
                    &mut state,
                    op.operands.first(),
                    crop,
                    &mut blocks,
                    &mut unsafe_reason,
                )
            }
            "\"" => {
                state.word_space = number(op.operands.first()).unwrap_or(state.word_space);
                state.char_space = number(op.operands.get(1)).unwrap_or(state.char_space);
                state.tlm = state.tlm.then(Matrix([1., 0., 0., 1., 0., -state.leading]));
                state.tm = state.tlm;
                show(
                    doc,
                    &fonts,
                    page,
                    &mut state,
                    op.operands.get(2),
                    crop,
                    &mut blocks,
                    &mut unsafe_reason,
                )
            }
            "Do" => {
                if let Some(name) = op.operands.first().and_then(|v| v.as_name().ok()) {
                    if let Some(dict) = xobjects.get(name) {
                        let subtype = dict
                            .get(b"Subtype")
                            .ok()
                            .and_then(|v| v.as_name().ok())
                            .unwrap_or_default();
                        if subtype == b"Image"
                            && state.ctm.area() / (crop[2] * crop[3]).max(1.) >= 0.20
                        {
                            large = true
                        } else if subtype == b"Form" {
                            unsafe_reason = Some("formXObjectText".into())
                        }
                    }
                }
            }
            "W" | "W*" => unsafe_reason = Some("clippedText".into()),
            _ => {}
        }
    }
    Ok(Extracted {
        blocks,
        large_image: large,
        unsafe_reason,
    })
}

fn show(
    doc: &Document,
    fonts: &BTreeMap<Vec<u8>, &Dictionary>,
    page: u32,
    state: &mut TextState,
    obj: Option<&Object>,
    crop: [f32; 4],
    blocks: &mut Vec<PdfBlock>,
    unsafe_reason: &mut Option<String>,
) {
    let Some(Object::String(bytes, _)) = obj else {
        return;
    };
    let Some(font) = fonts.get(&state.font) else {
        *unsafe_reason = Some("missingFont".into());
        return;
    };
    let Ok(encoding) = font.get_font_encoding(doc) else {
        *unsafe_reason = Some("unsupportedEncoding".into());
        return;
    };
    let Ok(text) = Document::decode_text(&encoding, bytes) else {
        *unsafe_reason = Some("unsupportedEncoding".into());
        return;
    };
    if text.contains('\u{fffd}') || text.trim().is_empty() {
        if text.contains('\u{fffd}') {
            *unsafe_reason = Some("unmappedGlyph".into())
        }
        return;
    }
    let advance_points = glyph_advance(font, bytes, &text, state);
    let combined = state.ctm.then(state.tm);
    let (x0, y0) = combined.point(0., state.rise);
    let (x1, y1) = combined.point(advance_points, state.rise + state.size);
    let left = x0.min(x1);
    let right = x0.max(x1);
    let bottom = y0.min(y1);
    let top = y0.max(y1);
    let nx = ((left - crop[0]) / crop[2]).clamp(0., 1.);
    let ny = ((crop[1] + crop[3] - top) / crop[3]).clamp(0., 1.);
    let nw = ((right - left) / crop[2]).clamp(0.001, 1. - nx);
    let nh = ((top - bottom) / crop[3]).clamp(0.001, 1. - ny);
    let hint = font
        .get(b"BaseFont")
        .ok()
        .and_then(|v| v.as_name().ok())
        .map(|v| String::from_utf8_lossy(v).into_owned());
    blocks.push(PdfBlock {
        page,
        ordinal: blocks.len(),
        text,
        bounds: [nx, ny, nw, nh],
        font_hint: hint,
    });
    advance(state, advance_points)
}
fn glyph_advance(font: &Dictionary, bytes: &[u8], text: &str, state: &TextState) -> f32 {
    let widths = font.get(b"Widths").ok().and_then(|v| v.as_array().ok());
    let first = font
        .get(b"FirstChar")
        .ok()
        .and_then(|v| v.as_i64().ok())
        .unwrap_or(0);
    let units = if bytes.len() == text.chars().count() {
        bytes
            .iter()
            .map(|b| {
                widths
                    .and_then(|w| w.get((*b as i64 - first).max(0) as usize))
                    .and_then(|v| number(Some(v)))
                    .unwrap_or(500.)
            })
            .sum::<f32>()
            / 1000.
    } else {
        text.chars().count() as f32 * 0.5
    };
    let spaces = text.chars().filter(|c| *c == ' ').count() as f32;
    ((units * state.size)
        + (text.chars().count() as f32 * state.char_space)
        + (spaces * state.word_space))
        * state.scale
}
fn advance(state: &mut TextState, value: f32) {
    state.tm = state.tm.then(Matrix([1., 0., 0., 1., value, 0.]))
}
fn matrix(values: &[Object]) -> Option<Matrix> {
    if values.len() < 6 {
        return None;
    }
    Some(Matrix([
        number(values.first())?,
        number(values.get(1))?,
        number(values.get(2))?,
        number(values.get(3))?,
        number(values.get(4))?,
        number(values.get(5))?,
    ]))
}
fn vertical_font(fonts: &BTreeMap<Vec<u8>, &Dictionary>, name: &[u8]) -> bool {
    fonts
        .get(name)
        .and_then(|f| f.get(b"Encoding").ok())
        .and_then(|v| v.as_name().ok())
        .is_some_and(|v| v.ends_with(b"-V"))
}
fn page_xobjects<'a>(doc: &'a Document, page_id: ObjectId) -> BTreeMap<Vec<u8>, &'a Dictionary> {
    let mut out = BTreeMap::new();
    let Ok((resource, _)) = doc.get_page_resources(page_id) else {
        return out;
    };
    let Some(resources) = resource else {
        return out;
    };
    let Ok(x) = resources
        .get_deref(b"XObject", doc)
        .and_then(Object::as_dict)
    else {
        return out;
    };
    for (name, value) in x {
        if let Ok(id) = value.as_reference() {
            if let Ok(dict) = doc
                .get_object(id)
                .and_then(Object::as_stream)
                .map(|s| &s.dict)
            {
                out.insert(name.clone(), dict);
            }
        }
    }
    out
}
fn display_to_page_bounds(b: [f32; 4], rotation: i32) -> [f32; 4] {
    match rotation.rem_euclid(360) {
        90 => [b[1], 1. - b[0] - b[2], b[3], b[2]],
        180 => [1. - b[0] - b[2], 1. - b[1] - b[3], b[2], b[3]],
        270 => [1. - b[1] - b[3], b[0], b[3], b[2]],
        _ => b,
    }
}
pub fn ocr_bounds_to_page(bounds: [f32; 4], rotation: i32) -> [f32; 4] {
    display_to_page_bounds(bounds, rotation)
}
pub fn page_bounds_to_display(b: [f32; 4], rotation: i32) -> [f32; 4] {
    match rotation.rem_euclid(360) {
        90 => [1. - b[1] - b[3], b[0], b[3], b[2]],
        180 => [1. - b[0] - b[2], 1. - b[1] - b[3], b[2], b[3]],
        270 => [b[1], 1. - b[0] - b[2], b[3], b[2]],
        _ => b,
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
fn page_crop(doc: &Document, dict: &Dictionary) -> Option<[f32; 4]> {
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
    Some([x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()])
}
fn resolve_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok(),
        _ => None,
    }
}
