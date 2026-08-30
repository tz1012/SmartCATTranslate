use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use tokio::sync::{oneshot, Semaphore};

use super::{AppIdentity, Blocklist, ForegroundAppProvider, PlatformError};

pub const MAX_CLIPBOARD_FORMATS: usize = 128;
pub const MAX_CLIPBOARD_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_SELECTION_BYTES: usize = 4 * 1024 * 1024;
const MAX_CAPTURE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardFormatId {
    Text,
    Native(u32),
    Named(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardFormat {
    pub id: ClipboardFormatId,
    pub data: Vec<u8>,
}

impl ClipboardFormat {
    pub fn new(id: ClipboardFormatId, data: Vec<u8>) -> Result<Self, CaptureError> {
        if matches!(&id, ClipboardFormatId::Named(name) if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control))
        {
            return Err(CaptureError::InvalidFormat);
        }
        if data.len() > MAX_CLIPBOARD_SNAPSHOT_BYTES {
            return Err(CaptureError::ClipboardTooLarge);
        }
        Ok(Self { id, data })
    }
}

impl std::fmt::Debug for ClipboardFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardFormat")
            .field("id", &self.id)
            .field("byte_len", &self.data.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct ClipboardSnapshot {
    generation: u64,
    items: Vec<ClipboardFormat>,
}

impl std::fmt::Debug for ClipboardSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardSnapshot")
            .field("generation", &"redacted")
            .field("format_count", &self.items.len())
            .field(
                "byte_len",
                &self.items.iter().map(|item| item.data.len()).sum::<usize>(),
            )
            .finish()
    }
}

impl ClipboardSnapshot {
    pub fn new(generation: u64, items: Vec<ClipboardFormat>) -> Self {
        Self { generation, items }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn items(&self) -> &[ClipboardFormat] {
        &self.items
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClipboardLimits {
    pub max_formats: usize,
    pub max_bytes: usize,
    pub max_selection_bytes: usize,
}

impl Default for ClipboardLimits {
    fn default() -> Self {
        Self {
            max_formats: MAX_CLIPBOARD_FORMATS,
            max_bytes: MAX_CLIPBOARD_SNAPSHOT_BYTES,
            max_selection_bytes: MAX_SELECTION_BYTES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("clipboard access was denied")]
    ClipboardAccessDenied,
    #[error("clipboard content did not change")]
    ClipboardUnchanged,
    #[error("no selected text was available")]
    NoSelection,
    #[error("the copy request failed")]
    CopyFailed,
    #[error("the clipboard snapshot exceeded its limit")]
    ClipboardTooLarge,
    #[error("the selected text exceeded its limit")]
    SelectionTooLarge,
    #[error("the clipboard format was invalid")]
    InvalidFormat,
    #[error("the original clipboard could not be restored")]
    RestoreFailed,
    #[error("the foreground application is blocked")]
    BlockedApplication { catalog_name: Option<String> },
    #[error("the foreground application could not be identified")]
    ForegroundUnavailable,
    #[error("the clipboard backend is unavailable")]
    BackendUnavailable,
}

impl CaptureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ClipboardAccessDenied => "clipboard_access_denied",
            Self::ClipboardUnchanged => "clipboard_unchanged",
            Self::NoSelection => "no_selection",
            Self::CopyFailed => "copy_failed",
            Self::ClipboardTooLarge => "clipboard_too_large",
            Self::SelectionTooLarge => "selection_too_large",
            Self::InvalidFormat => "invalid_clipboard_format",
            Self::RestoreFailed => "clipboard_restore_failed",
            Self::BlockedApplication { .. } => "blocked_application",
            Self::ForegroundUnavailable => "foreground_unavailable",
            Self::BackendUnavailable => "clipboard_backend_unavailable",
        }
    }
}

impl From<PlatformError> for CaptureError {
    fn from(_: PlatformError) -> Self {
        Self::ForegroundUnavailable
    }
}

pub trait ClipboardPort: Send + Sync + 'static {
    fn snapshot(&self, limits: ClipboardLimits) -> Result<ClipboardSnapshot, CaptureError>;
    fn generation(&self) -> Result<u64, CaptureError>;
    fn read_plain_text(&self, max_bytes: usize) -> Result<Option<String>, CaptureError>;
    /// Restores while holding the platform clipboard lock only if the current
    /// generation is still the synthetic copy owned by this transaction.
    fn conditional_restore(
        &self,
        expected_generation: u64,
        snapshot: &ClipboardSnapshot,
    ) -> Result<bool, CaptureError>;
}

