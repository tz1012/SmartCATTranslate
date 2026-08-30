use super::{
    ooxml::{xml::extract_text_nodes, OoxmlPackage},
    types::{DocumentError, DocumentOptions, Segment, TranslatedSegment},
};
use uuid::Uuid;

pub fn parts(package: &OoxmlPackage, options: &DocumentOptions) -> Vec<String> {
    package
        .entries
        .keys()
        .filter(|name| {
            name.as_str() == "word/document.xml"
                || name.starts_with("word/header") && name.ends_with(".xml")
                || name.starts_with("word/footer") && name.ends_with(".xml")
                || matches!(name.as_str(), "word/footnotes.xml" | "word/endnotes.xml")
                || options.include_comments
                    && name.starts_with("word/comments")
                    && name.ends_with(".xml")
        })
        .cloned()
        .collect()
}

pub fn extract(
    package: &OoxmlPackage,
    options: &DocumentOptions,
) -> Result<Vec<Segment>, DocumentError> {
    let mut segments = Vec::new();
    for part in parts(package, options) {
        for (ordinal, text) in extract_text_nodes(
            package.read(&part).ok_or(DocumentError::InvalidPackage)?,
            b"t",
        )?
        .into_iter()
        .enumerate()
        {
            if !text.trim().is_empty() {
                segments.push(Segment {
                    id: Uuid::new_v4(),
                    part: part.clone(),
                    ordinal,
                    location: format!("{part}/text:{}", ordinal + 1),
                    text,
                });
            }
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
