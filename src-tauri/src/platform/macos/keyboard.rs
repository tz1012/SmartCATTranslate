use std::{
    collections::HashSet,
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::hotkeys::{
    Chord, HotkeyObserver, KeyCode, KeyDevice, KeyEvent, KeyEventPhase, KeyEventSource, LogicalKey,
    Modifiers, ObserverActivationGuard, ObserverAvailability, ObserverExitHandshake, PhysicalKey,
    PlatformError, RegistrationProbe, RegistrationProbeStatus, Trigger,
};

type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFAllocatorRef = *const c_void;
type CFStringRef = *const c_void;

const CG_SESSION_EVENT_TAP: u32 = 1;
const CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
const CG_EVENT_KEY_DOWN: u32 = 10;
const CG_EVENT_KEY_UP: u32 = 11;
const CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
const CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;
const CG_KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
const CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
const CG_MASK_CONTROL: u64 = 1 << 18;
const CG_MASK_ALTERNATE: u64 = 1 << 19;
const CG_MASK_SHIFT: u64 = 1 << 17;
const CG_MASK_COMMAND: u64 = 1 << 20;
const KEY_EVENT_MASK: u64 = (1 << CG_EVENT_KEY_DOWN) | (1 << CG_EVENT_KEY_UP);

static OBSERVER_ACTIVE: AtomicBool = AtomicBool::new(false);
static OBSERVER_HEALTHY: AtomicBool = AtomicBool::new(false);
static OBSERVER_FAILURES: AtomicUsize = AtomicUsize::new(0);
static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static CALLBACK_RUN_LOOP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static EVENT_SENDER: OnceLock<Mutex<Option<mpsc::SyncSender<KeyEvent>>>> = OnceLock::new();
static HELD_KEYS: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();

const START_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const REENABLE_ATTEMPTS: usize = 3;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: Option<
            unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef,
        >,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(loop_ref: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRunLoopStop(loop_ref: CFRunLoopRef);
    fn CFRunLoopWakeUp(loop_ref: CFRunLoopRef);
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
}

#[derive(Default)]
pub struct MacRegistrationProbe;

impl MacRegistrationProbe {
    pub fn new() -> Self {
        Self
    }
}

impl RegistrationProbe for MacRegistrationProbe {
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus {
        if !trigger_is_representable(trigger) {
            return RegistrationProbeStatus::Invalid;
        }
        // macOS has no RegisterHotKey equivalent for these listen-only
        // sequences. Report observer capability without claiming ownership.
        let trusted = unsafe { AXIsProcessTrusted() };
        let observer_available = trusted && OBSERVER_HEALTHY.load(Ordering::Acquire);
        match trigger {
            Trigger::Sequence { .. } => {
                RegistrationProbeStatus::UnsupportedSequence { observer_available }
            }
            Trigger::Chord { .. } if observer_available => {
                RegistrationProbeStatus::AvailableViaObserver
            }
            Trigger::Chord { .. } if !trusted => RegistrationProbeStatus::PermissionDenied,
            Trigger::Chord { .. } => RegistrationProbeStatus::BackendError,
        }
    }
}

#[derive(Default)]
pub struct MacKeyEventSource;

impl MacKeyEventSource {
    pub fn new() -> Self {
        Self
    }
}

impl KeyEventSource for MacKeyEventSource {
    fn availability(&self) -> ObserverAvailability {
        if !unsafe { AXIsProcessTrusted() } {
            ObserverAvailability::AccessibilityPermissionRequired
        } else if OBSERVER_HEALTHY.load(Ordering::Acquire) {
            ObserverAvailability::Available
        } else {
            ObserverAvailability::Unsupported
        }
    }

    fn start(
        &self,
        sender: mpsc::SyncSender<KeyEvent>,
    ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(PlatformError::AccessibilityPermissionRequired);
        }
        let activation = ObserverActivationGuard::acquire(&OBSERVER_ACTIVE)?;
        let failure_generation = OBSERVER_FAILURES.load(Ordering::Acquire);
        let Ok(mut event_sender) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).lock() else {
            return Err(PlatformError::BackendUnavailable);
        };
        *event_sender = Some(sender);
        drop(event_sender);
        let Ok(mut held_keys) = HELD_KEYS.get_or_init(|| Mutex::new(HashSet::new())).lock() else {
            clear_worker_state();
            return Err(PlatformError::BackendUnavailable);
        };
        held_keys.clear();
        drop(held_keys);

        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let exit_handshake = Arc::new(ObserverExitHandshake::new());
        let worker = thread::Builder::new()
            .name("smartcat-macos-keyboard".to_owned())
            .spawn({
                let exit_handshake = Arc::clone(&exit_handshake);
                move || event_tap_thread(ready_sender, done_sender, exit_handshake)
            })
            .map_err(|_| {
                clear_worker_state();
                PlatformError::BackendUnavailable
            })?;
        match ready_receiver.recv_timeout(START_TIMEOUT) {
            Ok(Ok(run_loop)) => {
                activation.commit();
                Ok(Box::new(MacObserver {
                    run_loop,
                    failure_generation,
                    done_receiver: Some(done_receiver),
                    worker: Some(worker),
                    exit_handshake,
                    stopped: false,
                }))
            }
            Ok(Err(error)) => {
                let completed = matches!(
                    done_receiver.recv_timeout(STOP_TIMEOUT),
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
                );
                if completed {
                    let _ = worker.join();
                    clear_worker_state();
                } else {
                    exit_handshake.request_release(&OBSERVER_ACTIVE);
                    drop(worker);
                    disconnect_event_sender();
                    OBSERVER_HEALTHY.store(false, Ordering::Release);
                    // A late failing worker remains the singleton owner until
                    // finish_worker_state releases ACTIVE on its own exit.
                    activation.commit();
                }
                Err(error)
            }
            Err(_) => {
                if let Some(run_loop) = current_run_loop() {
                    unsafe {
                        CFRunLoopStop(run_loop.raw());
                        CFRunLoopWakeUp(run_loop.raw());
                    }
                }
                let completed = matches!(
                    done_receiver.recv_timeout(STOP_TIMEOUT),
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
                );
                if completed {
                    let _ = worker.join();
                    clear_worker_state();
                } else {
                    exit_handshake.request_release(&OBSERVER_ACTIVE);
                    drop(worker);
                    disconnect_event_sender();
                    OBSERVER_HEALTHY.store(false, Ordering::Release);
                    // Quarantine the late worker by retaining ACTIVE until its
                    // own cleanup completes; no newer singleton can be erased.
                    activation.commit();
                }
                Err(PlatformError::BackendUnavailable)
            }
        }
    }
}

