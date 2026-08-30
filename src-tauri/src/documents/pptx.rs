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
    source_fingerprint: &str,
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
                let location = format!("{part}/text:{}", ordinal + 1);
                segments.push(Segment {
                    id: stable_segment_id(
                        source_fingerprint,
                        DocumentFormat::Pptx,
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
