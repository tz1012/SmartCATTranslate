use std::{
    collections::HashSet,
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_HOTKEY_ALREADY_REGISTERED,
        ERROR_INVALID_PARAMETER,
    },
    System::Threading::{CreateEventW, SetEvent},
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
            CallNextHookEx, DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW,
            SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT,
            LLKHF_EXTENDED, MSG, MWMO_INPUTAVAILABLE, PM_NOREMOVE, PM_REMOVE, QS_ALLINPUT,
            WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

use crate::hotkeys::{
    Chord, HotkeyObserver, KeyCode, KeyDevice, KeyEvent, KeyEventPhase, KeyEventSource, LogicalKey,
    Modifiers, ObserverActivationGuard, ObserverAvailability, ObserverExitHandshake, PhysicalKey,
    PlatformError, RegistrationProbe, RegistrationProbeStatus, Trigger,
};

static OBSERVER_ACTIVE: AtomicBool = AtomicBool::new(false);
static OBSERVER_HEALTHY: AtomicBool = AtomicBool::new(false);
static EVENT_SENDER: OnceLock<Mutex<Option<mpsc::SyncSender<KeyEvent>>>> = OnceLock::new();
static HELD_KEYS: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();

const START_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const WAIT_OBJECT_0: u32 = 0;
const WAIT_INFINITE: u32 = u32::MAX;
const UNREGISTER_ATTEMPTS: usize = 3;

trait RegistrationApi {
    fn register(&self, id: i32, modifiers: u32, vk: u32) -> Result<(), u32>;
    fn unregister(&self, id: i32) -> Result<(), u32>;
}

struct WinRegistrationApi;

impl RegistrationApi for WinRegistrationApi {
    fn register(&self, id: i32, modifiers: u32, vk: u32) -> Result<(), u32> {
        if unsafe { RegisterHotKey(ptr::null_mut(), id, modifiers, vk) } == 0 {
            Err(unsafe { GetLastError() })
        } else {
            Ok(())
        }
    }

    fn unregister(&self, id: i32) -> Result<(), u32> {
        if unsafe { UnregisterHotKey(ptr::null_mut(), id) } == 0 {
            Err(unsafe { GetLastError() })
        } else {
            Ok(())
        }
    }
}

struct RegistrationLease<'a, A: RegistrationApi> {
    api: &'a A,
    id: i32,
    registered: bool,
}

impl<'a, A: RegistrationApi> RegistrationLease<'a, A> {
    fn acquire(api: &'a A, id: i32, modifiers: u32, vk: u32) -> Result<Self, u32> {
        api.register(id, modifiers, vk)?;
        Ok(Self {
            api,
            id,
            registered: true,
        })
    }