struct MacObserver {
    run_loop: Arc<RetainedRunLoop>,
    failure_generation: usize,
    done_receiver: Option<mpsc::Receiver<()>>,
    worker: Option<JoinHandle<()>>,
    exit_handshake: Arc<ObserverExitHandshake>,
    stopped: bool,
}

impl HotkeyObserver for MacObserver {
    fn stop(&mut self) -> Result<(), PlatformError> {
        if self.stopped {
            return Ok(());
        }
        let Some(done_receiver) = self.done_receiver.take() else {
            return Err(PlatformError::ShutdownFailed);
        };
        let already_stopped = match done_receiver.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => true,
            Err(mpsc::TryRecvError::Empty) => false,
        };
        if !already_stopped {
            unsafe {
                CFRunLoopStop(self.run_loop.raw());
                CFRunLoopWakeUp(self.run_loop.raw());
            }
        }
        let stopped = already_stopped
            || matches!(
                done_receiver.recv_timeout(STOP_TIMEOUT),
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
            );
        if !stopped {
            disconnect_event_sender();
            OBSERVER_HEALTHY.store(false, Ordering::Release);
            self.exit_handshake.request_release(&OBSERVER_ACTIVE);
            self.worker.take();
            self.stopped = true;
            return Err(PlatformError::ShutdownFailed);
        }
        let join_failed = self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err());
        OBSERVER_ACTIVE.store(false, Ordering::Release);
        self.stopped = true;
        if join_failed {
            Err(PlatformError::ShutdownFailed)
        } else if OBSERVER_FAILURES.load(Ordering::Acquire) != self.failure_generation {
            Err(PlatformError::ObserverDisabled)
        } else {
            Ok(())
        }
    }
}

