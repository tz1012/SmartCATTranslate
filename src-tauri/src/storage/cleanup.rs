use crate::core::diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, SystemTime},
};

const PENDING_FILE: &str = ".cleanup-pending.json";
const PENDING_TEMP_FILE: &str = ".cleanup-pending.json.tmp";
const MAX_PENDING_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct CleanupService {
    root: PathBuf,
    pending_path: PathBuf,
    shared: Arc<Mutex<CleanupSharedState>>,
}

struct CleanupSharedState {
    pending: HashSet<String>,
    metadata_error: bool,
}

type SharedCleanupState = Mutex<HashMap<PathBuf, Weak<Mutex<CleanupSharedState>>>>;
static SHARED_STATES: OnceLock<SharedCleanupState> = OnceLock::new();

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
        let registry = SHARED_STATES.get_or_init(|| Mutex::new(HashMap::new()));
        let mut registry = registry.lock().unwrap_or_else(|value| value.into_inner());
        if let Some(shared) = registry.get(&root).and_then(Weak::upgrade) {
            return Ok(Self {
                root,
                pending_path,
                shared,
            });
        }
        let (pending, metadata_error) = recover_pending_metadata(
            &root,
            load_pending(&pending_path),
            load_pending(&root.join(PENDING_TEMP_FILE)),
        )?;
        let shared = Arc::new(Mutex::new(CleanupSharedState {
            pending,
            metadata_error,
        }));
        registry.insert(root.clone(), Arc::downgrade(&shared));
        Ok(Self {
            root,
            pending_path,
            shared,
        })
    }

    pub fn on_start(&self) -> Result<CleanupStats, CleanupError> {
        let mut stats = CleanupStats::default();
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        let mut failed = state.metadata_error;
        let pending = state.pending.iter().cloned().collect::<Vec<_>>();
        for job_id in pending {
            match self.remove_job_raw(&job_id) {
                Ok(removed) => {
                    stats.include(removed);
                    state.pending.remove(&job_id);
                }
                Err(_) => failed = true,
            }
        }
        match self.purge_internal(Duration::from_secs(7 * 24 * 60 * 60), &mut state.pending) {
            Ok(removed) => stats.include(removed),
            Err(_) => failed = true,
        }
        if self.persist_pending(&mut state).is_err() {
            failed = true;
        }
        if failed || !state.pending.is_empty() {
            state.metadata_error = true;
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
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        let cleanup_result = self.purge_internal(maximum_age, &mut state.pending);
        let persistence_result = self.persist_pending(&mut state);
        match (cleanup_result, persistence_result) {
            (Ok(stats), Ok(())) if state.pending.is_empty() => {
                self.emit_succeeded(stats);
                Ok(stats)
            }
            _ => {
                self.emit_failed();
                Err(CleanupError::Unavailable)
            }
        }
    }

    pub fn has_pending(&self) -> bool {
        let state = self
            .shared
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        state.metadata_error || !state.pending.is_empty()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn finish_cleanup(&self, job_id: &str) -> Result<CleanupStats, CleanupError> {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        match self.remove_job_raw(job_id) {
            Ok(stats) => {
                state.pending.remove(job_id);
                if self.persist_pending(&mut state).is_err() {
                    self.emit_failed();
                    return Err(CleanupError::Unavailable);
                }
                self.emit_succeeded(stats);
                Ok(stats)
            }
            Err(error) => {
                if valid_job_id(job_id) {
                    state.pending.insert(job_id.to_owned());
                    let _ = self.persist_pending(&mut state);
                }
                self.emit_failed();
                Err(error)
            }
        }
    }

    fn purge_internal(
        &self,
        maximum_age: Duration,
        pending: &mut HashSet<String>,
    ) -> Result<CleanupStats, CleanupError> {
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
                    pending.insert(name.to_owned());
                    failed = true;
                    continue;
                }
            };
            if is_link_or_reparse(&metadata) || !metadata.is_dir() {
                pending.insert(name.to_owned());
                failed = true;
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(modified).unwrap_or_default() > maximum_age {
                match self.remove_job_raw(name) {
                    Ok(removed) => stats.include(removed),
                    Err(_) => {
                        pending.insert(name.to_owned());
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

    fn persist_pending(&self, state: &mut CleanupSharedState) -> Result<(), CleanupError> {
        let mut pending = state.pending.iter().cloned().collect::<Vec<_>>();
        pending.sort();
        let temporary_path = self.root.join(PENDING_TEMP_FILE);
        let mut ready_for_recovery = false;
        let result = (|| {
            let encoded = serde_json::to_vec(&pending).map_err(|_| CleanupError::Unavailable)?;
            remove_safe_regular_file(&temporary_path)?;
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)
                .map_err(|_| CleanupError::Unavailable)?;
            output
                .write_all(&encoded)
                .and_then(|_| output.flush())
                .and_then(|_| output.sync_all())
                .map_err(|_| CleanupError::Unavailable)?;
            drop(output);
            ready_for_recovery = true;
            atomic_replace(&temporary_path, &self.pending_path)?;
            let _ = File::open(&self.root).and_then(|directory| directory.sync_all());
            Ok(())
        })();
        if result.is_err() && !ready_for_recovery {
            let _ = remove_safe_regular_file(&temporary_path);
        }
        state.metadata_error = result.is_err();
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

fn load_pending(path: &Path) -> Result<Option<HashSet<String>>, CleanupError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    Ok(Some(values.into_iter().collect()))
}

fn recover_pending_metadata(
    root: &Path,
    primary: Result<Option<HashSet<String>>, CleanupError>,
    temporary: Result<Option<HashSet<String>>, CleanupError>,
) -> Result<(HashSet<String>, bool), CleanupError> {
    let mut pending = scan_uuid_job_roots(root)?;
    let mut metadata_error = false;
    match primary {
        Ok(Some(values)) => pending.extend(values),
        Ok(None) => {}
        Err(_) => metadata_error = true,
    }
    match temporary {
        Ok(Some(values)) => {
            pending.extend(values);
            metadata_error = true;
        }
        Ok(None) => {}
        Err(_) => metadata_error = true,
    }
    Ok((pending, metadata_error))
}

fn scan_uuid_job_roots(root: &Path) -> Result<HashSet<String>, CleanupError> {
    let mut pending = HashSet::new();
    for entry in fs::read_dir(root).map_err(|_| CleanupError::Unavailable)? {
        let entry = entry.map_err(|_| CleanupError::Unavailable)?;
        if let Some(name) = entry
            .file_name()
            .to_str()
            .filter(|value| valid_job_id(value))
        {
            pending.insert(name.to_owned());
        }
    }
    Ok(pending)
}

fn remove_safe_regular_file(path: &Path) -> Result<(), CleanupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_file() => {
            Err(CleanupError::OutsideRoot)
        }
        Ok(_) => fs::remove_file(path).map_err(|_| CleanupError::Unavailable),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CleanupError::Unavailable),
    }
}

fn validate_replace_target(path: &Path) -> Result<(), CleanupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_file() => {
            Err(CleanupError::OutsideRoot)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CleanupError::Unavailable),
    }
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), CleanupError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    validate_replace_target(destination)?;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(CleanupError::Unavailable);
    }
    Ok(())
}

#[cfg(unix)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), CleanupError> {
    validate_replace_target(destination)?;
    fs::rename(source, destination).map_err(|_| CleanupError::Unavailable)
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
