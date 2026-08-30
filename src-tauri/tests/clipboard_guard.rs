use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use smartcat_translate::hotkeys::{
    AppIdentity, BlockedApp, Blocklist, CaptureError, ClipboardFormat, ClipboardFormatId,
    ClipboardGuard, ClipboardLimits, ClipboardPort, ClipboardSnapshot, CopySynthesizer,
    ForegroundAppProvider, Platform, PlatformError, RestoreStatus, SelectedTextAcquirer,
};

#[derive(Clone)]
struct FakeClipboard {
    state: Arc<Mutex<FakeClipboardState>>,
}

#[derive(Clone)]
struct FakeClipboardState {
    generation: u64,
    items: Vec<ClipboardFormat>,
    access_error: bool,
    mutate_after_read: Option<Vec<ClipboardFormat>>,
    accesses: usize,
    restore_error: bool,
}

impl FakeClipboard {
    fn new(items: Vec<ClipboardFormat>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeClipboardState {
                generation: 1,
                items,
                access_error: false,
                mutate_after_read: None,
                accesses: 0,
                restore_error: false,
            })),
        }
    }

    fn replace(&self, items: Vec<ClipboardFormat>) {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        state.items = items;
    }

    fn items(&self) -> Vec<ClipboardFormat> {
        self.state.lock().unwrap().items.clone()
    }
}

impl ClipboardPort for FakeClipboard {
    fn snapshot(&self, limits: ClipboardLimits) -> Result<ClipboardSnapshot, CaptureError> {
        let mut state = self.state.lock().unwrap();
        state.accesses += 1;
        if state.access_error {
            return Err(CaptureError::ClipboardAccessDenied);
        }
        if state.items.len() > limits.max_formats
            || state
                .items
                .iter()
                .map(|item| item.data.len())
                .sum::<usize>()
                > limits.max_bytes
        {
            return Err(CaptureError::ClipboardTooLarge);
        }
        Ok(ClipboardSnapshot::new(
            state.generation,
            state.items.clone(),
        ))
    }

    fn generation(&self) -> Result<u64, CaptureError> {
        let mut state = self.state.lock().unwrap();
        state.accesses += 1;
        if state.access_error {
            Err(CaptureError::ClipboardAccessDenied)
        } else {
            Ok(state.generation)
        }
    }

    fn read_plain_text(&self, max_bytes: usize) -> Result<Option<String>, CaptureError> {
        let mut state = self.state.lock().unwrap();
        state.accesses += 1;
        let text = state.items.iter().find_map(|item| match item.id {
            ClipboardFormatId::Text => String::from_utf8(item.data.clone()).ok(),
            _ => None,
        });
        if text.as_ref().is_some_and(|text| text.len() > max_bytes) {
            return Err(CaptureError::SelectionTooLarge);
        }
        if let Some(items) = state.mutate_after_read.take() {
            state.generation += 1;
            state.items = items;
        }
        Ok(text)
    }

    fn conditional_restore(
        &self,
        expected_generation: u64,
        snapshot: &ClipboardSnapshot,
    ) -> Result<bool, CaptureError> {
        let mut state = self.state.lock().unwrap();
        state.accesses += 1;
        if state.generation != expected_generation {
            return Ok(false);
        }
        if state.restore_error {
            return Err(CaptureError::RestoreFailed);
        }
        state.generation += 1;
        state.items = snapshot.items().to_vec();
        Ok(true)
    }
}