pub trait CopySynthesizer: Send + Sync + 'static {
    /// Performs the bounded native input transaction synchronously so task
    /// cancellation cannot split input submission from generation ownership.
    fn synthesize_copy(&self) -> Result<(), CaptureError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreStatus {
    Restored,
    SkippedConcurrentChange,
    NotNeeded,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CapturedSelection {
    pub text: String,
    pub restore_status: RestoreStatus,
}

impl std::fmt::Debug for CapturedSelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapturedSelection")
            .field("text_byte_len", &self.text.len())
            .field("restore_status", &self.restore_status)
            .finish()
    }
}

pub struct ClipboardGuard<C, S> {
    clipboard: Arc<C>,
    copier: Arc<S>,
    serial: Arc<Semaphore>,
    limits: ClipboardLimits,
}

impl<C, S> ClipboardGuard<C, S>
where
    C: ClipboardPort,
    S: CopySynthesizer,
{
    pub fn new(clipboard: Arc<C>, copier: Arc<S>) -> Self {
        Self {
            clipboard,
            copier,
            serial: Arc::new(Semaphore::new(1)),
            limits: ClipboardLimits::default(),
        }
    }

    pub async fn capture_selected_text(
        &self,
        timeout: Duration,
        trigger_already_copied: bool,
    ) -> Result<CapturedSelection, CaptureError> {
        let permit = Arc::clone(&self.serial)
            .acquire_owned()
            .await
            .map_err(|_| CaptureError::BackendUnavailable)?;
        let clipboard = Arc::clone(&self.clipboard);
        let copier = Arc::clone(&self.copier);
        let limits = self.limits;
        let (result_sender, result_receiver) = oneshot::channel();
        thread::Builder::new()
            .name("smartcat-clipboard-capture".to_owned())
            .spawn(move || {
                // The permit is deliberately owned by this worker. Dropping
                // the calling future cannot let a second capture overlap the
                // pending OS copy or its conditional restoration.
                let _permit = permit;
                let result = Self::capture_selected_text_blocking(
                    &clipboard,
                    &copier,
                    limits,
                    timeout,
                    trigger_already_copied,
                );
                let _ = result_sender.send(result);
            })
            .map_err(|_| CaptureError::BackendUnavailable)?;
        result_receiver
            .await
            .map_err(|_| CaptureError::BackendUnavailable)?
    }

    fn capture_selected_text_blocking(
        clipboard: &Arc<C>,
        copier: &Arc<S>,
        limits: ClipboardLimits,
        timeout: Duration,
        trigger_already_copied: bool,
    ) -> Result<CapturedSelection, CaptureError> {
        if trigger_already_copied {
            return Self::read_without_copy(clipboard, limits);
        }

        let snapshot = clipboard.snapshot(limits)?;
        let mut restoration = Restoration::new(Arc::clone(clipboard), snapshot);
        let copy_result = copier.synthesize_copy();
        let generation_after_input = clipboard.generation()?;
        if generation_after_input != restoration.snapshot.generation() {
            restoration.owned_generation = Some(generation_after_input);
        }
        if let Err(error) = copy_result {
            return Err(restoration.finish_error(error));
        }
        let timeout = timeout.min(MAX_CAPTURE_TIMEOUT);
        let deadline = Instant::now() + timeout;
        let captured_generation = loop {
            let generation = match clipboard.generation() {
                Ok(generation) => generation,
                Err(error) => return Err(restoration.finish_error(error)),
            };
            if generation != restoration.snapshot.generation() {
                break generation;
            }
            if Instant::now() >= deadline {
                return Err(restoration.finish_error(CaptureError::ClipboardUnchanged));
            }
            thread::sleep(POLL_INTERVAL.min(timeout));
        };
        restoration.owned_generation = Some(captured_generation);
        let text = match clipboard.read_plain_text(limits.max_selection_bytes) {
            Ok(text) => text,
            Err(error) => return Err(restoration.finish_error(error)),
        }
        .filter(|value| !value.trim().is_empty())
        .ok_or(CaptureError::NoSelection);
        let text = match text {
            Ok(text) => text,
            Err(error) => return Err(restoration.finish_error(error)),
        };

        let restore_status = restoration.finish()?;
        Ok(CapturedSelection {
            text,
            restore_status,
        })
    }

    fn read_without_copy(
        clipboard: &Arc<C>,
        limits: ClipboardLimits,
    ) -> Result<CapturedSelection, CaptureError> {
        let text = clipboard
            .read_plain_text(limits.max_selection_bytes)?
            .filter(|value| !value.trim().is_empty())
            .ok_or(CaptureError::NoSelection)?;
        Ok(CapturedSelection {
            text,
            restore_status: RestoreStatus::NotNeeded,
        })
    }
}