impl Drop for MacObserver {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn event_tap_thread(
    ready: mpsc::SyncSender<Result<Arc<RetainedRunLoop>, PlatformError>>,
    done: mpsc::SyncSender<()>,
    exit_handshake: Arc<ObserverExitHandshake>,
) {
    unsafe {
        let tap = CGEventTapCreate(
            CG_SESSION_EVENT_TAP,
            CG_HEAD_INSERT_EVENT_TAP,
            CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            KEY_EVENT_MASK,
            Some(event_tap_callback),
            ptr::null_mut(),
        );
        if tap.is_null() {
            let _ = ready.send(Err(PlatformError::AccessibilityPermissionRequired));
            finish_worker_state(&exit_handshake);
            let _ = done.send(());
            return;
        }
        let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
        if source.is_null() {
            CFRelease(tap);
            let _ = ready.send(Err(PlatformError::BackendUnavailable));
            finish_worker_state(&exit_handshake);
            let _ = done.send(());
            return;
        }
        let run_loop = CFRunLoopGetCurrent();
        CFRetain(run_loop);
        let retained_run_loop = Arc::new(RetainedRunLoop(run_loop as usize));
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        if !CGEventTapIsEnabled(tap) {
            CGEventTapEnable(tap, true);
        }
        if !CGEventTapIsEnabled(tap) {
            CFRelease(source);
            CFRelease(tap);
            let _ = ready.send(Err(PlatformError::BackendUnavailable));
            finish_worker_state(&exit_handshake);
            let _ = done.send(());
            return;
        }
        EVENT_TAP.store(tap, Ordering::Release);
        CALLBACK_RUN_LOOP.store(run_loop, Ordering::Release);
        set_current_run_loop(Some(Arc::clone(&retained_run_loop)));
        OBSERVER_HEALTHY.store(true, Ordering::Release);
        if ready.send(Ok(Arc::clone(&retained_run_loop))).is_err() {
            CFRelease(source);
            CFRelease(tap);
            finish_worker_state(&exit_handshake);
            let _ = done.send(());
            return;
        }
        CFRunLoopRun();
        CFRelease(source);
        CFRelease(tap);
        finish_worker_state(&exit_handshake);
        let _ = done.send(());
    }
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    if is_tap_disabled_event(event_type) {
        let tap = EVENT_TAP.load(Ordering::Acquire);
        let recovered = !tap.is_null()
            && recover_disabled_tap(event_type, || {
                unsafe { CGEventTapEnable(tap, true) };
                unsafe { CGEventTapIsEnabled(tap) }
            });
        if recovered {
            OBSERVER_HEALTHY.store(true, Ordering::Release);
        } else {
            OBSERVER_HEALTHY.store(false, Ordering::Release);
            OBSERVER_FAILURES.fetch_add(1, Ordering::AcqRel);
            disconnect_event_sender_nonblocking();
            let run_loop = CALLBACK_RUN_LOOP.load(Ordering::Acquire);
            if !run_loop.is_null() {
                unsafe {
                    CFRunLoopStop(run_loop);
                    CFRunLoopWakeUp(run_loop);
                }
            }
        }
        return event;
    }
    let phase = match event_type {
        CG_EVENT_KEY_DOWN => KeyEventPhase::Down,
        CG_EVENT_KEY_UP => KeyEventPhase::Up,
        _ => return event,
    };
    if event.is_null() {
        OBSERVER_HEALTHY.store(false, Ordering::Release);
        return event;
    }
    let keycode = unsafe { CGEventGetIntegerValueField(event, CG_KEYBOARD_EVENT_KEYCODE) } as u16;
    if let Some(key) = mac_keycode(keycode) {
        let flags = unsafe { CGEventGetFlags(event) };
        let native_repeat =
            unsafe { CGEventGetIntegerValueField(event, CG_KEYBOARD_EVENT_AUTOREPEAT) } != 0;
        let repeat = if let Ok(mut held) = HELD_KEYS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .try_lock()
        {
            match phase {
                KeyEventPhase::Down => native_repeat || !held.insert(keycode),
                KeyEventPhase::Up => {
                    held.remove(&keycode);
                    false
                }
            }
        } else {
            true
        };
        if let Ok(guard) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).try_lock() {
            if let Some(sender) = guard.as_ref() {
                let _ = sender.try_send(KeyEvent {
                    chord: Chord {
                        modifiers: Modifiers {
                            ctrl: flags & CG_MASK_CONTROL != 0,
                            alt: flags & CG_MASK_ALTERNATE != 0,
                            shift: flags & CG_MASK_SHIFT != 0,
                            meta: flags & CG_MASK_COMMAND != 0,
                        },
                        key,
                    },
                    phase,
                    repeat,
                    device: KeyDevice(0),
                });
            }
        }
    }
    // A listen-only tap must return the original event unchanged.
    event
}

