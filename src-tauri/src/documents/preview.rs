use std::{collections::HashMap, io::Cursor, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use quick_xml::{events::Event, Reader};
use serde::Serialize;

use super::{
    ooxml::{xml::extract_text_nodes, PreviewPackage},
    DocumentError, DocumentFormat,
};

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DocumentResultPreview {
    PdfPage {
        location: String,
        label: String,
        image_data_url: String,
        width: u32,
        height: u32,
    },
    PptxSlide {
        location: String,
        label: String,
        width: u64,
        height: u64,
        focus_text_ordinal: Option<usize>,
        shapes: Vec<PreviewShape>,
    },
    XlsxCell {
        location: String,
        label: String,
        focus_cell: String,
        columns: Vec<String>,
        rows: Vec<Vec<PreviewCell>>,
    },
    DocxContext {
        location: String,
        label: String,
        lines: Vec<PreviewLine>,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewShape {
    pub id: String,
    pub name: String,
    pub text: String,
    pub x: u64,
    pub y: u64,
    pub width: u64,
    pub height: u64,
    pub text_start: usize,
    pub text_end: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCell {
    pub reference: String,
    pub value: String,
    pub focused: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewLine {
    pub ordinal: usize,
    pub text: String,
    pub focused: bool,
}

pub async fn read_result_preview(
    output: &Path,
    location: &str,
) -> Result<DocumentResultPreview, DocumentError> {
    if location.is_empty()
        || location.len() > 512
        || location.chars().any(char::is_control)
        || !output.is_file()
    {
        return Err(DocumentError::InvalidPackage);
    }
    match DocumentFormat::from_path(output).ok_or(DocumentError::Unsupported)? {
        DocumentFormat::Pdf => preview_pdf(output, location).await,
        DocumentFormat::Pptx => preview_pptx(output, location),
        DocumentFormat::Xlsx => preview_xlsx(output, location),
        DocumentFormat::Docx => preview_docx(output, location),
    }
}

async fn preview_pdf(
    output: &Path,
    location: &str,
) -> Result<DocumentResultPreview, DocumentError> {
    let page = number_after(location, "page:").ok_or(DocumentError::Unsupported)?;
    if page == 0 {
        return Err(DocumentError::Unsupported);
    }
    let rendered = super::pdf::render_page(output, page as u32, 96).await?;
    let rgba = image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.rgba)
        .ok_or(DocumentError::InvalidPackage)?;
    let preview = image::DynamicImage::ImageRgba8(rgba);
    let preview = if preview.width() > 1_200 || preview.height() > 900 {
        preview.thumbnail(1_200, 900)
    } else {
        preview
    };
    let (width, height) = (preview.width(), preview.height());
    let mut bytes = Cursor::new(Vec::new());
    preview
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|_| DocumentError::Io)?;
    Ok(DocumentResultPreview::PdfPage {
        location: location.to_owned(),
        label: format!("Page {page}"),
        image_data_url: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes.into_inner())
        ),
        width,
        height,
    })
}

fn preview_pptx(output: &Path, location: &str) -> Result<DocumentResultPreview, DocumentError> {
    let slide = number_after(location, "slide:")
        .or_else(|| number_after(location, "slides/slide"))
        .ok_or(DocumentError::Unsupported)?;
    let mut package = PreviewPackage::open(output)?;
    let part = format!("ppt/slides/slide{slide}.xml");
    let xml = package.read(&part)?;
    let presentation = package.read_optional("ppt/presentation.xml")?;
    let (width, height) = presentation
        .as_deref()
        .and_then(slide_size)
        .unwrap_or((12_192_000, 6_858_000));
    let shapes = slide_shapes(&xml)?;
    let focus_text_ordinal = number_after(location, "text:").map(|value| value.saturating_sub(1));
    Ok(DocumentResultPreview::PptxSlide {
        location: location.to_owned(),
        label: format!("Slide {slide}"),
        width,
        height,
        focus_text_ordinal,
        shapes,
    })
}

fn slide_size(xml: &[u8]) -> Option<(u64, u64)> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(event) | Event::Empty(event)
                if event.local_name().as_ref() == b"sldSz" =>
            {
                return Some((attr_u64(&event, b"cx")?, attr_u64(&event, b"cy")?));
            }
            Event::DocType(_) | Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    }
}

