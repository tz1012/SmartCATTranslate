use super::{
    docx,
    ooxml::{
        xml::{extract_text_nodes, replace_text_nodes},
        OoxmlPackage,
    },
    output::{next_output_path, publish_atomic},
    pptx,
    types::*,
    xlsx,
};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::Path};

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn inspect_document(
    source: &Path,
    options: &DocumentOptions,
) -> Result<DocumentPlan, DocumentError> {
    let format = DocumentFormat::from_path(source).ok_or(DocumentError::Unsupported)?;
    let file_name = source
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(DocumentError::Io)?
        .to_owned();
    if file_name.to_ascii_lowercase().ends_with('m') {
        return Err(DocumentError::UnsafePackage);
    }
    let bytes = fs::read(source).map_err(|_| DocumentError::Io)?;
    let source_hash = hash_bytes(&bytes);
    let package = OoxmlPackage::open(&bytes)?;
    if package
        .entries
        .keys()
        .any(|n| n.ends_with("vbaProject.bin") || n.contains("encryptedPackage"))
    {
        return Err(DocumentError::UnsafePackage);
    }
    let segments = match format {
        DocumentFormat::Docx => docx::extract(&package, options)?,
        DocumentFormat::Pptx => pptx::extract(&package, options)?,
        DocumentFormat::Xlsx => xlsx::extract(&package, options)?,
    };
    let manifest = DocumentManifest {
        format,
        file_name,
        segment_count: segments.len(),
        part_count: package.entries.len(),
        source_hash,
    };
    Ok(DocumentPlan {
        source: source.to_owned(),
        format,
        manifest,
        segments,
    })
}

pub fn rebuild_document(
    plan: &DocumentPlan,
    translated: &[TranslatedSegment],
    options: &DocumentOptions,
    job_id: uuid::Uuid,
) -> Result<DocumentReport, DocumentError> {
    let source_bytes = fs::read(&plan.source).map_err(|_| DocumentError::Io)?;
    if hash_bytes(&source_bytes) != plan.manifest.source_hash {
        return Err(DocumentError::SourceChanged);
    }
    let mut package = OoxmlPackage::open(&source_bytes)?;
    match plan.format {
        DocumentFormat::Docx => docx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Pptx => pptx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Xlsx => xlsx::rebuild(&mut package, &plan.segments, translated)?,
    };
    let output_bytes = package.write()?;
    let reopened = OoxmlPackage::open(&output_bytes)?;
    if reopened.entries.len() != plan.manifest.part_count {
        return Err(DocumentError::ValidationFailed);
    }
    let output = next_output_path(&plan.source, &options.target_language)?;
    publish_atomic(&output, &output_bytes)?;
    if hash_bytes(&fs::read(&plan.source).map_err(|_| DocumentError::Io)?)
        != plan.manifest.source_hash
    {
        let _ = fs::remove_file(&output);
        return Err(DocumentError::SourceChanged);
    }
    let warnings = translated
        .iter()
        .filter_map(|t| {
            plan.segments.iter().find(|s| s.id == t.id).and_then(|s| {
                (t.text.chars().count() > s.text.chars().count().saturating_mul(18) / 10 + 8).then(
                    || DocumentWarning {
                        code: "possibleReflow".into(),
                        location: Some(s.location.clone()),
                        message: "Translated text may require layout adjustment.".into(),
                    },
                )
            })
        })
        .collect();
    Ok(DocumentReport {
        job_id,
        format: plan.format,
        output_path: output.to_string_lossy().into_owned(),
        output_name: output
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("translated document")
            .to_owned(),
        translated_segments: translated.len(),
        warnings,
        publishable: true,
        source_hash: plan.manifest.source_hash.clone(),
        output_hash: hash_bytes(&output_bytes),
    })
}

pub(crate) fn replace_selected_nodes(
    package: &mut OoxmlPackage,
    segments: &[Segment],
    translated: &[TranslatedSegment],
    local_name: &[u8],
) -> Result<(), DocumentError> {
    let by_id = translated
        .iter()
        .map(|v| (v.id, v))
        .collect::<HashMap<_, _>>();
    let mut by_part: HashMap<&str, Vec<&Segment>> = HashMap::new();
    for segment in segments {
        by_part.entry(&segment.part).or_default().push(segment);
    }
    for (part, selected) in by_part {
        let xml = package.read(part).ok_or(DocumentError::InvalidPackage)?;
        let mut values = extract_text_nodes(xml, local_name)?;
        for segment in selected {
            let translation = by_id
                .get(&segment.id)
                .ok_or(DocumentError::ValidationFailed)?;
            *values
                .get_mut(segment.ordinal)
                .ok_or(DocumentError::ValidationFailed)? = translation.text.clone();
        }
        package.replace(part, replace_text_nodes(xml, local_name, &values)?)?;
    }
    Ok(())
}
