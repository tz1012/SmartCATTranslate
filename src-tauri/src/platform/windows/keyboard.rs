use std::{
    collections::HashSet,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
};

use windows_sys::Win32::{
    Foundation::{
        GetLastError, ERROR_ACCESS_DENIED, ERROR_HOTKEY_ALREADY_REGISTERED, ERROR_INVALID_PARAMETER,
    },
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
            MOD_SHIFT, MOD_WIN, VK_ADD, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_DELETE,
            VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LWIN,
            VK_MENU, VK_MULTIPLY, VK_NEXT, VK_NUMLOCK, VK_NUMPAD0, VK_PAUSE, VK_PRIOR, VK_RETURN,
            VK_RIGHT, VK_RWIN, VK_SCROLL, VK_SHIFT, VK_SNAPSHOT, VK_SPACE, VK_SUBTRACT, VK_TAB,
            VK_UP,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
            SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT,
            MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
            WM_SYSKEYUP,
        },
    },
};

use crate::hotkeys::{
    Chord, HotkeyObserver, KeyCode, KeyDevice, KeyEvent, KeyEventPhase, KeyEventSource, LogicalKey,
    Modifiers, ObserverAvailability, PhysicalKey, PlatformError, RegistrationProbe,
    RegistrationProbeStatus, Trigger,
};

static OBSERVER_ACTIVE: AtomicBool = AtomicBool::new(false);
static EVENT_SENDER: OnceLock<Mutex<Option<mpsc::SyncSender<KeyEvent>>>> = OnceLock::new();
static HELD_KEYS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
static NEXT_PROBE_ID: AtomicI32 = AtomicI32::new(0x5000);

pub struct WindowsRegistrationProbe {
    observer_available: bool,
}

impl WindowsRegistrationProbe {
    pub fn new(observer_available: bool) -> Self {
        Self { observer_available }
    }
}

impl RegistrationProbe for WindowsRegistrationProbe {
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus {
        let Trigger::Chord { chord } = trigger else {
            return RegistrationProbeStatus::UnsupportedSequence {
                observer_available: self.observer_available,
            };
        };
        if chord.modifiers.meta {
            return RegistrationProbeStatus::OsReserved;
        }
        let Some(vk) = chord_to_virtual_key(*chord) else {
            return RegistrationProbeStatus::Invalid;
        };
        let id = NEXT_PROBE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(if current >= 0xBFFE {
                    0x5000
                } else {
                    current + 1
                })
            })
            .unwrap_or(0x5000);
        let modifiers = hotkey_modifiers(chord.modifiers) | MOD_NOREPEAT;
        // SAFETY: this thread-scoped registration uses a bounded valid ID and
        // is removed before the method returns.
        if unsafe { RegisterHotKey(ptr::null_mut(), id, modifiers, vk) } == 0 {
            // SAFETY: GetLastError is read immediately after the failed API call.
            return match unsafe { GetLastError() } {
                ERROR_HOTKEY_ALREADY_REGISTERED => RegistrationProbeStatus::Occupied {
                    observer_available: self.observer_available,
                },
                ERROR_ACCESS_DENIED => RegistrationProbeStatus::PermissionDenied,
                ERROR_INVALID_PARAMETER => RegistrationProbeStatus::Invalid,
                _ => RegistrationProbeStatus::BackendError,
            };
        }
        // SAFETY: the exact ID registered above is unregistered on the same thread.
        if unsafe { UnregisterHotKey(ptr::null_mut(), id) } == 0 {
            RegistrationProbeStatus::BackendError
        } else {
            RegistrationProbeStatus::Available
        }
    }
}

#[derive(Default)]
pub struct WindowsKeyEventSource;

impl WindowsKeyEventSource {
    pub fn new() -> Self {
        Self
    }
}

impl KeyEventSource for WindowsKeyEventSource {
    fn availability(&self) -> ObserverAvailability {
        ObserverAvailability::Available
    }