fn finish_worker_state(exit_handshake: &ObserverExitHandshake) {
    clear_worker_state();
    exit_handshake.mark_exited(&OBSERVER_ACTIVE);
}

fn clear_worker_state() {
    disconnect_event_sender();
    EVENT_TAP.store(ptr::null_mut(), Ordering::Release);
    CALLBACK_RUN_LOOP.store(ptr::null_mut(), Ordering::Release);
    set_current_run_loop(None);
    OBSERVER_HEALTHY.store(false, Ordering::Release);
}

fn disconnect_event_sender() {
    if let Ok(mut sender) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).lock() {
        sender.take();
    }
    if let Ok(mut held) = HELD_KEYS.get_or_init(|| Mutex::new(HashSet::new())).lock() {
        held.clear();
    }
}

fn disconnect_event_sender_nonblocking() {
    if let Ok(mut sender) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).try_lock() {
        sender.take();
    }
    if let Ok(mut held) = HELD_KEYS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .try_lock()
    {
        held.clear();
    }
}

struct RetainedRunLoop(usize);

impl RetainedRunLoop {
    fn raw(&self) -> CFRunLoopRef {
        self.0 as CFRunLoopRef
    }
}

impl Drop for RetainedRunLoop {
    fn drop(&mut self) {
        unsafe { CFRelease(self.raw()) };
    }
}

static CURRENT_RUN_LOOP: OnceLock<Mutex<Option<Arc<RetainedRunLoop>>>> = OnceLock::new();

fn set_current_run_loop(value: Option<Arc<RetainedRunLoop>>) {
    if let Ok(mut run_loop) = CURRENT_RUN_LOOP.get_or_init(|| Mutex::new(None)).lock() {
        *run_loop = value;
    }
}

fn current_run_loop() -> Option<Arc<RetainedRunLoop>> {
    CURRENT_RUN_LOOP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|value| value.clone())
}

fn is_tap_disabled_event(event_type: u32) -> bool {
    matches!(
        event_type,
        CG_EVENT_TAP_DISABLED_BY_TIMEOUT | CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    )
}

fn recover_disabled_tap<F>(event_type: u32, mut enable_and_check: F) -> bool
where
    F: FnMut() -> bool,
{
    if !is_tap_disabled_event(event_type) {
        return true;
    }
    (0..REENABLE_ATTEMPTS).any(|_| enable_and_check())
}

fn trigger_is_representable(trigger: &Trigger) -> bool {
    let chords: &[Chord] = match trigger {
        Trigger::Chord { chord } => std::slice::from_ref(chord),
        Trigger::Sequence { steps, .. } => steps,
    };
    chords
        .iter()
        .all(|chord| (0..=126).any(|native| mac_keycode(native) == Some(chord.key)))
}