fn slide_shapes(xml: &[u8]) -> Result<Vec<PreviewShape>, DocumentError> {
    #[derive(Default)]
    struct Pending {
        id: String,
        name: String,
        text: Vec<String>,
        x: u64,
        y: u64,
        width: u64,
        height: u64,
        text_start: usize,
    }
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut shape: Option<(usize, Pending)> = None;
    let mut inside_text = false;
    let mut text_ordinal = 0usize;
    let mut shapes = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) => {
                depth += 1;
                let local = event.local_name();
                if shape.is_none() && matches!(local.as_ref(), b"sp" | b"graphicFrame" | b"pic") {
                    shape = Some((
                        depth,
                        Pending {
                            text_start: text_ordinal,
                            ..Default::default()
                        },
                    ));
                }
                if let Some((_, pending)) = shape.as_mut() {
                    match local.as_ref() {
                        b"cNvPr" => {
                            pending.id = attr_string(&event, b"id").unwrap_or_default();
                            pending.name = attr_string(&event, b"name").unwrap_or_default();
                        }
                        b"off" => {
                            pending.x = attr_u64(&event, b"x").unwrap_or(0);
                            pending.y = attr_u64(&event, b"y").unwrap_or(0);
                        }
                        b"ext" => {
                            pending.width = attr_u64(&event, b"cx").unwrap_or(0);
                            pending.height = attr_u64(&event, b"cy").unwrap_or(0);
                        }
                        b"t" => inside_text = true,
                        _ => {}
                    }
                }
            }
            Event::Empty(event) => {
                if let Some((_, pending)) = shape.as_mut() {
                    match event.local_name().as_ref() {
                        b"cNvPr" => {
                            pending.id = attr_string(&event, b"id").unwrap_or_default();
                            pending.name = attr_string(&event, b"name").unwrap_or_default();
                        }
                        b"off" => {
                            pending.x = attr_u64(&event, b"x").unwrap_or(0);
                            pending.y = attr_u64(&event, b"y").unwrap_or(0);
                        }
                        b"ext" => {
                            pending.width = attr_u64(&event, b"cx").unwrap_or(0);
                            pending.height = attr_u64(&event, b"cy").unwrap_or(0);
                        }
                        _ => {}
                    }
                }
            }
            Event::Text(event) if inside_text => {
                if let Some((_, pending)) = shape.as_mut() {
                    pending.text.push(
                        event
                            .xml10_content()
                            .map_err(|_| DocumentError::InvalidPackage)?
                            .into_owned(),
                    );
                }
                text_ordinal += 1;
                inside_text = false;
            }
            Event::End(event) => {
                if event.local_name().as_ref() == b"t" {
                    inside_text = false;
                }
                if shape.as_ref().is_some_and(|(start, _)| *start == depth) {
                    let (_, pending) = shape.take().expect("shape exists");
                    if !pending.text.is_empty() || pending.width > 0 || pending.height > 0 {
                        shapes.push(PreviewShape {
                            id: sanitize(&pending.id, 80),
                            name: sanitize(&pending.name, 160),
                            text: sanitize(&pending.text.join(" "), 1_000),
                            x: pending.x,
                            y: pending.y,
                            width: pending.width,
                            height: pending.height,
                            text_start: pending.text_start,
                            text_end: text_ordinal,
                        });
                    }
                    if shapes.len() >= 200 {
                        break;
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(shapes)
}

fn preview_xlsx(output: &Path, location: &str) -> Result<DocumentResultPreview, DocumentError> {
    let focus = cell_after(location).ok_or(DocumentError::Unsupported)?;
    let mut package = PreviewPackage::open(output)?;
    let workbook = package.read("xl/workbook.xml")?;
    let relationships = package.read("xl/_rels/workbook.xml.rels")?;
    let part =
        worksheet_part(location, &workbook, &relationships).ok_or(DocumentError::Unsupported)?;
    let shared = package
        .read_optional("xl/sharedStrings.xml")?
        .as_deref()
        .map(shared_strings)
        .transpose()?
        .unwrap_or_default();
    let worksheet = package.read(&part)?;
    let cells = worksheet_cells(&worksheet, &shared)?;
    let (focus_col, focus_row) = parse_cell(&focus).ok_or(DocumentError::Unsupported)?;
    let start_col = focus_col.saturating_sub(2).max(1);
    let end_col = focus_col.saturating_add(2);
    let start_row = focus_row.saturating_sub(2).max(1);
    let end_row = focus_row.saturating_add(2);
    let columns = (start_col..=end_col).map(column_name).collect::<Vec<_>>();
    let rows = (start_row..=end_row)
        .map(|row| {
            (start_col..=end_col)
                .map(|col| {
                    let reference = format!("{}{}", column_name(col), row);
                    PreviewCell {
                        value: sanitize(
                            cells.get(&reference).map(String::as_str).unwrap_or(""),
                            500,
                        ),
                        focused: reference == focus,
                        reference,
                    }
                })
                .collect()
        })
        .collect();
    Ok(DocumentResultPreview::XlsxCell {
        location: location.to_owned(),
        label: focus.clone(),
        focus_cell: focus,
        columns,
        rows,
    })
}

fn worksheet_part(location: &str, workbook: &[u8], relationships: &[u8]) -> Option<String> {
    if let Some(start) = location.find("xl/worksheets/sheet") {
        let suffix = &location[start + "xl/worksheets/".len()..];
        let name = suffix.split('/').next()?;
        return Some(format!("xl/worksheets/{name}"));
    }
    let sheet = location.strip_prefix("sheet:")?.rsplit_once("/cell:")?.0;
    workbook_sheet_part(workbook, relationships, sheet).or_else(|| {
        sheet
            .strip_prefix("sheet")
            .and_then(|value| value.parse::<usize>().ok())
            .map(|index| format!("xl/worksheets/sheet{index}.xml"))
    })
}

fn workbook_sheet_part(workbook: &[u8], relationships: &[u8], wanted: &str) -> Option<String> {
    let mut reader = Reader::from_reader(workbook);
    let mut buf = Vec::new();
    let relationship_id = loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(event) | Event::Empty(event)
                if event.local_name().as_ref() == b"sheet"
                    && attr_string(&event, b"name").as_deref() == Some(wanted) =>
            {
                break attr_string(&event, b"id")?;
            }
            Event::DocType(_) | Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    };
    let mut reader = Reader::from_reader(relationships);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(event) | Event::Empty(event)
                if event.local_name().as_ref() == b"Relationship"
                    && attr_string(&event, b"Id").as_deref() == Some(&relationship_id) =>
            {
                let target = attr_string(&event, b"Target")?;
                return safe_workbook_target(&target);
            }
            Event::DocType(_) | Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    }
}

