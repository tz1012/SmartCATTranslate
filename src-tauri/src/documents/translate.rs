use super::{
    segments::ProtectionMap,
    types::{DocumentError, Segment, TranslatedSegment},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
struct Input<'a> {
    id: String,
    text: &'a str,
}
#[derive(Deserialize)]
struct Output {
    id: String,
    #[serde(alias = "translatedText", alias = "translation")]
    text: String,
}

pub struct PreparedBatch {
    pub source: String,
    maps: Vec<(Uuid, ProtectionMap)>,
}
pub fn prepare_batch(
    segments: &[Segment],
    protected_terms: &[String],
    job: Uuid,
) -> Result<PreparedBatch, DocumentError> {
    if segments.is_empty()
        || segments.len() > 80
        || segments
            .iter()
            .map(|s| s.text.chars().count())
            .sum::<usize>()
            > 12_000
    {
        return Err(DocumentError::ValidationFailed);
    }
    let mut maps = Vec::with_capacity(segments.len());
    let mut input = Vec::with_capacity(segments.len());
    for segment in segments {
        let map = ProtectionMap::apply(&segment.text, protected_terms, job)?;
        maps.push((segment.id, map));
    }
    for (id, map) in &maps {
        input.push(Input {
            id: id.to_string(),
            text: &map.tokenized,
        });
    }
    let payload = serde_json::to_string(&input).map_err(|_| DocumentError::ValidationFailed)?;
    Ok(PreparedBatch {
        source: payload,
        maps,
    })
}
pub fn finish_batch(
    batch: &PreparedBatch,
    output: &str,
) -> Result<Vec<TranslatedSegment>, DocumentError> {
    let values: Vec<Output> =
        serde_json::from_str(output).map_err(|_| DocumentError::ValidationFailed)?;
    if values.len() != batch.maps.len() {
        return Err(DocumentError::ValidationFailed);
    }
    let mut result = Vec::with_capacity(values.len());
    for (id, map) in &batch.maps {
        let item = values
            .iter()
            .find(|v| v.id == id.to_string())
            .ok_or(DocumentError::ValidationFailed)?;
        result.push(TranslatedSegment {
            id: *id,
            text: map.restore(&item.text)?,
        });
    }
    Ok(result)
}

pub fn batches(segments: &[Segment]) -> Vec<&[Segment]> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < segments.len() {
        let mut end = start;
        let mut chars = 0;
        while end < segments.len()
            && end - start < 80
            && chars + segments[end].text.chars().count() <= 12_000
        {
            chars += segments[end].text.chars().count();
            end += 1;
        }
        if end == start {
            end += 1;
        }
        out.push(&segments[start..end]);
        start = end;
    }
    out
}