    fn restore(&mut self) -> Result<(), u32> {
        let mut last_error = 0;
        for _ in 0..UNREGISTER_ATTEMPTS {
            match self.api.unregister(self.id) {
                Ok(()) => {
                    self.registered = false;
                    return Ok(());
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }
}

impl<A: RegistrationApi> Drop for RegistrationLease<'_, A> {
    fn drop(&mut self) {
        if self.registered {
            let _ = self.restore();
        }
    }
}

fn probe_registered_with_api<A: RegistrationApi>(
    api: &A,
    id: i32,
    modifiers: u32,
    vk: u32,
    observer_available: bool,
) -> RegistrationProbeStatus {
    let mut lease = match RegistrationLease::acquire(api, id, modifiers, vk) {
        Ok(lease) => lease,
        Err(ERROR_HOTKEY_ALREADY_REGISTERED) => {
            return RegistrationProbeStatus::Occupied { observer_available };
        }
        Err(ERROR_ACCESS_DENIED) => return RegistrationProbeStatus::PermissionDenied,
        Err(ERROR_INVALID_PARAMETER) => return RegistrationProbeStatus::Invalid,
        Err(_) => return RegistrationProbeStatus::BackendError,
    };
    if lease.restore().is_ok() {
        RegistrationProbeStatus::Available
    } else {
        RegistrationProbeStatus::BackendError
    }
}

#[derive(Default)]
pub struct WindowsRegistrationProbe;

impl WindowsRegistrationProbe {
    pub fn new() -> Self {
        Self
    }
}

impl RegistrationProbe for WindowsRegistrationProbe {
    fn probe_and_restore(&self, trigger: &Trigger) -> RegistrationProbeStatus {
        let Trigger::Chord { chord } = trigger else {
            return RegistrationProbeStatus::UnsupportedSequence {
                observer_available: OBSERVER_HEALTHY.load(Ordering::Acquire),
            };
        };
        if chord.modifiers.meta {
            return RegistrationProbeStatus::OsReserved;
        }
        let Some(vk) = chord_to_virtual_key(*chord) else {
            return RegistrationProbeStatus::Invalid;
        };
        if native_key_to_key(NativeKey::from_registration(vk)) != Some(chord.key) {
            return if OBSERVER_HEALTHY.load(Ordering::Acquire) {
                RegistrationProbeStatus::AvailableViaObserver
            } else {
                RegistrationProbeStatus::Invalid
            };
        }
        let modifiers = hotkey_modifiers(chord.modifiers) | MOD_NOREPEAT;
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let Ok(worker) = thread::Builder::new()
            .name("smartcat-hotkey-probe".to_owned())
            .spawn(move || {
                const PROBE_ID: i32 = 0x5000;
                // A thread-owned registration is also removed by Windows when
                // this short-lived probe thread exits, including unregister failures.
                let status = probe_registered_with_api(
                    &WinRegistrationApi,
                    PROBE_ID,
                    modifiers,
                    vk,
                    OBSERVER_HEALTHY.load(Ordering::Acquire),
                );
                let _ = result_sender.send(status);
            })
        else {
            return RegistrationProbeStatus::BackendError;
        };
        match result_receiver.recv_timeout(START_TIMEOUT) {
            Ok(status) => {
                if worker.join().is_err() {
                    RegistrationProbeStatus::BackendError
                } else {
                    status
                }
            }
            Err(_) => {
                drop(worker);
                RegistrationProbeStatus::BackendError
            }
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
        if OBSERVER_HEALTHY.load(Ordering::Acquire) {
            ObserverAvailability::Available
        } else {
            ObserverAvailability::Unsupported
        }
    }

    fn start(
        &self,
        sender: mpsc::SyncSender<KeyEvent>,
    ) -> Result<Box<dyn HotkeyObserver>, PlatformError> {
        let activation = ObserverActivationGuard::acquire(&OBSERVER_ACTIVE)?;
        let Ok(mut event_sender) = EVENT_SENDER.get_or_init(|| Mutex::new(None)).lock() else {
            OBSERVER_ACTIVE.store(false, Ordering::Release);
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

        let stop_event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if stop_event.is_null() {
            clear_worker_state();
            return Err(PlatformError::BackendUnavailable);
        }
        let stop_event = Arc::new(WindowsStopEvent(stop_event as usize));
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let exit_handshake = Arc::new(ObserverExitHandshake::new());
        let worker = thread::Builder::new()
            .name("smartcat-windows-keyboard".to_owned())
            .spawn({
                let stop_event = Arc::clone(&stop_event);
                let exit_handshake = Arc::clone(&exit_handshake);
                move || hook_thread(stop_event, exit_handshake, ready_sender, done_sender)
            })
            .map_err(|_| {
                clear_worker_state();
                PlatformError::BackendUnavailable
            })?;
        match ready_receiver.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => {
                activation.commit();
                Ok(Box::new(WindowsObserver {
                    stop_event,
                    done_receiver: Some(done_receiver),
                    worker: Some(worker),
                    exit_handshake,
                    stopped: false,
                }))
            }
            _ => {
                unsafe { SetEvent(stop_event.raw()) };
                let completed = done_receiver.recv_timeout(STOP_TIMEOUT).is_ok();
                if completed {
                    let _ = worker.join();
                } else {
                    exit_handshake.request_release(&OBSERVER_ACTIVE);
                    drop(worker);
                    disconnect_event_sender();
                    OBSERVER_HEALTHY.store(false, Ordering::Release);
                    // The late worker exclusively owns cleanup. Keep ACTIVE set
                    // so it cannot erase a newer observer's singleton state.
                    activation.commit();
                }
                Err(PlatformError::BackendUnavailable)
            }
        }
    }
}

struct WindowsObserver {
    stop_event: Arc<WindowsStopEvent>,
    done_receiver: Option<mpsc::Receiver<()>>,
    worker: Option<JoinHandle<()>>,
    exit_handshake: Arc<ObserverExitHandshake>,
    stopped: bool,
}

impl HotkeyObserver for WindowsObserver {
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
        let result = if already_stopped {
            let join_failed = self
                .worker
                .take()
                .is_some_and(|handle| handle.join().is_err());
            OBSERVER_ACTIVE.store(false, Ordering::Release);
            if join_failed {
                Err(PlatformError::ShutdownFailed)
            } else {
                Ok(())
            }
        } else {
            finish_observer_shutdown(
                || {
                    if unsafe { SetEvent(self.stop_event.raw()) } == 0 {
                        Err(PlatformError::ShutdownFailed)
                    } else {
                        Ok(())
                    }
                },
                done_receiver,
                &mut self.worker,
                STOP_TIMEOUT,
                &self.exit_handshake,
                &OBSERVER_ACTIVE,
            )
        };
        self.stopped = self.worker.is_none();
        result
    }
}

struct WindowsStopEvent(usize);

impl WindowsStopEvent {
    fn raw(&self) -> *mut c_void {
        self.0 as *mut c_void
    }
}

impl Drop for WindowsStopEvent {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.raw()) };
    }
}