#[derive(Clone)]
struct FakeCopier {
    clipboard: FakeClipboard,
    selected: Option<Vec<ClipboardFormat>>,
    fail: bool,
    delay: Duration,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl FakeCopier {
    fn selecting(clipboard: FakeClipboard, text: &str) -> Self {
        Self {
            clipboard,
            selected: Some(vec![text_item(text)]),
            fail: false,
            delay: Duration::ZERO,
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl CopySynthesizer for FakeCopier {
    fn synthesize_copy(&self) -> Result<(), CaptureError> {
        if self.fail {
            return Err(CaptureError::CopyFailed);
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        if let Some(items) = &self.selected {
            self.clipboard.replace(items.clone());
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

fn text_item(value: &str) -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::Text, value.as_bytes().to_vec()).unwrap()
}

fn html_item(value: &str) -> ClipboardFormat {
    ClipboardFormat::new(
        ClipboardFormatId::Named("text/html".into()),
        value.as_bytes().to_vec(),
    )
    .unwrap()
}

fn binary_item(id: u32, value: &[u8]) -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::Native(id), value.to_vec()).unwrap()
}

#[tokio::test]
async fn restores_text_html_and_binary_formats_after_capture() {
    let original = vec![
        html_item("<b>old</b>"),
        text_item("old"),
        binary_item(49301, &[0, 1, 2, 255]),
    ];
    let clipboard = FakeClipboard::new(original.clone());
    let copier = FakeCopier::selecting(clipboard.clone(), "selected text");

    let captured = ClipboardGuard::new(Arc::new(clipboard.clone()), Arc::new(copier))
        .capture_selected_text(Duration::from_millis(100), false)
        .await
        .unwrap();

    assert_eq!(captured.text, "selected text");
    assert_eq!(captured.restore_status, RestoreStatus::Restored);
    assert_eq!(clipboard.items(), original);
}

#[tokio::test]
async fn never_overwrites_a_clipboard_changed_by_another_owner() {
    let clipboard = FakeClipboard::new(vec![text_item("old")]);
    clipboard.state.lock().unwrap().mutate_after_read = Some(vec![text_item("user change")]);
    let copier = FakeCopier::selecting(clipboard.clone(), "selected");

    let captured = ClipboardGuard::new(Arc::new(clipboard.clone()), Arc::new(copier))
        .capture_selected_text(Duration::from_millis(100), false)
        .await
        .unwrap();

    assert_eq!(captured.text, "selected");
    assert_eq!(
        captured.restore_status,
        RestoreStatus::SkippedConcurrentChange
    );
    assert_eq!(clipboard.items(), vec![text_item("user change")]);
}

#[tokio::test]
async fn reports_timeout_blank_access_copy_and_restore_failures_without_details() {
    let unchanged = FakeClipboard::new(vec![text_item("old")]);
    let no_selection = FakeCopier {
        selected: None,
        ..FakeCopier::selecting(unchanged.clone(), "unused")
    };
    assert_eq!(
        ClipboardGuard::new(Arc::new(unchanged), Arc::new(no_selection))
            .capture_selected_text(Duration::from_millis(5), false)
            .await
            .unwrap_err(),
        CaptureError::ClipboardUnchanged
    );

    let blank = FakeClipboard::new(vec![text_item("old")]);
    let blank_copier = FakeCopier::selecting(blank.clone(), "   \n");
    assert_eq!(
        ClipboardGuard::new(Arc::new(blank), Arc::new(blank_copier))
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::NoSelection
    );

    let denied = FakeClipboard::new(vec![text_item("old")]);
    denied.state.lock().unwrap().access_error = true;
    let denied_copier = FakeCopier::selecting(denied.clone(), "selected");
    assert_eq!(
        ClipboardGuard::new(Arc::new(denied), Arc::new(denied_copier))
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::ClipboardAccessDenied
    );

    let copy_failure = FakeClipboard::new(vec![text_item("old")]);
    let failing_copier = FakeCopier {
        fail: true,
        ..FakeCopier::selecting(copy_failure.clone(), "selected")
    };
    assert_eq!(
        ClipboardGuard::new(Arc::new(copy_failure), Arc::new(failing_copier))
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::CopyFailed
    );

    let restore_failure = FakeClipboard::new(vec![text_item("old")]);
    let restore_copier = FakeCopier::selecting(restore_failure.clone(), "selected");
    restore_failure.state.lock().unwrap().restore_error = true;
    assert_eq!(
        ClipboardGuard::new(Arc::new(restore_failure), Arc::new(restore_copier))
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::RestoreFailed
    );
}

#[tokio::test]
async fn copy_step_can_be_skipped_when_the_trigger_already_copied() {
    let clipboard = FakeClipboard::new(vec![text_item("already selected")]);
    let copier = FakeCopier {
        fail: true,
        ..FakeCopier::selecting(clipboard.clone(), "must not be used")
    };

    let captured = ClipboardGuard::new(Arc::new(clipboard.clone()), Arc::new(copier))
        .capture_selected_text(Duration::from_millis(50), true)
        .await
        .unwrap();

    assert_eq!(captured.text, "already selected");
    assert_eq!(captured.restore_status, RestoreStatus::NotNeeded);
    assert_eq!(clipboard.items(), vec![text_item("already selected")]);
}

#[tokio::test]
async fn concurrent_requests_are_serialized_and_cancellation_restores_owned_clipboard() {
    let clipboard = FakeClipboard::new(vec![text_item("old")]);
    let copier = FakeCopier {
        delay: Duration::from_millis(20),
        ..FakeCopier::selecting(clipboard.clone(), "selected")
    };
    let max_active = Arc::clone(&copier.max_active);
    let guard = Arc::new(ClipboardGuard::new(
        Arc::new(clipboard.clone()),
        Arc::new(copier),
    ));
    let first = tokio::spawn({
        let guard = Arc::clone(&guard);
        async move {
            guard
                .capture_selected_text(Duration::from_millis(100), false)
                .await
        }
    });
    let second = tokio::spawn({
        let guard = Arc::clone(&guard);
        async move {
            guard
                .capture_selected_text(Duration::from_millis(100), false)
                .await
        }
    });
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(max_active.load(Ordering::SeqCst), 1);

    let cancel_clipboard = FakeClipboard::new(vec![text_item("before cancel")]);
    let cancel_copier = FakeCopier::selecting(cancel_clipboard.clone(), "temporary selection");
    let cancel_guard = Arc::new(ClipboardGuard::new(
        Arc::new(cancel_clipboard.clone()),
        Arc::new(cancel_copier),
    ));
    let task = tokio::spawn(async move {
        cancel_guard
            .capture_selected_text(Duration::from_secs(10), false)
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    task.abort();
    let _ = task.await;
    assert_eq!(cancel_clipboard.items(), vec![text_item("before cancel")]);
}

struct FakeForeground(AppIdentity);

impl ForegroundAppProvider for FakeForeground {
    fn current(&self) -> Result<AppIdentity, PlatformError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn blocklist_rejects_sanitized_identity_before_any_clipboard_access() {
    let clipboard = FakeClipboard::new(vec![text_item("secret")]);
    let copier = FakeCopier::selecting(clipboard.clone(), "secret");
    let blocklist = Blocklist::new(vec![BlockedApp::new(
        Platform::Windows,
        Some("KeePass.exe"),
        None,
        Some("Password manager"),
    )
    .unwrap()])
    .unwrap();
    let acquirer = SelectedTextAcquirer::new(
        Arc::new(FakeForeground(
            AppIdentity::new(Some("KEEPASS.EXE"), None).unwrap(),
        )),
        blocklist,
        ClipboardGuard::new(Arc::new(clipboard.clone()), Arc::new(copier)),
    );

    assert_eq!(
        acquirer
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::BlockedApplication {
            catalog_name: Some("Password manager".into())
        }
    );
    assert_eq!(clipboard.state.lock().unwrap().accesses, 0);
    assert!(BlockedApp::new(
        Platform::Windows,
        Some("C:\\Secrets\\KeePass.exe"),
        None,
        None,
    )
    .is_err());
}

#[tokio::test]
async fn format_count_snapshot_bytes_selection_bytes_and_timeout_are_bounded() {
    let many = FakeClipboard::new(
        (0..ClipboardLimits::default().max_formats + 1)
            .map(|id| binary_item(id as u32 + 100, &[1]))
            .collect(),
    );
    let copier = FakeCopier::selecting(many.clone(), "selected");
    assert_eq!(
        ClipboardGuard::new(Arc::new(many), Arc::new(copier))
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::ClipboardTooLarge
    );

    let old = FakeClipboard::new(vec![text_item("old")]);
    let large_selection = "x".repeat(ClipboardLimits::default().max_selection_bytes + 1);
    let copier = FakeCopier::selecting(old.clone(), &large_selection);
    assert_eq!(
        ClipboardGuard::new(Arc::new(old), Arc::new(copier))
            .capture_selected_text(Duration::from_millis(50), false)
            .await
            .unwrap_err(),
        CaptureError::SelectionTooLarge
    );

    let timeout = FakeClipboard::new(vec![text_item("old")]);
    let copier = FakeCopier {
        selected: None,
        ..FakeCopier::selecting(timeout.clone(), "unused")
    };
    let started = std::time::Instant::now();
    let _ = ClipboardGuard::new(Arc::new(timeout), Arc::new(copier))
        .capture_selected_text(Duration::from_secs(60), false)
        .await;
    assert!(started.elapsed() < Duration::from_secs(2));
}
