use std::{
    collections::HashSet,
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
};

use crate::hotkeys::{
    Chord, HotkeyObserver, KeyCode, KeyDevice, KeyEvent, KeyEventPhase, KeyEventSource, LogicalKey,
    Modifiers, ObserverAvailability, PhysicalKey, PlatformError, RegistrationProbe,
    RegistrationProbeStatus, Trigger,
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
const CG_KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
const CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
const CG_MASK_CONTROL: u64 = 1 << 18;
const CG_MASK_ALTERNATE: u64 = 1 << 19;
const CG_MASK_SHIFT: u64 = 1 << 17;
const CG_MASK_COMMAND: u64 = 1 << 20;
const KEY_EVENT_MASK: u64 = (1 << CG_EVENT_KEY_DOWN) | (1 << CG_EVENT_KEY_UP);

static OBSERVER_ACTIVE: AtomicBool = AtomicBool::new(false);
static EVENT_SENDER: OnceLock<Mutex<Option<mpsc::SyncSender<KeyEvent>>>> = OnceLock::new();
static HELD_KEYS: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();

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
    fn CFRelease(value: *const c_void);
}

pub struct MacRegistrationProbe;

impl RegistrationProbe for MacRegistrationProbe {
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus {
        // macOS has no RegisterHotKey equivalent for these listen-only
        // sequences. Report observer capability without claiming ownership.
        let trusted = unsafe { AXIsProcessTrusted() };
        match trigger {
            Trigger::Sequence { .. } => RegistrationProbeStatus::UnsupportedSequence {
                observer_available: trusted,
            },
            Trigger::Chord { .. } if trusted => RegistrationProbeStatus::AvailableViaObserver,
            Trigger::Chord { .. } => RegistrationProbeStatus::PermissionDenied,
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
        if unsafe { AXIsProcessTrusted() } {
            ObserverAvailability::Available
        } else {
            ObserverAvailability::AccessibilityPermissionRequired
        }
    }

    fn start(
        &self,
        sender: mpsc::SyncSender<KeyEvent>,
    ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
        if self.availability() != ObserverAvailability::Available {
            return Err(PlatformError::AccessibilityPermissionRequired);
        }
        if OBSERVER_ACTIVE.swap(true, Ordering::AcqRel) {
            return Err(PlatformError::AlreadyRunning);
        }
        *EVENT_SENDER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| PlatformError::BackendUnavailable)? = Some(sender);
        HELD_KEYS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .map_err(|_| PlatformError::BackendUnavailable)?
            .clear();

        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("smartcat-macos-keyboard".to_owned())
            .spawn(move || event_tap_thread(ready_sender))
            .map_err(|_| {
                clear_observer_state();
                PlatformError::BackendUnavailable
            })?;
        match ready_receiver.recv() {
            Ok(Ok(run_loop)) => Ok(Box::new(MacObserver {
                run_loop,
                worker: Some(worker),
                stopped: false,
            })),
            Ok(Err(error)) => {
                let _ = worker.join();
                clear_observer_state();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                clear_observer_state();
                Err(PlatformError::BackendUnavailable)
            }
        }
    }
}

struct MacObserver {
    run_loop: usize,
    worker: Option<JoinHandle<()>>,
    stopped: bool,
}

impl HotkeyObserver for MacObserver {
    fn stop(&mut self) -> Result<(), PlatformError> {
        if self.stopped {
            return Ok(());
        }
        let run_loop = self.run_loop as CFRunLoopRef;
        unsafe {
            CFRunLoopStop(run_loop);
            CFRunLoopWakeUp(run_loop);
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| PlatformError::ShutdownFailed)?;
        }
        self.stopped = true;
        Ok(())
    }
}

impl Drop for MacObserver {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn event_tap_thread(ready: mpsc::SyncSender<Result<usize, PlatformError>>) {
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
            clear_observer_state();
            return;
        }
        let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
        if source.is_null() {
            CFRelease(tap);
            let _ = ready.send(Err(PlatformError::BackendUnavailable));
            clear_observer_state();
            return;
        }
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        if ready.send(Ok(run_loop as usize)).is_err() {
            CFRelease(source);
            CFRelease(tap);
            clear_observer_state();
            return;
        }
        CFRunLoopRun();
        CFRelease(source);
        CFRelease(tap);
        clear_observer_state();
    }
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    let phase = match event_type {
        CG_EVENT_KEY_DOWN => KeyEventPhase::Down,
        CG_EVENT_KEY_UP => KeyEventPhase::Up,
        _ => return event,
    };
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

fn clear_observer_state() {
    if let Ok(mut sender) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).lock() {
        sender.take();
    }
    if let Ok(mut held) = HELD_KEYS.get_or_init(|| Mutex::new(HashSet::new())).lock() {
        held.clear();
    }
    OBSERVER_ACTIVE.store(false, Ordering::Release);
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
    use super::mac_keycode;
    use crate::hotkeys::{KeyCode, PhysicalKey};

    #[test]
    fn maps_physical_c_key_without_observing_text() {
        assert_eq!(mac_keycode(8), Some(KeyCode::Physical(PhysicalKey::KeyC)));
    }
}
