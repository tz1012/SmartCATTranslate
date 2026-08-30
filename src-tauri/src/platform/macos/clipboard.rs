use async_trait::async_trait;

use crate::hotkeys::{
    CaptureError, ClipboardLimits, ClipboardPort, ClipboardSnapshot, CopySynthesizer,
};

/// AppKit pasteboard support remains fail-closed until the native multi-format
/// item adapter is exercised on both macOS CI architectures. This prevents a
/// text-only fallback from destroying rich clipboard contents.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacClipboard;

#[derive(Clone, Copy, Debug, Default)]
pub struct MacCopySynthesizer;

impl ClipboardPort for MacClipboard {
    fn snapshot(&self, _limits: ClipboardLimits) -> Result<ClipboardSnapshot, CaptureError> {
        Err(CaptureError::BackendUnavailable)
    }

    fn generation(&self) -> Result<u64, CaptureError> {
        Err(CaptureError::BackendUnavailable)
    }

    fn read_plain_text(&self, _max_bytes: usize) -> Result<Option<String>, CaptureError> {
        Err(CaptureError::BackendUnavailable)
    }

    fn restore(&self, _snapshot: &ClipboardSnapshot) -> Result<(), CaptureError> {
        Err(CaptureError::RestoreFailed)
    }
}

#[async_trait]
impl CopySynthesizer for MacCopySynthesizer {
    async fn synthesize_copy(&self) -> Result<(), CaptureError> {
        Err(CaptureError::CopyFailed)
    }
}
