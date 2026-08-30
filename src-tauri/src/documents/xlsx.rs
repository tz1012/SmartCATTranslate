use std::collections::{BTreeMap, HashMap};

use quick_xml::{events::Event, Reader};

use super::{
    ooxml::{xml::extract_text_nodes, OoxmlPackage},
    types::{
        stable_segment_id, DocumentError, DocumentFormat, DocumentOptions, Segment,
        TranslatedSegment,
    },
};

pub fn parts(package: &OoxmlPackage, options: &DocumentOptions) -> Vec<String> {
    package
        .entries
        .keys()
        .filter(|name| {
            name.as_str() == "xl/sharedStrings.xml"
                || name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml")
                || options.include_comments
                    && name.starts_with("xl/comments")
                    && name.ends_with(".xml")
        })
        .cloned()
        .collect()
}

pub fn extract(
    package: &OoxmlPackage,
    options: &DocumentOptions,
    source_fingerprint: &str,
) -> Result<Vec<Segment>, DocumentError> {
    let sheets = workbook_sheets(package)?;
    let mut worksheet_locations: HashMap<String, Vec<Option<String>>> = HashMap::new();
    let mut shared_consumers: BTreeMap<usize, Vec<String>> = BTreeMap::new();

    for sheet in &sheets {
        let xml = package
            .read(&sheet.part)
            .ok_or(DocumentError::InvalidPackage)?;
        let metadata = worksheet_text_metadata(xml, &sheet.name)?;
        worksheet_locations.insert(sheet.part.clone(), metadata.inline_locations);
        for (shared_index, cells) in metadata.shared_consumers {
            shared_consumers
                .entry(shared_index)
                .or_default()
                .extend(cells);
        }
    }

    let shared_text_indices = package
        .read("xl/sharedStrings.xml")
        .map(shared_text_indices)
        .transpose()?
        .unwrap_or_default();
    let mut shared_part_ordinals = HashMap::<usize, usize>::new();
    let mut segments = Vec::new();
    for part in parts(package, options) {
        let values = extract_text_nodes(
            package.read(&part).ok_or(DocumentError::InvalidPackage)?,
            b"t",
        )?;
        for (ordinal, text) in values.into_iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let location = if part == "xl/sharedStrings.xml" {
                let shared_index = *shared_text_indices
                    .get(ordinal)
                    .ok_or(DocumentError::InvalidPackage)?;
                let consumers = shared_consumers.get(&shared_index);
                let Some(primary) = consumers.and_then(|values| values.first()) else {
                    // An orphaned shared string has no visible worksheet location and must not
                    // create a report item that cannot be previewed in the translated workbook.
                    continue;
                };
                let part_ordinal = shared_part_ordinals.entry(shared_index).or_default();
                *part_ordinal += 1;
                format!(
                    "{primary}/sharedString:{}/text:{}/uses:{}",
                    shared_index + 1,
                    *part_ordinal,
                    consumers.map(Vec::len).unwrap_or(1)
                )
            } else if let Some(locations) = worksheet_locations.get(&part) {
                locations
                    .get(ordinal)
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| format!("{part}/text:{}", ordinal + 1))
            } else {
                format!("{part}/text:{}", ordinal + 1)
            };
            segments.push(Segment {
                id: stable_segment_id(
                    source_fingerprint,
                    DocumentFormat::Xlsx,
                    &part,
                    &location,
                    ordinal,
                ),
                part: part.clone(),
                ordinal,
                location,
                text,
            });
        }
    }
    Ok(segments)
}

pub fn rebuild(
    package: &mut OoxmlPackage,
    segments: &[Segment],
    translated: &[TranslatedSegment],
) -> Result<(), DocumentError> {
    super::pipeline::replace_selected_nodes(package, segments, translated, b"t")
}

struct WorkbookSheet {
    name: String,
    part: String,
}

fn workbook_sheets(package: &OoxmlPackage) -> Result<Vec<WorkbookSheet>, DocumentError> {
    let relationships = workbook_relationships(
        package
            .read("xl/_rels/workbook.xml.rels")
            .ok_or(DocumentError::InvalidPackage)?,
    )?;
    let xml = package
        .read("xl/workbook.xml")
        .ok_or(DocumentError::InvalidPackage)?;
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut sheets = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) | Event::Empty(event)
                if event.local_name().as_ref() == b"sheet" =>
            {
                let name = attr_string(&event, b"name").ok_or(DocumentError::InvalidPackage)?;
                let id = attr_string(&event, b"id").ok_or(DocumentError::InvalidPackage)?;
                let part = relationships
                    .get(&id)
                    .cloned()
                    .ok_or(DocumentError::InvalidPackage)?;
                if package.read(&part).is_none() {
                    return Err(DocumentError::InvalidPackage);
                }
                sheets.push(WorkbookSheet {
                    name: sanitize_sheet_name(&name),
                    part,
                });
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(sheets)
}