struct Restoration<C: ClipboardPort> {
    clipboard: Arc<C>,
    snapshot: ClipboardSnapshot,
    owned_generation: Option<u64>,
    finished: bool,
}

impl<C: ClipboardPort> Restoration<C> {
    fn new(clipboard: Arc<C>, snapshot: ClipboardSnapshot) -> Self {
        Self {
            clipboard,
            snapshot,
            owned_generation: None,
            finished: false,
        }
    }

    fn finish(&mut self) -> Result<RestoreStatus, CaptureError> {
        let status = self.restore_if_still_owned()?;
        self.finished = true;
        Ok(status)
    }

    fn finish_error(&mut self, original: CaptureError) -> CaptureError {
        match self.finish() {
            Ok(_) => original,
            Err(restore_error) => restore_error,
        }
    }

    fn restore_if_still_owned(&self) -> Result<RestoreStatus, CaptureError> {
        let Some(owned_generation) = self.owned_generation else {
            return Ok(RestoreStatus::NotNeeded);
        };
        if !self
            .clipboard
            .conditional_restore(owned_generation, &self.snapshot)?
        {
            return Ok(RestoreStatus::SkippedConcurrentChange);
        }
        Ok(RestoreStatus::Restored)
    }
}

impl<C: ClipboardPort> Drop for Restoration<C> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.restore_if_still_owned();
        }
    }
}

pub struct SelectedTextAcquirer<F, C, S> {
    foreground: Arc<F>,
    blocklist: Blocklist,
    guard: ClipboardGuard<C, S>,
}

impl<F, C, S> SelectedTextAcquirer<F, C, S>
where
    F: ForegroundAppProvider + 'static,
    C: ClipboardPort,
    S: CopySynthesizer,
{
    pub fn new(foreground: Arc<F>, blocklist: Blocklist, guard: ClipboardGuard<C, S>) -> Self {
        Self {
            foreground,
            blocklist,
            guard,
        }
    }

    pub async fn capture_selected_text(
        &self,
        timeout: Duration,
        trigger_already_copied: bool,
    ) -> Result<CapturedSelection, CaptureError> {
        let app: AppIdentity = self.foreground.current()?;
        if let Some(entry) = self.blocklist.blocking_entry(&app) {
            return Err(CaptureError::BlockedApplication {
                catalog_name: entry.catalog_name().map(str::to_owned),
            });
        }
        self.guard
            .capture_selected_text(timeout, trigger_already_copied)
            .await
    }
}