fn worksheet_cells(
    xml: &[u8],
    shared: &[String],
) -> Result<HashMap<String, String>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut cell_ref = None;
    let mut cell_type = String::new();
    let mut value = String::new();
    let mut inside_value = false;
    let mut cells = HashMap::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) if event.local_name().as_ref() == b"c" => {
                cell_ref = attr_string(&event, b"r");
                cell_type = attr_string(&event, b"t").unwrap_or_default();
                value.clear();
            }
            Event::Start(event)
                if cell_ref.is_some() && matches!(event.local_name().as_ref(), b"v" | b"t") =>
            {
                inside_value = true;
            }
            Event::Text(event) if inside_value => {
                value.push_str(
                    &event
                        .xml10_content()
                        .map_err(|_| DocumentError::InvalidPackage)?,
                );
                inside_value = false;
            }
            Event::End(event) if event.local_name().as_ref() == b"c" => {
                if let Some(reference) = cell_ref.take() {
                    let resolved = if cell_type == "s" {
                        value
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared.get(index))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        value.clone()
                    };
                    cells.insert(reference, resolved);
                }
            }
            Event::End(event) if matches!(event.local_name().as_ref(), b"v" | b"t") => {
                inside_value = false;
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(cells)
}

fn shared_strings(xml: &[u8]) -> Result<Vec<String>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut inside_item = false;
    let mut inside_text = false;
    let mut current = String::new();
    let mut values = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) if event.local_name().as_ref() == b"si" => {
                inside_item = true;
                current.clear();
            }
            Event::Start(event) if inside_item && event.local_name().as_ref() == b"t" => {
                inside_text = true;
            }
            Event::Text(event) if inside_text => {
                current.push_str(
                    &event
                        .xml10_content()
                        .map_err(|_| DocumentError::InvalidPackage)?,
                );
                inside_text = false;
            }
            Event::End(event) if event.local_name().as_ref() == b"t" => inside_text = false,
            Event::End(event) if event.local_name().as_ref() == b"si" => {
                values.push(current.clone());
                inside_item = false;
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(values)
}

