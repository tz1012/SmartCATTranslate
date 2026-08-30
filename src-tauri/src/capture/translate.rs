use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{layout::TextBlock, TranslatedBlock};

const MAX_BLOCKS: usize = 2_000;

#[derive(Serialize)]
struct SourceBlock<'a> {
    id: String,
    text: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputBlock {
    id: String,
    #[serde(alias = "text", alias = "translation")]
    translated_text: String,
}

pub fn structured_source(blocks: &[TextBlock]) -> Result<String, StructuredTranslationError> {
    if blocks.is_empty() || blocks.len() > MAX_BLOCKS {
        return Err(StructuredTranslationError::InvalidInput);
    }
    let values = blocks
        .iter()
        .map(|block| SourceBlock {
            id: block.id.to_string(),
            text: &block.text,
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values).map_err(|_| StructuredTranslationError::InvalidInput)
}

pub fn parse_structured_translation(
    blocks: &[TextBlock],
    output: &str,
) -> Result<Vec<TranslatedBlock>, StructuredTranslationError> {
    let parsed: Vec<OutputBlock> =
        serde_json::from_str(output).map_err(|_| StructuredTranslationError::InvalidOutput)?;
    if parsed.len() != blocks.len() {
        return Err(StructuredTranslationError::InvalidOutput);
    }
    let expected = blocks
        .iter()
        .map(|block| (block.id.to_string(), block))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut translated = Vec::with_capacity(blocks.len());
    for item in parsed {
        let source = expected
            .get(&item.id)
            .ok_or(StructuredTranslationError::InvalidOutput)?;
        if !seen.insert(item.id) || item.translated_text.chars().count() > 200_000 {
            return Err(StructuredTranslationError::InvalidOutput);
        }
        translated.push(TranslatedBlock {
            id: source.id,
            source_ids: source.source_ids.clone(),
            source_text: source.text.clone(),
            translated_text: item.translated_text,
            bounds: source.bounds,
            confidence: source.confidence,
            direction: Some(source.direction),
            visible: true,
        });
    }
    translated.sort_by_key(|block| {
        blocks
            .iter()
            .position(|source| source.id == block.id)
            .unwrap_or(usize::MAX)
    });
    Ok(translated)
}

pub fn fallback_by_order(blocks: &[TextBlock], output: &str) -> Vec<TranslatedBlock> {
    let chunks = output.split("\n\n").collect::<Vec<_>>();
    blocks
        .iter()
        .enumerate()
        .map(|(index, source)| TranslatedBlock {
            id: source.id,
            source_ids: source.source_ids.clone(),
            source_text: source.text.clone(),
            translated_text: chunks
                .get(index)
                .copied()
                .unwrap_or(&source.text)
                .to_owned(),
            bounds: source.bounds,
            confidence: source.confidence,
            direction: Some(source.direction),
            visible: true,
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum StructuredTranslationError {
    #[error("invalid capture translation input")]
    InvalidInput,
    #[error("capture translation returned mismatched block IDs")]
    InvalidOutput,
}
