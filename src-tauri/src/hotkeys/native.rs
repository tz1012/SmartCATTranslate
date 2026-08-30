use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use uuid::Uuid;

use super::{AppIdentity, KeyEvent, SequenceEngine, SequenceEngineError};

const KEY_EVENT_QUEUE_CAPACITY: usize = 128;
const ACTIVATION_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverAvailability {
    Available,
    AccessibilityPermissionRequired,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    #[error("keyboard observation is already running")]
    AlreadyRunning,
    #[error("accessibility permission is required")]
    AccessibilityPermissionRequired,
    #[error("operating system permission was denied")]
    PermissionDenied,
    #[error("keyboard event queue is full")]
    EventQueueFull,
    #[error("foreground application identity is unavailable")]
    IdentityUnavailable,
    #[error("platform backend is unavailable")]
    BackendUnavailable,
    #[error("the key cannot be represented by this platform")]
    InvalidKey,
    #[error("platform resource shutdown failed")]
    ShutdownFailed,
    #[error("keyboard observer was disabled and could not be reactivated")]
    ObserverDisabled,
}

pub(crate) struct ObserverActivationGuard<'a> {
    active: &'a AtomicBool,
    committed: bool,
}

pub(crate) struct ObserverExitHandshake(AtomicU8);

impl ObserverExitHandshake {
    const OWNED: u8 = 0;
    const RELEASE_ON_EXIT: u8 = 1;
    const EXITED: u8 = 2;

    pub(crate) fn new() -> Self {
        Self(AtomicU8::new(Self::OWNED))
    }

    pub(crate) fn request_release(&self, active: &AtomicBool) {
        match self.0.compare_exchange(
            Self::OWNED,
            Self::RELEASE_ON_EXIT,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(Self::RELEASE_ON_EXIT) => {}
            Err(Self::EXITED) => active.store(false, Ordering::Release),
            Err(_) => unreachable!("observer exit handshake has an invalid state"),
        }
    }

    pub(crate) fn mark_exited(&self, active: &AtomicBool) {
        match self.0.compare_exchange(
            Self::OWNED,
            Self::EXITED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(Self::EXITED) => {}
            Err(Self::RELEASE_ON_EXIT) => {
                self.0.store(Self::EXITED, Ordering::Release);
                active.store(false, Ordering::Release);
            }
            Err(_) => unreachable!("observer exit handshake has an invalid state"),
        }
    }
}

impl<'a> ObserverActivationGuard<'a> {
    pub(crate) fn acquire(active: &'a AtomicBool) -> Result<Self, PlatformError> {
        if active.swap(true, Ordering::AcqRel) {
            return Err(PlatformError::AlreadyRunning);
        }
        Ok(Self {
            active,
            committed: false,
        })
    }

    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ObserverActivationGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.active.store(false, Ordering::Release);
        }
    }
}

pub trait HotkeyObserver: Send {
    /// Stops producing events before returning, including error returns, so
    /// the controller can always join its worker without leaking a thread.
    fn stop(&mut self) -> Result<(), PlatformError>;
}

pub trait KeyEventSource: Send + Sync + 'static {
    fn availability(&self) -> ObserverAvailability;

    /// Starts one listen-only observer. Implementations must use `try_send`
    /// from native callbacks so a full bounded queue never blocks input.
    fn start(
        &self,
        sender: mpsc::SyncSender<KeyEvent>,
    ) -> Result<Box<dyn HotkeyObserver>, PlatformError>;
}