fn preview_docx(output: &Path, location: &str) -> Result<DocumentResultPreview, DocumentError> {
    let marker = location.rfind("/text:").ok_or(DocumentError::Unsupported)?;
    let part = &location[..marker];
    if !part.starts_with("word/") || !part.ends_with(".xml") {
        return Err(DocumentError::Unsupported);
    }
    let ordinal = location[marker + 6..]
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(DocumentError::Unsupported)?;
    let mut package = PreviewPackage::open(output)?;
    let xml = package.read(part)?;
    let values = extract_text_nodes(&xml, b"t")?;
    let focus = ordinal - 1;
    if focus >= values.len() {
        return Err(DocumentError::Unsupported);
    }
    let start = focus.saturating_sub(3);
    let end = (focus + 4).min(values.len());
    let lines = values[start..end]
        .iter()
        .enumerate()
        .map(|(offset, text)| PreviewLine {
            ordinal: start + offset + 1,
            text: sanitize(text, 1_000),
            focused: start + offset == focus,
        })
        .collect();
    Ok(DocumentResultPreview::DocxContext {
        location: location.to_owned(),
        label: sanitize(part, 180),
        lines,
    })
}

fn attr_string(event: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    event
        .attributes()
        .with_checks(false)
        .flatten()
        .find(|attribute| attribute.key.local_name().as_ref() == name)
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
}

fn attr_u64(event: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<u64> {
    attr_string(event, name)?.parse().ok()
}

fn number_after(location: &str, marker: &str) -> Option<usize> {
    let start = location.find(marker)? + marker.len();
    let digits = location[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn cell_after(location: &str) -> Option<String> {
    let start = location.find("cell:")? + 5;
    let cell = location[start..]
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    parse_cell(&cell).map(|_| cell)
}

fn parse_cell(cell: &str) -> Option<(usize, usize)> {
    let split = cell.find(|character: char| character.is_ascii_digit())?;
    let (column, row) = cell.split_at(split);
    if column.is_empty()
        || row.is_empty()
        || !column.chars().all(|c| c.is_ascii_uppercase())
        || !row.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let column = column.bytes().try_fold(0usize, |value, byte| {
        value
            .checked_mul(26)?
            .checked_add((byte - b'A' + 1) as usize)
    })?;
    let row = row.parse::<usize>().ok()?;
    (column > 0 && row > 0).then_some((column, row))
}

fn column_name(mut column: usize) -> String {
    let mut value = Vec::new();
    while column > 0 {
        column -= 1;
        value.push((b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    value.iter().rev().collect()
}

fn sanitize(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(max)
        .collect()
}

fn safe_workbook_target(target: &str) -> Option<String> {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches('/');
    if target.is_empty()
        || target.split('/').any(|component| {
            component.is_empty() || matches!(component, "." | "..") || component.contains(':')
        })
    {
        return None;
    }
    Some(if target.starts_with("xl/") {
        target.to_owned()
    } else {
        format!("xl/{target}")
    })
}