fn mac_keycode(code: u16) -> Option<KeyCode> {
    use PhysicalKey::*;
    Some(match code {
        0 => KeyCode::Physical(KeyA),
        1 => KeyCode::Physical(KeyS),
        2 => KeyCode::Physical(KeyD),
        3 => KeyCode::Physical(KeyF),
        4 => KeyCode::Physical(KeyH),
        5 => KeyCode::Physical(KeyG),
        6 => KeyCode::Physical(KeyZ),
        7 => KeyCode::Physical(KeyX),
        8 => KeyCode::Physical(KeyC),
        9 => KeyCode::Physical(KeyV),
        11 => KeyCode::Physical(KeyB),
        12 => KeyCode::Physical(KeyQ),
        13 => KeyCode::Physical(KeyW),
        14 => KeyCode::Physical(KeyE),
        15 => KeyCode::Physical(KeyR),
        16 => KeyCode::Physical(KeyY),
        17 => KeyCode::Physical(KeyT),
        18 => KeyCode::Physical(Digit1),
        19 => KeyCode::Physical(Digit2),
        20 => KeyCode::Physical(Digit3),
        21 => KeyCode::Physical(Digit4),
        22 => KeyCode::Physical(Digit6),
        23 => KeyCode::Physical(Digit5),
        24 => KeyCode::Physical(Equal),
        25 => KeyCode::Physical(Digit9),
        26 => KeyCode::Physical(Digit7),
        27 => KeyCode::Physical(Minus),
        28 => KeyCode::Physical(Digit8),
        29 => KeyCode::Physical(Digit0),
        30 => KeyCode::Physical(BracketRight),
        31 => KeyCode::Physical(KeyO),
        32 => KeyCode::Physical(KeyU),
        33 => KeyCode::Physical(BracketLeft),
        34 => KeyCode::Physical(KeyI),
        35 => KeyCode::Physical(KeyP),
        36 => KeyCode::Logical(LogicalKey::Enter),
        37 => KeyCode::Physical(KeyL),
        38 => KeyCode::Physical(KeyJ),
        39 => KeyCode::Physical(Quote),
        40 => KeyCode::Physical(KeyK),
        41 => KeyCode::Physical(Semicolon),
        42 => KeyCode::Physical(Backslash),
        43 => KeyCode::Physical(Comma),
        44 => KeyCode::Physical(Slash),
        45 => KeyCode::Physical(KeyN),
        46 => KeyCode::Physical(KeyM),
        47 => KeyCode::Physical(Period),
        48 => KeyCode::Logical(LogicalKey::Tab),
        49 => KeyCode::Logical(LogicalKey::Space),
        50 => KeyCode::Physical(Backquote),
        51 => KeyCode::Logical(LogicalKey::Backspace),
        53 => KeyCode::Logical(LogicalKey::Escape),
        57 => KeyCode::Physical(CapsLock),
        64 => KeyCode::Physical(F17),
        65 => KeyCode::Physical(NumpadDecimal),
        67 => KeyCode::Physical(NumpadMultiply),
        69 => KeyCode::Physical(NumpadAdd),
        71 => KeyCode::Physical(NumLock),
        75 => KeyCode::Physical(NumpadDivide),
        76 => KeyCode::Physical(NumpadEnter),
        78 => KeyCode::Physical(NumpadSubtract),
        79 => KeyCode::Physical(F18),
        80 => KeyCode::Physical(F19),
        81 => KeyCode::Physical(NumpadEqual),
        82 => KeyCode::Physical(Numpad0),
        83 => KeyCode::Physical(Numpad1),
        84 => KeyCode::Physical(Numpad2),
        85 => KeyCode::Physical(Numpad3),
        86 => KeyCode::Physical(Numpad4),
        87 => KeyCode::Physical(Numpad5),
        88 => KeyCode::Physical(Numpad6),
        89 => KeyCode::Physical(Numpad7),
        90 => KeyCode::Physical(F20),
        91 => KeyCode::Physical(Numpad8),
        92 => KeyCode::Physical(Numpad9),
        96 => KeyCode::Physical(F5),
        97 => KeyCode::Physical(F6),
        98 => KeyCode::Physical(F7),
        99 => KeyCode::Physical(F3),
        100 => KeyCode::Physical(F8),
        101 => KeyCode::Physical(F9),
        103 => KeyCode::Physical(F11),
        105 => KeyCode::Physical(F13),
        106 => KeyCode::Physical(F16),
        107 => KeyCode::Physical(F14),
        109 => KeyCode::Physical(F10),
        111 => KeyCode::Physical(F12),
        113 => KeyCode::Physical(F15),
        115 => KeyCode::Logical(LogicalKey::Home),
        116 => KeyCode::Logical(LogicalKey::PageUp),
        117 => KeyCode::Logical(LogicalKey::Delete),
        118 => KeyCode::Physical(F4),
        119 => KeyCode::Logical(LogicalKey::End),
        120 => KeyCode::Physical(F2),
        121 => KeyCode::Logical(LogicalKey::PageDown),
        122 => KeyCode::Physical(F1),
        123 => KeyCode::Logical(LogicalKey::ArrowLeft),
        124 => KeyCode::Logical(LogicalKey::ArrowRight),
        125 => KeyCode::Logical(LogicalKey::ArrowDown),
        126 => KeyCode::Logical(LogicalKey::ArrowUp),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        thread,
    };

    use super::{
        is_tap_disabled_event, mac_keycode, recover_disabled_tap, trigger_is_representable,
        CFRetain, CFRunLoopGetCurrent, MacObserver, RetainedRunLoop, OBSERVER_ACTIVE,
        OBSERVER_FAILURES,
    };
    use crate::hotkeys::{
        parse_trigger, HotkeyObserver, KeyCode, ObserverExitHandshake, PhysicalKey, PlatformError,
    };

    static OBSERVER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn maps_physical_c_key_without_observing_text() {
        assert_eq!(mac_keycode(8), Some(KeyCode::Physical(PhysicalKey::KeyC)));
    }

    #[test]
    fn rejects_keys_the_macos_adapter_cannot_observe() {
        assert!(trigger_is_representable(&parse_trigger("Ctrl+C").unwrap()));
        assert!(!trigger_is_representable(
            &parse_trigger("Ctrl+F24").unwrap()
        ));
    }

    #[test]
    fn recognizes_both_event_tap_disable_notifications() {
        assert!(is_tap_disabled_event(u32::MAX - 1));
        assert!(is_tap_disabled_event(u32::MAX));
        assert!(!is_tap_disabled_event(10));
    }

    #[test]
    fn disabled_tap_reactivation_is_bounded_and_reports_failure() {
        let attempts = AtomicUsize::new(0);
        assert!(!recover_disabled_tap(u32::MAX - 1, || {
            attempts.fetch_add(1, Ordering::Relaxed);
            false
        }));
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn mac_startup_activation_rolls_back_on_injected_failure() {
        let active = AtomicBool::new(false);
        {
            let _guard = crate::hotkeys::ObserverActivationGuard::acquire(&active).unwrap();
        }
        assert!(!active.load(Ordering::Acquire));
    }

    #[test]
    fn observer_can_join_worker_that_exited_before_stop_without_a_stale_run_loop_call() {
        let _serial = OBSERVER_TEST_LOCK.lock().unwrap();
        let native = unsafe { CFRunLoopGetCurrent() };
        unsafe { CFRetain(native) };
        let run_loop = Arc::new(RetainedRunLoop(native as usize));
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            done_sender.send(()).unwrap();
            finished_sender.send(()).unwrap();
        });
        finished_receiver.recv().unwrap();
        let mut observer = MacObserver {
            run_loop,
            failure_generation: OBSERVER_FAILURES.load(Ordering::Acquire),
            done_receiver: Some(done_receiver),
            worker: Some(worker),
            exit_handshake: Arc::new(ObserverExitHandshake::new()),
            stopped: false,
        };

        assert_eq!(observer.stop(), Ok(()));
        assert!(observer.worker.is_none());
    }

    #[test]
    fn observer_stop_releases_singleton_after_worker_panics() {
        let _serial = OBSERVER_TEST_LOCK.lock().unwrap();
        OBSERVER_ACTIVE.store(true, Ordering::Release);
        let native = unsafe { CFRunLoopGetCurrent() };
        unsafe { CFRetain(native) };
        let run_loop = Arc::new(RetainedRunLoop(native as usize));
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            drop(done_sender);
            panic!("injected observer worker failure");
        });
        let mut observer = MacObserver {
            run_loop,
            failure_generation: OBSERVER_FAILURES.load(Ordering::Acquire),
            done_receiver: Some(done_receiver),
            worker: Some(worker),
            exit_handshake: Arc::new(ObserverExitHandshake::new()),
            stopped: false,
        };

        assert_eq!(observer.stop(), Err(PlatformError::ShutdownFailed));
        assert!(!OBSERVER_ACTIVE.load(Ordering::Acquire));
        assert!(observer.worker.is_none());
    }
}
