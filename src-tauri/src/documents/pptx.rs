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
            (name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
                || (options.include_notes
                    && name.starts_with("ppt/notesSlides/notesSlide")
                    && name.ends_with(".xml"))
                || name.starts_with("ppt/charts/chart") && name.ends_with(".xml")
                || name.starts_with("ppt/diagrams/data") && name.ends_with(".xml")
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