    fn start(
        &self,
        sender: mpsc::SyncSender<KeyEvent>,
    ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
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
            .name("smartcat-windows-keyboard".to_owned())
            .spawn(move || hook_thread(ready_sender))
            .map_err(|_| {
                clear_observer_state();
                PlatformError::BackendUnavailable
            })?;
        match ready_receiver.recv() {
            Ok(Ok(thread_id)) => Ok(Box::new(WindowsObserver {
                thread_id,
                worker: Some(worker),
                stopped: false,
            })),
            _ => {
                let _ = worker.join();
                clear_observer_state();
                Err(PlatformError::BackendUnavailable)
            }
        }
    }
}

struct WindowsObserver {
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
    stopped: bool,
}

impl HotkeyObserver for WindowsObserver {
    fn stop(&mut self) -> Result<(), PlatformError> {
        if self.stopped {
            return Ok(());
        }
        // SAFETY: thread_id belongs to the live hook thread, whose queue was
        // created before start returned.
        if unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) } == 0 {
            return Err(PlatformError::ShutdownFailed);
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| PlatformError::ShutdownFailed)?;
        }
        self.stopped = true;
        Ok(())
    }
}

impl Drop for WindowsObserver {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn hook_thread(ready: mpsc::SyncSender<Result<u32, PlatformError>>) {
    // SAFETY: all Win32 handles remain owned by this dedicated thread.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), ptr::null_mut(), 0);
        if hook.is_null() {
            let _ = ready.send(Err(PlatformError::BackendUnavailable));
            clear_observer_state();
            return;
        }
        let thread_id = GetCurrentThreadId();
        if ready.send(Ok(thread_id)).is_err() {
            UnhookWindowsHookEx(hook);
            clear_observer_state();
            return;
        }
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(hook);
        clear_observer_state();
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == HC_ACTION as i32 {
        let phase = match wparam as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyEventPhase::Down),
            WM_KEYUP | WM_SYSKEYUP => Some(KeyEventPhase::Up),
            _ => None,
        };
        if let Some(phase) = phase {
            // SAFETY: Windows supplies a KBDLLHOOKSTRUCT for HC_ACTION keyboard messages.
            let native = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if let Some(chord) = virtual_key_to_chord(native.vkCode) {
                let repeat = if let Ok(mut held) = HELD_KEYS
                    .get_or_init(|| Mutex::new(HashSet::new()))
                    .try_lock()
                {
                    match phase {
                        KeyEventPhase::Down => !held.insert(native.vkCode),
                        KeyEventPhase::Up => {
                            held.remove(&native.vkCode);
                            false
                        }
                    }
                } else {
                    true
                };
                if let Ok(guard) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).try_lock() {
                    if let Some(sender) = guard.as_ref() {
                        let _ = sender.try_send(KeyEvent {
                            chord,
                            phase,
                            repeat,
                            device: KeyDevice(0),
                        });
                    }
                }
            }
        }
    }
    // SAFETY: listen-only hooks must always preserve host application input.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
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

fn hotkey_modifiers(modifiers: Modifiers) -> u32 {
    (if modifiers.ctrl { MOD_CONTROL } else { 0 })
        | (if modifiers.alt { MOD_ALT } else { 0 })
        | (if modifiers.shift { MOD_SHIFT } else { 0 })
        | (if modifiers.meta { MOD_WIN } else { 0 })
}

fn chord_to_virtual_key(chord: Chord) -> Option<u32> {
    key_to_virtual_key(chord.key)
}

fn key_to_virtual_key(key: KeyCode) -> Option<u32> {
    match key {
        KeyCode::Physical(key) => physical_to_virtual_key(key),
        KeyCode::Logical(key) => Some(match key {
            LogicalKey::ArrowUp => VK_UP as u32,
            LogicalKey::ArrowDown => VK_DOWN as u32,
            LogicalKey::ArrowLeft => 0x25,
            LogicalKey::ArrowRight => VK_RIGHT as u32,
            LogicalKey::Backspace => VK_BACK as u32,
            LogicalKey::Delete => VK_DELETE as u32,
            LogicalKey::End => VK_END as u32,
            LogicalKey::Enter => VK_RETURN as u32,
            LogicalKey::Escape => VK_ESCAPE as u32,
            LogicalKey::Home => VK_HOME as u32,
            LogicalKey::Insert => VK_INSERT as u32,
            LogicalKey::PageDown => VK_NEXT as u32,
            LogicalKey::PageUp => VK_PRIOR as u32,
            LogicalKey::Space => VK_SPACE as u32,
            LogicalKey::Tab => VK_TAB as u32,
        }),
    }
}