fn finish_observer_shutdown<F>(
    mut signal_stop: F,
    done_receiver: mpsc::Receiver<()>,
    worker: &mut Option<JoinHandle<()>>,
    timeout: Duration,
    exit_handshake: &ObserverExitHandshake,
    active: &AtomicBool,
) -> Result<(), PlatformError>
where
    F: FnMut() -> Result<(), PlatformError>,
{
    let signal_error = signal_stop().err();
    let completed = matches!(
        done_receiver.recv_timeout(timeout),
        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
    );
    let join_error = if completed {
        let failed = worker.take().is_some_and(|handle| handle.join().is_err());
        active.store(false, Ordering::Release);
        failed
    } else {
        // A native API failure must not turn application shutdown into an
        // unbounded join. Dropping the handle detaches only after producers
        // have been disconnected by the caller.
        exit_handshake.request_release(active);
        worker.take();
        true
    };
    if let Some(error) = signal_error {
        Err(error)
    } else if join_error {
        Err(PlatformError::ShutdownFailed)
    } else {
        Ok(())
    }
}

impl Drop for WindowsObserver {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn hook_thread(
    stop_event: Arc<WindowsStopEvent>,
    exit_handshake: Arc<ObserverExitHandshake>,
    ready: mpsc::SyncSender<Result<(), PlatformError>>,
    done: mpsc::SyncSender<()>,
) {
    // SAFETY: all Win32 handles remain owned by this dedicated thread.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), ptr::null_mut(), 0);
        if hook.is_null() {
            let _ = ready.send(Err(PlatformError::BackendUnavailable));
            clear_worker_state();
            exit_handshake.mark_exited(&OBSERVER_ACTIVE);
            let _ = done.send(());
            return;
        }
        OBSERVER_HEALTHY.store(true, Ordering::Release);
        if ready.send(Ok(())).is_err() {
            UnhookWindowsHookEx(hook);
            clear_worker_state();
            exit_handshake.mark_exited(&OBSERVER_ACTIVE);
            let _ = done.send(());
            return;
        }
        loop {
            let stop_handle = stop_event.raw();
            let wait = MsgWaitForMultipleObjectsEx(
                1,
                &stop_handle,
                WAIT_INFINITE,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
            if wait == WAIT_OBJECT_0 {
                break;
            }
            if wait != WAIT_OBJECT_0 + 1 {
                break;
            }
            let mut quit = false;
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    quit = true;
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if quit {
                break;
            }
        }
        UnhookWindowsHookEx(hook);
        clear_worker_state();
        exit_handshake.mark_exited(&OBSERVER_ACTIVE);
        let _ = done.send(());
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
            let native_key = NativeKey::new(
                native.vkCode,
                native.scanCode,
                native.flags & LLKHF_EXTENDED != 0,
            );
            if let Some(chord) = native_key_to_chord(native_key) {
                let held_id = native_key.identity();
                let repeat = if let Ok(mut held) = HELD_KEYS
                    .get_or_init(|| Mutex::new(HashSet::new()))
                    .try_lock()
                {
                    match phase {
                        KeyEventPhase::Down => !held.insert(held_id),
                        KeyEventPhase::Up => {
                            held.remove(&held_id);
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

fn clear_worker_state() {
    disconnect_event_sender();
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
        Backslash => 0xDC,
        IntlBackslash => 0xE2,
        BracketLeft => 0xDB,
        BracketRight => 0xDD,
        Comma => 0xBC,
        Equal => 0xBB,
        NumpadEqual => 0x92,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeKey {
    vk_code: u32,
    scan_code: u32,
    extended: bool,
}

impl NativeKey {
    const fn new(vk_code: u32, scan_code: u32, extended: bool) -> Self {
        Self {
            vk_code,
            scan_code,
            extended,
        }
    }

    const fn from_registration(vk_code: u32) -> Self {
        Self::new(vk_code, 0, false)
    }

    const fn identity(self) -> u64 {
        self.vk_code as u64 | ((self.scan_code as u64) << 32) | ((self.extended as u64) << 63)
    }
}

fn native_key_to_key(native: NativeKey) -> Option<KeyCode> {
    let vk = native.vk_code;
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
    Some(if (0x41..=0x5A).contains(&vk) {
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
            0xBB if native.extended && native.scan_code == 0x59 => {
                KeyCode::Physical(PhysicalKey::NumpadEqual)
            }
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
            0x92 => KeyCode::Physical(PhysicalKey::NumpadEqual),
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
            value if value == VK_INSERT as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad0)
            }
            value if value == VK_END as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad1)
            }
            value if value == VK_DOWN as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad2)
            }
            value if value == VK_NEXT as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad3)
            }
            0x25 if !native.extended => KeyCode::Physical(PhysicalKey::Numpad4),
            0x0C if !native.extended => KeyCode::Physical(PhysicalKey::Numpad5),
            value if value == VK_RIGHT as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad6)
            }
            value if value == VK_HOME as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad7)
            }
            value if value == VK_UP as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad8)
            }
            value if value == VK_PRIOR as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::Numpad9)
            }
            value if value == VK_DELETE as u32 && !native.extended => {
                KeyCode::Physical(PhysicalKey::NumpadDecimal)
            }
            value if value == VK_UP as u32 => KeyCode::Logical(LogicalKey::ArrowUp),
            value if value == VK_DOWN as u32 => KeyCode::Logical(LogicalKey::ArrowDown),
            0x25 => KeyCode::Logical(LogicalKey::ArrowLeft),
            value if value == VK_RIGHT as u32 => KeyCode::Logical(LogicalKey::ArrowRight),
            value if value == VK_BACK as u32 => KeyCode::Logical(LogicalKey::Backspace),
            value if value == VK_DELETE as u32 => KeyCode::Logical(LogicalKey::Delete),
            value if value == VK_END as u32 => KeyCode::Logical(LogicalKey::End),
            value if value == VK_RETURN as u32 && native.extended => {
                KeyCode::Physical(PhysicalKey::NumpadEnter)
            }
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
    })
}