fn workbook_relationships(xml: &[u8]) -> Result<HashMap<String, String>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut relationships = HashMap::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) | Event::Empty(event)
                if event.local_name().as_ref() == b"Relationship" =>
            {
                let Some(id) = attr_string(&event, b"Id") else {
                    return Err(DocumentError::InvalidPackage);
                };
                let Some(target) = attr_string(&event, b"Target") else {
                    return Err(DocumentError::InvalidPackage);
                };
                relationships.insert(id, normalize_workbook_target(&target)?);
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(relationships)
}

fn normalize_workbook_target(target: &str) -> Result<String, DocumentError> {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches('/');
    if target.is_empty()
        || target.split('/').any(|component| {
            component.is_empty() || matches!(component, "." | "..") || component.contains(':')
        })
    {
        return Err(DocumentError::InvalidPackage);
    }
    Ok(if target.starts_with("xl/") {
        target.to_owned()
    } else {
        format!("xl/{target}")
    })
}

struct WorksheetTextMetadata {
    inline_locations: Vec<Option<String>>,
    shared_consumers: BTreeMap<usize, Vec<String>>,
}

fn worksheet_text_metadata(
    xml: &[u8],
    sheet_name: &str,
) -> Result<WorksheetTextMetadata, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut cell_ref: Option<String> = None;
    let mut cell_type = String::new();
    let mut value = String::new();
    let mut inside_value = false;
    let mut inside_text = false;
    let mut inline_ordinal = 0usize;
    let mut inline_locations = Vec::new();
    let mut shared_consumers = BTreeMap::<usize, Vec<String>>::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) if event.local_name().as_ref() == b"c" => {
                cell_ref = attr_string(&event, b"r");
                cell_type = attr_string(&event, b"t").unwrap_or_default();
                value.clear();
                inline_ordinal = 0;
            }
            Event::Start(event) if event.local_name().as_ref() == b"v" && cell_ref.is_some() => {
                inside_value = true;
            }
            Event::Start(event) if event.local_name().as_ref() == b"t" => inside_text = true,
            Event::Text(event) if inside_value => {
                value.push_str(
                    &event
                        .xml10_content()
                        .map_err(|_| DocumentError::InvalidPackage)?,
                );
                inside_value = false;
            }
            Event::Text(_) if inside_text => {
                let location = if cell_type == "inlineStr" {
                    cell_ref.as_ref().map(|reference| {
                        inline_ordinal += 1;
                        format!(
                            "sheet:{sheet_name}/cell:{}/inlineText:{}",
                            sanitize_cell_reference(reference),
                            inline_ordinal
                        )
                    })
                } else {
                    None
                };
                inline_locations.push(location);
                inside_text = false;
            }
            Event::End(event) if event.local_name().as_ref() == b"v" => inside_value = false,
            Event::End(event) if event.local_name().as_ref() == b"t" => inside_text = false,
            Event::End(event) if event.local_name().as_ref() == b"c" => {
                if cell_type == "s" {
                    if let (Some(reference), Ok(shared_index)) =
                        (cell_ref.as_ref(), value.parse::<usize>())
                    {
                        shared_consumers
                            .entry(shared_index)
                            .or_default()
                            .push(format!(
                                "sheet:{sheet_name}/cell:{}",
                                sanitize_cell_reference(reference)
                            ));
                    }
                }
                cell_ref = None;
                cell_type.clear();
                value.clear();
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(WorksheetTextMetadata {
        inline_locations,
        shared_consumers,
    })
}

fn shared_text_indices(xml: &[u8]) -> Result<Vec<usize>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut shared_index = 0usize;
    let mut inside_item = false;
    let mut inside_text = false;
    let mut indices = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(event) if event.local_name().as_ref() == b"si" => inside_item = true,
            Event::Start(event) if event.local_name().as_ref() == b"t" => inside_text = true,
            Event::Text(_) if inside_text => {
                if !inside_item {
                    return Err(DocumentError::InvalidPackage);
                }
                indices.push(shared_index);
                inside_text = false;
            }
            Event::End(event) if event.local_name().as_ref() == b"t" => inside_text = false,
            Event::End(event) if event.local_name().as_ref() == b"si" => {
                inside_item = false;
                shared_index += 1;
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(indices)
}

fn attr_string(event: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    event
        .attributes()
        .with_checks(false)
        .flatten()
        .find(|attribute| attribute.key.local_name().as_ref() == name)
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
}

fn sanitize_sheet_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect()
}

fn sanitize_cell_reference(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '$')
        .take(32)
        .collect::<String>()
        .to_ascii_uppercase()
}