pub trait ForegroundAppProvider: Send + Sync {
    /// Returns only a sanitized executable basename and/or a bundle ID.
    /// Window titles, paths, and selected text are outside this contract.
    fn current(&self) -> Result<AppIdentity, PlatformError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NativeControllerError {
    #[error("platform observer could not start")]
    Platform(#[from] PlatformError),
    #[error("sequence engine rejected an event")]
    Sequence(#[from] SequenceEngineError),
    #[error("native event worker stopped unexpectedly")]
    WorkerStopped,
}

pub type NativeEventReceiver = mpsc::Receiver<Vec<Uuid>>;

pub struct NativeController {
    observer: Mutex<Option<Box<dyn HotkeyObserver>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown: Arc<AtomicBool>,
}

impl NativeController {
    pub fn start<S: KeyEventSource>(
        source: S,
        mut engine: SequenceEngine,
    ) -> Result<(Self, NativeEventReceiver), NativeControllerError> {
        let (event_sender, event_receiver) = mpsc::sync_channel(KEY_EVENT_QUEUE_CAPACITY);
        let observer = source.start(event_sender)?;
        let (activation_sender, activation_receiver) =
            mpsc::sync_channel(ACTIVATION_QUEUE_CAPACITY);
        let origin = Instant::now();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = thread::Builder::new()
            .name("smartcat-hotkey-engine".to_owned())
            .spawn(move || {
                while !worker_shutdown.load(Ordering::Acquire) {
                    let event = match event_receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(event) => event,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let Ok(outcome) = engine.on_event(event, origin.elapsed()) else {
                        break;
                    };
                    if !outcome.binding_ids.is_empty()
                        && activation_sender.try_send(outcome.binding_ids).is_err()
                    {
                        // Activations are advisory wakeups. Never block the native
                        // input callback or grow memory when the UI is stalled.
                        continue;
                    }
                }
            })
            .map_err(|_| PlatformError::BackendUnavailable)?;

        Ok((
            Self {
                observer: Mutex::new(Some(observer)),
                worker: Mutex::new(Some(worker)),
                shutdown,
            },
            activation_receiver,
        ))
    }

    pub fn stop(&self) -> Result<(), NativeControllerError> {
        self.shutdown.store(true, Ordering::Release);
        let mut shutdown_error = None;
        let observer = self
            .observer
            .lock()
            .map_err(|_| PlatformError::ShutdownFailed)?
            .take();
        if let Some(mut observer) = observer {
            if let Err(error) = observer.stop() {
                shutdown_error = Some(NativeControllerError::Platform(error));
            }
            drop(observer);
        }
        if let Some(worker) = self
            .worker
            .lock()
            .map_err(|_| PlatformError::ShutdownFailed)?
            .take()
        {
            if worker.join().is_err() && shutdown_error.is_none() {
                shutdown_error = Some(NativeControllerError::WorkerStopped);
            }
        }
        if let Some(error) = shutdown_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

impl Drop for NativeController {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        time::{Duration, Instant},
    };

    use uuid::Uuid;

    use super::{
        ForegroundAppProvider, HotkeyObserver, KeyEventSource, NativeController,
        NativeControllerError, ObserverAvailability, PlatformError,
    };
    use crate::hotkeys::{
        parse_trigger, AppIdentity, HotkeyAction, HotkeyBinding, KeyDevice, KeyEvent,
        KeyEventPhase, SequenceEngine, Trigger,
    };

    struct FakeSource {
        events: Vec<KeyEvent>,
    }

    struct FakeObserver;

    struct ErrorStopSource;

    struct ErrorStopObserver {
        sender: Option<mpsc::SyncSender<KeyEvent>>,
    }

    impl HotkeyObserver for FakeObserver {
        fn stop(&mut self) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    impl HotkeyObserver for ErrorStopObserver {
        fn stop(&mut self) -> Result<(), PlatformError> {
            self.sender.take();
            Err(PlatformError::ShutdownFailed)
        }
    }

    impl KeyEventSource for ErrorStopSource {
        fn availability(&self) -> ObserverAvailability {
            ObserverAvailability::Available
        }

        fn start(
            &self,
            sender: mpsc::SyncSender<KeyEvent>,
        ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
            Ok(Box::new(ErrorStopObserver {
                sender: Some(sender),
            }))
        }
    }

    impl KeyEventSource for FakeSource {
        fn availability(&self) -> ObserverAvailability {
            ObserverAvailability::Available
        }

        fn start(
            &self,
            sender: mpsc::SyncSender<KeyEvent>,
        ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
            for event in &self.events {
                sender
                    .try_send(*event)
                    .map_err(|_| PlatformError::EventQueueFull)?;
            }
            Ok(Box::new(FakeObserver))
        }
    }

    struct FakeForeground;

    impl ForegroundAppProvider for FakeForeground {
        fn current(&self) -> Result<AppIdentity, PlatformError> {
            AppIdentity::new(Some("editor.exe"), Some("com.example.editor"))
                .ok_or(PlatformError::IdentityUnavailable)
        }
    }

    fn binding(id: Uuid) -> HotkeyBinding {
        HotkeyBinding {
            id,
            trigger: parse_trigger("Ctrl+C, C").unwrap(),
            action: HotkeyAction::TranslateSelection,
            profile_id: Uuid::nil(),
            force: false,
        }
    }

    fn event(trigger: &str, at_up: bool) -> KeyEvent {
        let (parsed, remove_test_modifier) = if trigger.contains('+') {
            (parse_trigger(trigger).unwrap(), false)
        } else {
            (parse_trigger(&format!("Ctrl+{trigger}")).unwrap(), true)
        };
        let Trigger::Chord { mut chord } = parsed else {
            panic!("expected chord")
        };
        if remove_test_modifier {
            chord.modifiers.ctrl = false;
        }
        KeyEvent {
            chord,
            phase: if at_up {
                KeyEventPhase::Up
            } else {
                KeyEventPhase::Down
            },
            repeat: false,
            device: KeyDevice(0),
        }
    }

    #[test]
    fn fake_source_drives_one_sequence_match_and_foreground_identity_is_sanitized() {
        let id = Uuid::from_u128(44);
        let source = FakeSource {
            events: vec![
                event("Ctrl+C", false),
                event("Ctrl+C", true),
                event("C", false),
            ],
        };
        let engine = SequenceEngine::new(vec![binding(id)]).unwrap();
        let (controller, receiver) = NativeController::start(source, engine).unwrap();

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![id]
        );
        assert_eq!(
            FakeForeground.current().unwrap(),
            AppIdentity::new(Some("EDITOR.EXE"), Some("com.example.editor")).unwrap()
        );
        controller.stop().unwrap();
    }

    #[test]
    fn controller_uses_a_bounded_queue_and_stops_exactly_once() {
        let engine = SequenceEngine::new(Vec::<HotkeyBinding>::new()).unwrap();
        let source = FakeSource { events: Vec::new() };
        let (controller, _receiver) = NativeController::start(source, engine).unwrap();

        controller.stop().unwrap();
        controller.stop().unwrap();
    }

    #[test]
    fn controller_joins_worker_even_when_platform_shutdown_reports_an_error() {
        let engine = SequenceEngine::new(Vec::<HotkeyBinding>::new()).unwrap();
        let (controller, _receiver) = NativeController::start(ErrorStopSource, engine).unwrap();

        assert_eq!(
            controller.stop(),
            Err(NativeControllerError::Platform(
                PlatformError::ShutdownFailed
            ))
        );
        assert!(controller.worker.lock().unwrap().is_none());
        assert!(controller.observer.lock().unwrap().is_none());
    }

    #[test]
    fn controller_shutdown_is_bounded_even_if_observer_keeps_sender_alive() {
        struct StuckObserver(mpsc::SyncSender<KeyEvent>);
        impl HotkeyObserver for StuckObserver {
            fn stop(&mut self) -> Result<(), PlatformError> {
                let _ = &self.0;
                Err(PlatformError::ShutdownFailed)
            }
        }
        struct Source;
        impl KeyEventSource for Source {
            fn availability(&self) -> ObserverAvailability {
                ObserverAvailability::Available
            }
            fn start(
                &self,
                sender: mpsc::SyncSender<KeyEvent>,
            ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
                Ok(Box::new(StuckObserver(sender)))
            }
        }

        let engine = SequenceEngine::new(Vec::<HotkeyBinding>::new()).unwrap();
        let (controller, _receiver) = NativeController::start(Source, engine).unwrap();
        let started = Instant::now();
        assert_eq!(
            controller.stop(),
            Err(NativeControllerError::Platform(
                PlatformError::ShutdownFailed
            ))
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn observer_activation_rolls_back_after_startup_failure() {
        let active = AtomicBool::new(false);
        {
            let _guard = super::ObserverActivationGuard::acquire(&active).unwrap();
            assert!(active.load(Ordering::Acquire));
        }
        assert!(!active.load(Ordering::Acquire));
        let guard = super::ObserverActivationGuard::acquire(&active).unwrap();
        guard.commit();
        assert!(active.load(Ordering::Acquire));
        // A timed-out worker owns this final transition. Until it executes,
        // a newer singleton must remain blocked.
        assert!(matches!(
            super::ObserverActivationGuard::acquire(&active),
            Err(PlatformError::AlreadyRunning)
        ));
        active.store(false, Ordering::Release);
        assert!(super::ObserverActivationGuard::acquire(&active).is_ok());
    }

    #[test]
    fn observer_exit_handshake_releases_active_in_both_race_orders() {
        let active = AtomicBool::new(true);
        let worker_first = super::ObserverExitHandshake::new();
        worker_first.mark_exited(&active);
        assert!(active.load(Ordering::Acquire));
        worker_first.request_release(&active);
        assert!(!active.load(Ordering::Acquire));

        active.store(true, Ordering::Release);
        let timeout_first = super::ObserverExitHandshake::new();
        timeout_first.request_release(&active);
        assert!(active.load(Ordering::Acquire));
        timeout_first.mark_exited(&active);
        assert!(!active.load(Ordering::Acquire));
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn platform_contracts_are_thread_safe() {
        assert_send_sync::<Arc<dyn KeyEventSource>>();
        assert_send_sync::<Arc<dyn ForegroundAppProvider>>();
    }
}
