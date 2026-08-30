use super::{
    docx,
    ooxml::{
        xml::{extract_text_nodes, replace_text_nodes},
        OoxmlPackage,
    },
    output::{next_output_path_in, publish_atomic, publish_existing_partial},
    pdf, pptx,
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
    let (segments, part_count, page_count, page_kinds, has_signatures, has_forms, has_annotations) =
        if format == DocumentFormat::Pdf {
            let inspection = pdf::inspect(source, options.pdf_force_ocr)?;
            (
                inspection.segments,
                inspection.pages.len(),
                inspection.pages.len(),
                inspection
                    .pages
                    .iter()
                    .map(|p| p.kind.as_str().to_owned())
                    .collect(),
                inspection.has_signatures,
                inspection.has_forms,
                inspection.has_annotations,
            )
        } else {
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
                DocumentFormat::Pdf => unreachable!(),
            };
            (
                segments,
                package.entries.len(),
                0,
                Vec::new(),
                false,
                false,
                false,
            )
        };
    let manifest = DocumentManifest {
        format,
        file_name,
        segment_count: segments.len(),
        part_count,
        source_hash,
        page_count,
        page_kinds,
        has_signatures,
        has_forms,
        has_annotations,
    };
    Ok(DocumentPlan {
        source: source.to_owned(),
        format,
        manifest,
        segments,
        pdf_spool: None,
        resumed_from_stage: None,
    })
}

pub fn rebuild_document(
    plan: &DocumentPlan,
    translated: &[TranslatedSegment],
    options: &DocumentOptions,
    job_id: uuid::Uuid,
) -> Result<DocumentReport, DocumentError> {
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    rebuild_document_checked(plan, translated, options, job_id, &cancelled, &|_| {})
}

pub fn rebuild_document_checked(
    plan: &DocumentPlan,
    translated: &[TranslatedSegment],
    options: &DocumentOptions,
    job_id: uuid::Uuid,
    cancelled: &std::sync::atomic::AtomicBool,
    checkpoint: &(dyn Fn(&DocumentCheckpoint) + Sync),
) -> Result<DocumentReport, DocumentError> {
    use std::sync::atomic::Ordering;
    if cancelled.load(Ordering::Acquire) {
        return Err(DocumentError::Cancelled);
    }
    let source_bytes = fs::read(&plan.source).map_err(|_| DocumentError::Io)?;
    if hash_bytes(&source_bytes) != plan.manifest.source_hash {
        return Err(DocumentError::SourceChanged);
    }
    let output_directory = options.output_directory.as_deref().map(Path::new);
    let output = next_output_path_in(&plan.source, &options.target_language, output_directory)?;
    if plan.format == DocumentFormat::Pdf {
        let inspection = pdf::inspect(&plan.source, options.pdf_force_ocr)?;
        let parent = output.parent().ok_or(DocumentError::Io)?;
        let partial = parent.join(format!(".smartcat-partial-{}.pdf", uuid::Uuid::new_v4()));
        let result = (|| {
            let warnings = pdf::rebuild(
                &plan.source,
                &inspection,
                &plan.segments,
                translated,
                &partial,
                options,
                plan.pdf_spool.as_ref(),
                &translated
                    .iter()
                    .map(|value| format!("segment:{}", value.id))
                    .collect::<Vec<_>>(),
                cancelled,
                checkpoint,
            )?;
            if hash_bytes(&fs::read(&plan.source).map_err(|_| DocumentError::Io)?)
                != plan.manifest.source_hash
            {
                return Err(DocumentError::SourceChanged);
            }
            if cancelled.load(Ordering::Acquire) {
                return Err(DocumentError::Cancelled);
            }
            publish_existing_partial(&partial, &output)?;
            let output_bytes = match fs::read(&output) {
                Ok(v) => v,
                Err(_) => {
                    let _ = fs::remove_file(&output);
                    return Err(DocumentError::Io);
                }
            };
            Ok(DocumentReport {
                job_id,
                format: plan.format,
                output_path: output.to_string_lossy().into_owned(),
                output_name: output
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("translated.pdf")
                    .to_owned(),
                translated_segments: translated.len(),
                warnings,
                publishable: true,
                source_hash: plan.manifest.source_hash.clone(),
                output_hash: hash_bytes(&output_bytes),
                resumed_from_stage: plan.resumed_from_stage.clone(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&partial);
        }
        return result;
    }
    let mut package = OoxmlPackage::open(&source_bytes)?;
    match plan.format {
        DocumentFormat::Docx => docx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Pptx => pptx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Xlsx => xlsx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Pdf => unreachable!(),
    };
    let output_bytes = package.write()?;
    let reopened = OoxmlPackage::open(&output_bytes)?;
    if reopened.entries.len() != plan.manifest.part_count {
        return Err(DocumentError::ValidationFailed);
    }
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
        resumed_from_stage: plan.resumed_from_stage.clone(),
    })
}

pub fn set_resume_checkpoint(
    plan: &mut DocumentPlan,
    checkpoint: &DocumentCheckpoint,
    encrypted_results: &HashMap<String, TranslatedSegment>,
) -> Result<DocumentResumeState, DocumentError> {
    if checkpoint.source_fingerprint != plan.manifest.source_hash
        || checkpoint.raster_refs.iter().any(|value| {
            let path = Path::new(value);
            path.is_absolute()
                || path.components().any(|part| {
                    matches!(
                        part,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
        })
    {
        return Err(DocumentError::InvalidPackage);
    }
    plan.resumed_from_stage = Some(format!("{:?}", checkpoint.stage).to_ascii_lowercase());
    if let Some(spool) = plan.pdf_spool.as_mut() {
        spool.refs = checkpoint
            .raster_refs
            .iter()
            .filter_map(|relative| {
                let stem = Path::new(relative).file_stem()?.to_str()?;
                let page = stem.strip_prefix("page-")?.parse().ok()?;
                Some((page, relative.clone()))
            })
            .collect();
    }
    let translated = checkpoint
        .translated_result_refs
        .iter()
        .map(|reference| {
            encrypted_results
                .get(reference)
                .cloned()
                .ok_or(DocumentError::InvalidPackage)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let batch_cursor = if checkpoint.stage == DocumentStage::Translate {
        checkpoint.completed
    } else {
        checkpoint
            .stable_unit_id
            .strip_prefix("batch:")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    Ok(DocumentResumeState {
        batch_cursor,
        translated,
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
