use crate::documents::types::DocumentError;
use quick_xml::{events::Event, Reader, Writer};

pub fn extract_text_nodes(xml: &[u8], local_name: &[u8]) -> Result<Vec<String>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut inside = false;
    let mut values = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?
        {
            Event::Start(e) => inside = e.local_name().as_ref() == local_name,
            Event::Text(e) if inside => {
                values.push(
                    e.xml10_content()
                        .map_err(|_| DocumentError::InvalidPackage)?
                        .into_owned(),
                );
                inside = false;
            }
            Event::End(e) if e.local_name().as_ref() == local_name => inside = false,
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(values)
}

pub fn replace_text_nodes(
    xml: &[u8],
    local_name: &[u8],
    replacements: &[String],
) -> Result<Vec<u8>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut buf = Vec::new();
    let mut inside = false;
    let mut ordinal = 0usize;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|_| DocumentError::InvalidPackage)?;
        match event {
            Event::Start(ref e) => {
                inside = e.local_name().as_ref() == local_name;
                writer
                    .write_event(event.into_owned())
                    .map_err(|_| DocumentError::Io)?;
            }
            Event::Text(_) if inside => {
                let value = replacements
                    .get(ordinal)
                    .ok_or(DocumentError::ValidationFailed)?;
                writer
                    .write_event(Event::Text(quick_xml::events::BytesText::new(value)))
                    .map_err(|_| DocumentError::Io)?;
                ordinal += 1;
                inside = false;
            }
            Event::DocType(_) => return Err(DocumentError::InvalidPackage),
            Event::Eof => break,
            _ => writer
                .write_event(event.into_owned())
                .map_err(|_| DocumentError::Io)?,
        }
        buf.clear();
    }
    if ordinal != replacements.len() {
        return Err(DocumentError::ValidationFailed);
    }
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{extract_text_nodes, replace_text_nodes};

    #[test]
    fn preserves_markup_while_replacing_text() {
        let xml = br#"<?xml version="1.0"?><w:p xmlns:w="urn:w"><w:r><w:rPr/><w:t xml:space="preserve"> Hello </w:t></w:r></w:p>"#;
        let values = extract_text_nodes(xml, b"t").unwrap();
        assert_eq!(values, [" Hello "]);
        let rebuilt = replace_text_nodes(xml, b"t", &[" 안녕 ".to_owned()]).unwrap();
        let output = String::from_utf8(rebuilt).unwrap();
        assert!(output.contains("<w:rPr/>"));
        assert!(output.contains("xml:space=\"preserve\""));
        assert!(output.contains(" 안녕 "));
    }
}