fn physical_to_virtual_key(key: PhysicalKey) -> Option<u32> {
    use PhysicalKey::*;
    Some(match key {
        value if (KeyA as u32..=KeyZ as u32).contains(&(value as u32)) => {
            0x41 + (value as u32 - KeyA as u32)
        }
        value if (Digit0 as u32..=Digit9 as u32).contains(&(value as u32)) => {
            0x30 + (value as u32 - Digit0 as u32)
        }
        Backquote => 0xC0,
        Backslash | IntlBackslash => 0xDC,
        BracketLeft => 0xDB,
        BracketRight => 0xDD,
        Comma => 0xBC,
        Equal | NumpadEqual => 0xBB,
        Minus => 0xBD,
        Period => 0xBE,
        Quote => 0xDE,
        Semicolon => 0xBA,
        Slash => 0xBF,
        value if (Numpad0 as u32..=Numpad9 as u32).contains(&(value as u32)) => {
            VK_NUMPAD0 as u32 + (value as u32 - Numpad0 as u32)
        }
        NumpadAdd => VK_ADD as u32,
        NumpadDecimal => VK_DECIMAL as u32,
        NumpadDivide => VK_DIVIDE as u32,
        NumpadMultiply => VK_MULTIPLY as u32,
        NumpadSubtract => VK_SUBTRACT as u32,
        NumpadEnter => VK_RETURN as u32,
        IntlRo | IntlYen => return None,
        PrintScreen => VK_SNAPSHOT as u32,
        Pause => VK_PAUSE as u32,
        CapsLock => VK_CAPITAL as u32,
        NumLock => VK_NUMLOCK as u32,
        ScrollLock => VK_SCROLL as u32,
        ContextMenu => 0x5D,
        value if (F1 as u32..=F24 as u32).contains(&(value as u32)) => {
            VK_F1 as u32 + (value as u32 - F1 as u32)
        }
        _ => return None,
    })
}

