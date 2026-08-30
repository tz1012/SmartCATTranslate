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
                crate::documents::translate::batches(&plan.segments).len(),
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
    let completed_batch_cursor = crate::documents::translate::batches(&plan.segments).len();
    let translated_result_refs = translated
        .iter()
        .map(|value| format!("segment:{}", value.id))
        .collect::<Vec<_>>();
    checkpoint(&stage_checkpoint(
        plan,
        DocumentStage::Reflow,
        "reflow:ooxml",
        0,
        1,
        completed_batch_cursor,
        &translated_result_refs,
    ));
    match plan.format {
        DocumentFormat::Docx => docx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Pptx => pptx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Xlsx => xlsx::rebuild(&mut package, &plan.segments, translated)?,
        DocumentFormat::Pdf => unreachable!(),
    };
    checkpoint(&stage_checkpoint(
        plan,
        DocumentStage::Reflow,
        "reflow:completed",
        1,
        1,
        completed_batch_cursor,
        &translated_result_refs,
    ));
    let output_bytes = package.write()?;
    checkpoint(&stage_checkpoint(
        plan,
        DocumentStage::Save,
        "save:partial",
        0,
        1,
        completed_batch_cursor,
        &translated_result_refs,
    ));
    let reopened = OoxmlPackage::open(&output_bytes)?;
    if reopened.entries.len() != plan.manifest.part_count {
        return Err(DocumentError::ValidationFailed);
    }
    publish_atomic(&output, &output_bytes)?;
    checkpoint(&stage_checkpoint(
        plan,
        DocumentStage::Save,
        "save:synced",
        1,
        1,
        completed_batch_cursor,
        &translated_result_refs,
    ));
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

fn stage_checkpoint(
    plan: &DocumentPlan,
    stage: DocumentStage,
    stable_unit_id: &str,
    completed: usize,
    total: usize,
    completed_batch_cursor: usize,
    translated_result_refs: &[String],
) -> DocumentCheckpoint {
    let mut raster_refs = plan
        .pdf_spool
        .as_ref()
        .map(|spool| spool.refs.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    raster_refs.sort();
    DocumentCheckpoint {
        source_fingerprint: plan.manifest.source_hash.clone(),
        stage,
        stable_unit_id: stable_unit_id.to_owned(),
        completed,
        total,
        completed_batch_cursor,
        raster_refs,
        translated_result_refs: translated_result_refs.to_vec(),
    }
}

pub fn set_resume_checkpoint(
    plan: &mut DocumentPlan,
    checkpoint: &DocumentCheckpoint,
    encrypted_results: &HashMap<String, TranslatedSegment>,
) -> Result<DocumentResumeState, DocumentError> {
    let all_batches = crate::documents::translate::batches(&plan.segments);
    if checkpoint.source_fingerprint != plan.manifest.source_hash
        || checkpoint.completed_batch_cursor > all_batches.len()
        || (checkpoint.stage == DocumentStage::Translate
            && checkpoint.completed_batch_cursor > 0
            && !checkpoint.stable_unit_id.ends_with(":completed"))
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
    // Raster refs point to plaintext spool files and are never reused. A resumed PDF is
    // rendered again from the fingerprint-verified source before translations are applied.
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
    let batch_cursor = checkpoint.completed_batch_cursor;
    let expected_ids = all_batches
        .iter()
        .take(batch_cursor)
        .flat_map(|batch| batch.iter().map(|segment| segment.id))
        .collect::<Vec<_>>();
    if translated.len() != expected_ids.len()
        || translated
            .iter()
            .zip(expected_ids)
            .any(|(translated, expected)| translated.id != expected)
    {
        return Err(DocumentError::InvalidPackage);
    }
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
