use crate::core::diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Clone)]
pub struct CleanupService {
    root: PathBuf,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CleanupStats {
    pub item_count: u64,
    pub byte_count: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error("temporary storage is unavailable")]
    Unavailable,
    #[error("temporary cleanup target is outside the private root")]
    OutsideRoot,
}

impl CleanupService {
    pub fn new(root: PathBuf) -> Result<Self, CleanupError> {
        fs::create_dir_all(&root).map_err(|_| CleanupError::Unavailable)?;
        let root = root.canonicalize().map_err(|_| CleanupError::Unavailable)?;
        Ok(Self { root })
    }

    pub fn on_start(&self) -> Result<CleanupStats, CleanupError> {
        self.purge(Duration::from_secs(7 * 24 * 60 * 60))
    }
    pub fn create_job_root(&self, job_id: &str) -> Result<PathBuf, CleanupError> {
        if !valid_job_id(job_id) {
            return Err(CleanupError::OutsideRoot);
        }
        let candidate = self.root.join(job_id);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_dir() => {
                return Err(CleanupError::OutsideRoot)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&candidate).map_err(|_| CleanupError::Unavailable)?;
            }
            Err(_) => return Err(CleanupError::Unavailable),
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|_| CleanupError::Unavailable)?;
        if canonical.parent() != Some(self.root.as_path()) {
            return Err(CleanupError::OutsideRoot);
        }
        Ok(canonical)
    }
    pub fn on_job_complete(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
        self.remove_job(job_id)
    }
    pub fn on_job_cancel(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
        self.remove_job(job_id)
    }

    pub fn purge(&self, maximum_age: Duration) -> Result<CleanupStats, CleanupError> {
        let mut stats = CleanupStats::default();
        let now = SystemTime::now();
        for entry in fs::read_dir(&self.root).map_err(|_| CleanupError::Unavailable)? {
            let entry = entry.map_err(|_| CleanupError::Unavailable)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !valid_job_id(name) {
                continue;
            }
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| CleanupError::Unavailable)?;
            if is_link_or_reparse(&metadata) || !metadata.is_dir() {
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(modified).unwrap_or_default() > maximum_age {
                let removed = self.remove_job(name)?;
                stats.item_count += removed.item_count;
                stats.byte_count += removed.byte_count;
            }
        }
        DiagnosticEvent::new(
            DiagnosticEventName::TemporaryCleanup,
            DiagnosticOutcome::Succeeded,
        )
        .with_counts(stats.item_count, stats.byte_count)
        .emit();
        Ok(stats)
    }

    fn remove_job(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
        if !valid_job_id(job_id) {
            return Err(CleanupError::OutsideRoot);
        }
        let candidate = self.root.join(job_id);
        if !candidate.exists() {
            return Ok(CleanupStats::default());
        }
        let metadata = fs::symlink_metadata(&candidate).map_err(|_| CleanupError::Unavailable)?;
        if is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(CleanupError::OutsideRoot);
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|_| CleanupError::Unavailable)?;
        if canonical.parent() != Some(self.root.as_path()) {
            return Err(CleanupError::OutsideRoot);
        }
        let stats = measure_confined(&canonical, &canonical)?;
        fs::remove_dir_all(&canonical).map_err(|_| CleanupError::Unavailable)?;
        DiagnosticEvent::new(
            DiagnosticEventName::TemporaryCleanup,
            DiagnosticOutcome::Succeeded,
        )
        .with_counts(stats.item_count, stats.byte_count)
        .emit();
        Ok(stats)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn measure_confined(root: &Path, path: &Path) -> Result<CleanupStats, CleanupError> {
    let mut stats = CleanupStats::default();
    for entry in fs::read_dir(path).map_err(|_| CleanupError::Unavailable)? {
        let entry = entry.map_err(|_| CleanupError::Unavailable)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| CleanupError::Unavailable)?;
        if is_link_or_reparse(&metadata) {
            return Err(CleanupError::OutsideRoot);
        }
        let canonical = entry
            .path()
            .canonicalize()
            .map_err(|_| CleanupError::Unavailable)?;
        if canonical != root && !canonical.starts_with(root) {
            return Err(CleanupError::OutsideRoot);
        }
        stats.item_count += 1;
        if metadata.is_dir() {
            let nested = measure_confined(root, &canonical)?;
            stats.item_count += nested.item_count;
            stats.byte_count += nested.byte_count
        } else {
            stats.byte_count += metadata.len()
        }
    }
    Ok(stats)
}
#[cfg(windows)]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
#[cfg(not(windows))]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
fn valid_job_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
