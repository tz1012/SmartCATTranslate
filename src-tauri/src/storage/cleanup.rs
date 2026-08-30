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
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(modified).unwrap_or_default() > maximum_age {
                let removed = self.remove_job(name)?;
                stats.item_count += removed.item_count;
                stats.byte_count += removed.byte_count;
            }
        }
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
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CleanupError::OutsideRoot);
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|_| CleanupError::Unavailable)?;
        if canonical.parent() != Some(self.root.as_path()) {
            return Err(CleanupError::OutsideRoot);
        }
        let stats = measure_without_symlinks(&canonical)?;
        fs::remove_dir_all(&canonical).map_err(|_| CleanupError::Unavailable)?;
        Ok(stats)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn measure_without_symlinks(path: &Path) -> Result<CleanupStats, CleanupError> {
    let mut stats = CleanupStats::default();
    for entry in fs::read_dir(path).map_err(|_| CleanupError::Unavailable)? {
        let entry = entry.map_err(|_| CleanupError::Unavailable)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| CleanupError::Unavailable)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        stats.item_count += 1;
        if metadata.is_dir() {
            let nested = measure_without_symlinks(&entry.path())?;
            stats.item_count += nested.item_count;
            stats.byte_count += nested.byte_count
        } else {
            stats.byte_count += metadata.len()
        }
    }
    Ok(stats)
}
fn valid_job_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