fn virtual_key_to_chord(vk: u32) -> Option<Chord> {
    const LETTERS: [PhysicalKey; 26] = [
        PhysicalKey::KeyA,
        PhysicalKey::KeyB,
        PhysicalKey::KeyC,
        PhysicalKey::KeyD,
        PhysicalKey::KeyE,
        PhysicalKey::KeyF,
        PhysicalKey::KeyG,
        PhysicalKey::KeyH,
        PhysicalKey::KeyI,
        PhysicalKey::KeyJ,
        PhysicalKey::KeyK,
        PhysicalKey::KeyL,
        PhysicalKey::KeyM,
        PhysicalKey::KeyN,
        PhysicalKey::KeyO,
        PhysicalKey::KeyP,
        PhysicalKey::KeyQ,
        PhysicalKey::KeyR,
        PhysicalKey::KeyS,
        PhysicalKey::KeyT,
        PhysicalKey::KeyU,
        PhysicalKey::KeyV,
        PhysicalKey::KeyW,
        PhysicalKey::KeyX,
        PhysicalKey::KeyY,
        PhysicalKey::KeyZ,
    ];
    const DIGITS: [PhysicalKey; 10] = [
        PhysicalKey::Digit0,
        PhysicalKey::Digit1,
        PhysicalKey::Digit2,
        PhysicalKey::Digit3,
        PhysicalKey::Digit4,
        PhysicalKey::Digit5,
        PhysicalKey::Digit6,
        PhysicalKey::Digit7,
        PhysicalKey::Digit8,
        PhysicalKey::Digit9,
    ];
    const FUNCTION_KEYS: [PhysicalKey; 24] = [
        PhysicalKey::F1,
        PhysicalKey::F2,
        PhysicalKey::F3,
        PhysicalKey::F4,
        PhysicalKey::F5,
        PhysicalKey::F6,
        PhysicalKey::F7,
        PhysicalKey::F8,
        PhysicalKey::F9,
        PhysicalKey::F10,
        PhysicalKey::F11,
        PhysicalKey::F12,
        PhysicalKey::F13,
        PhysicalKey::F14,
        PhysicalKey::F15,
        PhysicalKey::F16,
        PhysicalKey::F17,
        PhysicalKey::F18,
        PhysicalKey::F19,
        PhysicalKey::F20,
        PhysicalKey::F21,
        PhysicalKey::F22,
        PhysicalKey::F23,
        PhysicalKey::F24,
    ];
    const NUMPAD_KEYS: [PhysicalKey; 10] = [
        PhysicalKey::Numpad0,
        PhysicalKey::Numpad1,
        PhysicalKey::Numpad2,
        PhysicalKey::Numpad3,
        PhysicalKey::Numpad4,
        PhysicalKey::Numpad5,
        PhysicalKey::Numpad6,
        PhysicalKey::Numpad7,
        PhysicalKey::Numpad8,
        PhysicalKey::Numpad9,
    ];
    let key = if (0x41..=0x5A).contains(&vk) {
        KeyCode::Physical(LETTERS[(vk - 0x41) as usize])
    } else if (0x30..=0x39).contains(&vk) {
        KeyCode::Physical(DIGITS[(vk - 0x30) as usize])
    } else if (VK_F1 as u32..=VK_F24 as u32).contains(&vk) {
        KeyCode::Physical(FUNCTION_KEYS[(vk - VK_F1 as u32) as usize])
    } else if (VK_NUMPAD0 as u32..=VK_NUMPAD0 as u32 + 9).contains(&vk) {
        KeyCode::Physical(NUMPAD_KEYS[(vk - VK_NUMPAD0 as u32) as usize])
    } else {
        match vk {
            0xBA => KeyCode::Physical(PhysicalKey::Semicolon),
            0xBB => KeyCode::Physical(PhysicalKey::Equal),
            0xBC => KeyCode::Physical(PhysicalKey::Comma),
            0xBD => KeyCode::Physical(PhysicalKey::Minus),
            0xBE => KeyCode::Physical(PhysicalKey::Period),
            0xBF => KeyCode::Physical(PhysicalKey::Slash),
            0xC0 => KeyCode::Physical(PhysicalKey::Backquote),
            0xDB => KeyCode::Physical(PhysicalKey::BracketLeft),
            0xDC => KeyCode::Physical(PhysicalKey::Backslash),
            0xDD => KeyCode::Physical(PhysicalKey::BracketRight),
            0xDE => KeyCode::Physical(PhysicalKey::Quote),
            0xE2 => KeyCode::Physical(PhysicalKey::IntlBackslash),
            value if value == VK_ADD as u32 => KeyCode::Physical(PhysicalKey::NumpadAdd),
            value if value == VK_DECIMAL as u32 => KeyCode::Physical(PhysicalKey::NumpadDecimal),
            value if value == VK_DIVIDE as u32 => KeyCode::Physical(PhysicalKey::NumpadDivide),
            value if value == VK_MULTIPLY as u32 => KeyCode::Physical(PhysicalKey::NumpadMultiply),
            value if value == VK_SUBTRACT as u32 => KeyCode::Physical(PhysicalKey::NumpadSubtract),
            value if value == VK_SNAPSHOT as u32 => KeyCode::Physical(PhysicalKey::PrintScreen),
            value if value == VK_PAUSE as u32 => KeyCode::Physical(PhysicalKey::Pause),
            value if value == VK_CAPITAL as u32 => KeyCode::Physical(PhysicalKey::CapsLock),
            value if value == VK_NUMLOCK as u32 => KeyCode::Physical(PhysicalKey::NumLock),
            value if value == VK_SCROLL as u32 => KeyCode::Physical(PhysicalKey::ScrollLock),
            0x5D => KeyCode::Physical(PhysicalKey::ContextMenu),
            value if value == VK_UP as u32 => KeyCode::Logical(LogicalKey::ArrowUp),
            value if value == VK_DOWN as u32 => KeyCode::Logical(LogicalKey::ArrowDown),
            0x25 => KeyCode::Logical(LogicalKey::ArrowLeft),
            value if value == VK_RIGHT as u32 => KeyCode::Logical(LogicalKey::ArrowRight),
            value if value == VK_BACK as u32 => KeyCode::Logical(LogicalKey::Backspace),
            value if value == VK_DELETE as u32 => KeyCode::Logical(LogicalKey::Delete),
            value if value == VK_END as u32 => KeyCode::Logical(LogicalKey::End),
            value if value == VK_RETURN as u32 => KeyCode::Logical(LogicalKey::Enter),
            value if value == VK_ESCAPE as u32 => KeyCode::Logical(LogicalKey::Escape),
            value if value == VK_HOME as u32 => KeyCode::Logical(LogicalKey::Home),
            value if value == VK_INSERT as u32 => KeyCode::Logical(LogicalKey::Insert),
            value if value == VK_NEXT as u32 => KeyCode::Logical(LogicalKey::PageDown),
            value if value == VK_PRIOR as u32 => KeyCode::Logical(LogicalKey::PageUp),
            value if value == VK_SPACE as u32 => KeyCode::Logical(LogicalKey::Space),
            value if value == VK_TAB as u32 => KeyCode::Logical(LogicalKey::Tab),
            _ => return None,
        }
    };
    let down = |key: u16| unsafe { GetAsyncKeyState(key as i32) } < 0;
    Some(Chord {
        modifiers: Modifiers {
            ctrl: down(VK_CONTROL),
            alt: down(VK_MENU),
            shift: down(VK_SHIFT),
            meta: down(VK_LWIN) || down(VK_RWIN),
        },
        key,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crate::hotkeys::{
        parse_trigger, KeyCode, KeyEventSource, ObserverAvailability, PhysicalKey, PlatformError,
        RegistrationProbe, RegistrationProbeStatus,
    };

    use super::{virtual_key_to_chord, WindowsKeyEventSource, WindowsRegistrationProbe};

    #[test]
    fn maps_punctuation_and_numpad_keys_used_by_configurable_shortcuts() {
        assert_eq!(
            virtual_key_to_chord(0xBB).unwrap().key,
            KeyCode::Physical(PhysicalKey::Equal)
        );
        assert_eq!(
            virtual_key_to_chord(0x61).unwrap().key,
            KeyCode::Physical(PhysicalKey::Numpad1)
        );
    }

    #[test]
    fn real_registration_trial_unregisters_immediately() {
        let probe = WindowsRegistrationProbe::new(true);
        let trigger = parse_trigger("Ctrl+Alt+Shift+F24").unwrap();

        assert_eq!(
            probe.probe_and_restore(&trigger),
            RegistrationProbeStatus::Available
        );
        assert_eq!(
            probe.probe_and_restore(&trigger),
            RegistrationProbeStatus::Available
        );
    }

    #[test]
    fn sequences_report_observer_capability_without_pretending_to_register() {
        let trigger = parse_trigger("Ctrl+C, C").unwrap();
        assert_eq!(
            WindowsRegistrationProbe::new(true).probe_and_restore(&trigger),
            RegistrationProbeStatus::UnsupportedSequence {
                observer_available: true
            }
        );
        assert_eq!(
            WindowsRegistrationProbe::new(false).probe_and_restore(&trigger),
            RegistrationProbeStatus::UnsupportedSequence {
                observer_available: false
            }
        );
    }

    #[test]
    fn observer_reports_windows_availability() {
        assert_eq!(
            WindowsKeyEventSource::new().availability(),
            ObserverAvailability::Available
        );
    }

    #[test]
    fn real_observer_rejects_duplicates_and_can_restart_after_clean_stop() {
        let source = WindowsKeyEventSource::new();
        let (sender, _receiver) = mpsc::sync_channel(4);
        let mut observer = source.start(sender).unwrap();
        let (duplicate_sender, _duplicate_receiver) = mpsc::sync_channel(4);
        assert!(matches!(
            source.start(duplicate_sender),
            Err(PlatformError::AlreadyRunning)
        ));
        observer.stop().unwrap();

        let (restart_sender, _restart_receiver) = mpsc::sync_channel(4);
        let mut restarted = source.start(restart_sender).unwrap();
        restarted.stop().unwrap();
    }
}
