use crate::core::diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime},
};

const PENDING_FILE: &str = ".cleanup-pending.json";
const MAX_PENDING_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct CleanupService {
    root: PathBuf,
    pending_path: PathBuf,
    pending: Arc<Mutex<HashSet<String>>>,
    metadata_error: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CleanupStats {
    pub item_count: u64,
    pub byte_count: u64,
}

impl CleanupStats {
    fn include(&mut self, other: Self) {
        self.item_count = self.item_count.saturating_add(other.item_count);
        self.byte_count = self.byte_count.saturating_add(other.byte_count);
    }
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
        let pending_path = root.join(PENDING_FILE);
        let (pending, metadata_error) = match load_pending(&pending_path) {
            Ok(pending) => (pending, false),
            Err(_) => (HashSet::new(), true),
        };
        Ok(Self {
            root,
            pending_path,
            pending: Arc::new(Mutex::new(pending)),
            metadata_error: Arc::new(AtomicBool::new(metadata_error)),
        })
    }

    pub fn on_start(&self) -> Result<CleanupStats, CleanupError> {
        let mut stats = CleanupStats::default();
        let mut failed = self.metadata_error.load(Ordering::Acquire);
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for job_id in pending {
            match self.remove_job_raw(&job_id) {
                Ok(removed) => {
                    stats.include(removed);
                    self.pending
                        .lock()
                        .unwrap_or_else(|value| value.into_inner())
                        .remove(&job_id);
                }
                Err(_) => failed = true,
            }
        }
        match self.purge_internal(Duration::from_secs(7 * 24 * 60 * 60)) {
            Ok(removed) => stats.include(removed),
            Err(_) => failed = true,
        }
        if self.persist_pending().is_err() {
            failed = true;
        }
        if failed || self.has_pending() {
            self.metadata_error.store(true, Ordering::Release);
            self.emit_failed();
            return Err(CleanupError::Unavailable);
        }
        self.emit_succeeded(stats);
        Ok(stats)
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
        self.finish_cleanup(job_id)
    }

    pub fn on_job_cancel(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
        self.finish_cleanup(job_id)
    }

    pub fn purge(&self, maximum_age: Duration) -> Result<CleanupStats, CleanupError> {
        match self.purge_internal(maximum_age) {
            Ok(stats) if self.persist_pending().is_ok() && !self.has_pending() => {
                self.emit_succeeded(stats);
                Ok(stats)
            }
            Ok(_) | Err(_) => {
                self.emit_failed();
                Err(CleanupError::Unavailable)
            }
        }
    }

    pub fn has_pending(&self) -> bool {
        self.metadata_error.load(Ordering::Acquire)
            || !self
                .pending
                .lock()
                .unwrap_or_else(|value| value.into_inner())
                .is_empty()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn finish_cleanup(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
        match self.remove_job_raw(job_id) {
            Ok(stats) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|value| value.into_inner())
                    .remove(job_id);
                if self.persist_pending().is_err() {
                    self.emit_failed();
                    return Err(CleanupError::Unavailable);
                }
                self.emit_succeeded(stats);
                Ok(stats)
            }
            Err(error) => {
                if valid_job_id(job_id) {
                    self.pending
                        .lock()
                        .unwrap_or_else(|value| value.into_inner())
                        .insert(job_id.to_owned());
                    let _ = self.persist_pending();
                }
                self.emit_failed();
                Err(error)
            }
        }
    }

    fn purge_internal(&self, maximum_age: Duration) -> Result<CleanupStats, CleanupError> {
        let mut stats = CleanupStats::default();
        let mut failed = false;
        let now = SystemTime::now();
        for entry in fs::read_dir(&self.root).map_err(|_| CleanupError::Unavailable)? {
            let entry = entry.map_err(|_| CleanupError::Unavailable)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !valid_job_id(name) {
                continue;
            }
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(_) => {
                    self.schedule(name);
                    failed = true;
                    continue;
                }
            };
            if is_link_or_reparse(&metadata) || !metadata.is_dir() {
                self.schedule(name);
                failed = true;
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(modified).unwrap_or_default() > maximum_age {
                match self.remove_job_raw(name) {
                    Ok(removed) => stats.include(removed),
                    Err(_) => {
                        self.schedule(name);
                        failed = true;
                    }
                }
            }
        }
        if failed {
            Err(CleanupError::Unavailable)
        } else {
            Ok(stats)
        }
    }

    fn schedule(&self, job_id: &str) {
        if valid_job_id(job_id) {
            self.pending
                .lock()
                .unwrap_or_else(|value| value.into_inner())
                .insert(job_id.to_owned());
        }
    }

    fn persist_pending(&self) -> Result<(), CleanupError> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        pending.sort();
        let result = (|| {
            let encoded = serde_json::to_vec(&pending).map_err(|_| CleanupError::Unavailable)?;
            let mut output = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&self.pending_path)
                .map_err(|_| CleanupError::Unavailable)?;
            output
                .write_all(&encoded)
                .and_then(|_| output.sync_all())
                .map_err(|_| CleanupError::Unavailable)
        })();
        self.metadata_error
            .store(result.is_err(), Ordering::Release);
        result
    }

    fn remove_job_raw(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
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
        Ok(stats)
    }

    fn emit_succeeded(&self, stats: CleanupStats) {
        DiagnosticEvent::new(
            DiagnosticEventName::TemporaryCleanup,
            DiagnosticOutcome::Succeeded,
        )
        .with_counts(stats.item_count, stats.byte_count)
        .emit();
    }

    fn emit_failed(&self) {
        DiagnosticEvent::new(
            DiagnosticEventName::TemporaryCleanup,
            DiagnosticOutcome::Failed,
        )
        .with_error_code("temporary_cleanup_pending")
        .emit();
    }
}

fn load_pending(path: &Path) -> Result<HashSet<String>, CleanupError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(_) => return Err(CleanupError::Unavailable),
    };
    if is_link_or_reparse(&metadata) || !metadata.is_file() || metadata.len() > MAX_PENDING_BYTES {
        return Err(CleanupError::Unavailable);
    }
    let values: Vec<String> =
        serde_json::from_slice(&fs::read(path).map_err(|_| CleanupError::Unavailable)?)
            .map_err(|_| CleanupError::Unavailable)?;
    if values.iter().any(|value| !valid_job_id(value)) {
        return Err(CleanupError::Unavailable);
    }
    Ok(values.into_iter().collect())
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
        stats.item_count = stats.item_count.saturating_add(1);
        if metadata.is_dir() {
            stats.include(measure_confined(root, &canonical)?);
        } else {
            stats.byte_count = stats.byte_count.saturating_add(metadata.len());
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
