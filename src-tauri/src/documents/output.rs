use super::types::DocumentError;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn next_output_path(source: &Path, target_language: &str) -> Result<PathBuf, DocumentError> {
    next_output_path_in(source, target_language, None)
}

pub fn next_output_path_in(source: &Path, target_language: &str, directory: Option<&Path>) -> Result<PathBuf, DocumentError> {
    let parent = directory.or_else(|| source.parent()).ok_or(DocumentError::Io)?;
    if !parent.is_dir() { return Err(DocumentError::Io); }
    let stem = source
        .file_stem()
        .and_then(|v| v.to_str())
        .ok_or(DocumentError::Io)?;
    let ext = source
        .extension()
        .and_then(|v| v.to_str())
        .ok_or(DocumentError::Io)?;
    let language: String = target_language
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    if language.is_empty() {
        return Err(DocumentError::Io);
    }
    for n in 1..=10_000u32 {
        let suffix = if n == 1 {
            String::new()
        } else {
            format!("_{n}")
        };
        let candidate = parent.join(format!("{stem}_번역_{language}{suffix}.{ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(DocumentError::OutputExists)
}

pub fn publish_atomic(output: &Path, bytes: &[u8]) -> Result<(), DocumentError> {
    if output.exists() {
        return Err(DocumentError::OutputExists);
    }
    let parent = output.parent().ok_or(DocumentError::Io)?;
    let partial = parent.join(format!(".smartcat-partial-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial)
            .map_err(|_| DocumentError::Io)?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| DocumentError::Io)?;
        drop(file);
        fs::rename(&partial, output).map_err(|_| DocumentError::Io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}