fn native_key_to_chord(native: NativeKey) -> Option<Chord> {
    let key = native_key_to_key(native)?;
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
    use std::{
        ptr,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        thread,
        time::Duration,
    };

    use crate::hotkeys::{
        parse_trigger, HotkeyObserver, KeyCode, KeyEventSource, ObserverAvailability, PhysicalKey,
        PlatformError, RegistrationProbe, RegistrationProbeStatus,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcessId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_END;

    use super::{
        finish_observer_shutdown, native_key_to_chord, native_key_to_key,
        probe_registered_with_api, NativeKey, ObserverExitHandshake, RegistrationApi,
        WindowsKeyEventSource, WindowsObserver, WindowsRegistrationProbe, WindowsStopEvent,
        OBSERVER_ACTIVE,
    };

    static OBSERVER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct FailingUnregisterApi {
        registered: AtomicBool,
        failures_remaining: AtomicUsize,
        unregister_calls: AtomicUsize,
    }

    impl RegistrationApi for FailingUnregisterApi {
        fn register(&self, _id: i32, _modifiers: u32, _vk: u32) -> Result<(), u32> {
            self.registered.store(true, Ordering::Release);
            Ok(())
        }

        fn unregister(&self, _id: i32) -> Result<(), u32> {
            self.unregister_calls.fetch_add(1, Ordering::Relaxed);
            if self
                .failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                Err(5)
            } else {
                self.registered.store(false, Ordering::Release);
                Ok(())
            }
        }
    }

    #[test]
    fn maps_punctuation_and_numpad_keys_used_by_configurable_shortcuts() {
        assert_eq!(
            native_key_to_chord(NativeKey::new(0xBB, 0x0D, false))
                .unwrap()
                .key,
            KeyCode::Physical(PhysicalKey::Equal)
        );
        assert_eq!(
            native_key_to_chord(NativeKey::new(0x61, 0x4F, false))
                .unwrap()
                .key,
            KeyCode::Physical(PhysicalKey::Numpad1)
        );
    }

    #[test]
    fn unregister_failure_is_retried_until_the_registration_is_restored() {
        let api = FailingUnregisterApi {
            registered: AtomicBool::new(false),
            failures_remaining: AtomicUsize::new(2),
            unregister_calls: AtomicUsize::new(0),
        };

        assert_eq!(
            probe_registered_with_api(&api, 1, 2, 3, true),
            RegistrationProbeStatus::Available
        );
        assert!(!api.registered.load(Ordering::Acquire));
        assert_eq!(api.unregister_calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn permanent_unregister_failure_is_non_forceable_and_bounded() {
        let api = FailingUnregisterApi {
            registered: AtomicBool::new(false),
            failures_remaining: AtomicUsize::new(usize::MAX),
            unregister_calls: AtomicUsize::new(0),
        };

        assert_eq!(
            probe_registered_with_api(&api, 1, 2, 3, true),
            RegistrationProbeStatus::BackendError
        );
        assert!(api.registered.load(Ordering::Acquire));
        assert!(api.unregister_calls.load(Ordering::Relaxed) <= 6);
    }

    #[test]
    fn shutdown_preserves_signal_error_but_still_reclaims_worker() {
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let mut worker = Some(thread::spawn(move || {
            let _ = done_sender.send(());
        }));

        assert_eq!(
            finish_observer_shutdown(
                || Err(PlatformError::ShutdownFailed),
                done_receiver,
                &mut worker,
                Duration::from_millis(100),
                &ObserverExitHandshake::new(),
                &AtomicBool::new(true),
            ),
            Err(PlatformError::ShutdownFailed)
        );
        assert!(worker.is_none());
    }

    #[test]
    fn observer_stop_skips_signaling_after_worker_has_already_finished() {
        let _serial = OBSERVER_TEST_LOCK.lock().unwrap();
        let raw = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        assert!(!raw.is_null());
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            done_sender.send(()).unwrap();
            finished_sender.send(()).unwrap();
        });
        finished_receiver.recv().unwrap();
        let mut observer = WindowsObserver {
            stop_event: Arc::new(WindowsStopEvent(raw as usize)),
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
        let raw = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        assert!(!raw.is_null());
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            drop(done_sender);
            panic!("injected observer worker failure");
        });
        let mut observer = WindowsObserver {
            stop_event: Arc::new(WindowsStopEvent(raw as usize)),
            done_receiver: Some(done_receiver),
            worker: Some(worker),
            exit_handshake: Arc::new(ObserverExitHandshake::new()),
            stopped: false,
        };

        assert_eq!(observer.stop(), Err(PlatformError::ShutdownFailed));
        assert!(!OBSERVER_ACTIVE.load(Ordering::Acquire));
        assert!(observer.worker.is_none());
    }

    #[test]
    fn real_registration_trial_unregisters_immediately() {
        let probe = WindowsRegistrationProbe::new();
        let function_key = 13 + unsafe { GetCurrentProcessId() } % 12;
        let trigger = parse_trigger(&format!("Ctrl+Alt+Shift+F{function_key}")).unwrap();

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
        let _serial = OBSERVER_TEST_LOCK.lock().unwrap();
        let trigger = parse_trigger("Ctrl+C, C").unwrap();
        assert_eq!(
            WindowsRegistrationProbe::new().probe_and_restore(&trigger),
            RegistrationProbeStatus::UnsupportedSequence {
                observer_available: false
            }
        );
    }

    #[test]
    fn observer_reports_windows_availability() {
        let _serial = OBSERVER_TEST_LOCK.lock().unwrap();
        assert_eq!(
            WindowsKeyEventSource::new().availability(),
            ObserverAvailability::Unsupported
        );
    }

    #[test]
    fn preserves_distinct_extended_and_international_keys() {
        assert_eq!(
            native_key_to_key(NativeKey::new(0x0D, 0x1C, true)),
            Some(KeyCode::Physical(PhysicalKey::NumpadEnter))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(0x0D, 0x1C, false)),
            Some(KeyCode::Logical(crate::hotkeys::LogicalKey::Enter))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(0x92, 0x59, false)),
            Some(KeyCode::Physical(PhysicalKey::NumpadEqual))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(0xBB, 0x59, true)),
            Some(KeyCode::Physical(PhysicalKey::NumpadEqual))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(0xBB, 0x0D, false)),
            Some(KeyCode::Physical(PhysicalKey::Equal))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(0xE2, 0x56, false)),
            Some(KeyCode::Physical(PhysicalKey::IntlBackslash))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(VK_END as u32, 0x4F, false)),
            Some(KeyCode::Physical(PhysicalKey::Numpad1))
        );
        assert_eq!(
            native_key_to_key(NativeKey::new(VK_END as u32, 0x4F, true)),
            Some(KeyCode::Logical(crate::hotkeys::LogicalKey::End))
        );
    }

    #[test]
    fn real_observer_rejects_duplicates_and_can_restart_after_clean_stop() {
        let _serial = OBSERVER_TEST_LOCK.lock().unwrap();
        let source = WindowsKeyEventSource::new();
        let (sender, _receiver) = mpsc::sync_channel(4);
        let mut observer = source.start(sender).unwrap();
        assert_eq!(source.availability(), ObserverAvailability::Available);
        assert_eq!(
            WindowsRegistrationProbe::new().probe_and_restore(&parse_trigger("Ctrl+C, C").unwrap()),
            RegistrationProbeStatus::UnsupportedSequence {
                observer_available: true
            }
        );
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
