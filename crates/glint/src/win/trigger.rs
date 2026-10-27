//! What opens the window:
//! - typing a character while the bare desktop has focus;
//! - the user's hotkey from anywhere else.
//!
//! The keyboard hook exists only while the desktop is the foreground window.
//! Windows tells us when the foreground changes, so nothing polls, and keys
//! typed into any other app never pass through this process.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, RegisterHotKey, ToUnicodeEx, UnregisterHotKey, HOT_KEY_MODIFIERS,
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_CAPITAL, VK_CONTROL, VK_LWIN, VK_MENU,
    VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetMessageW,
    GetWindowThreadProcessId, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    EVENT_SYSTEM_FOREGROUND, GUITHREADINFO, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_APP,
    WINEVENT_OUTOFCONTEXT, WM_HOTKEY, WM_KEYDOWN, WM_SYSKEYDOWN,
};

#[derive(Debug, Clone)]
pub enum Trigger {
    /// A character typed on the desktop; it becomes the start of the query.
    Typed(String),
    Hotkey,
    /// The app was launched again (Start menu, desktop shortcut) or the
    /// tray icon was clicked: show the window.
    Show,
    /// "Settings" in the tray menu.
    Settings,
    /// "Quit" in the tray menu.
    Quit,
    /// A window (raw HWND) took the foreground; may be our own.
    ForegroundChanged(isize),
    /// Custom shortcut number n (its index in the settings) was pressed.
    Shortcut(usize),
}

pub struct Config {
    pub desktop_typing: bool,
    /// Shown next to "Open search" in the tray menu.
    pub hotkey_label: String,
    pub hotkey: Option<(HOT_KEY_MODIFIERS, u32)>,
    /// Custom shortcuts, in settings order; `None` for unparsable ones.
    pub shortcuts: Vec<Option<(HOT_KEY_MODIFIERS, u32)>>,
}

type Sink = Box<dyn Fn(Trigger) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();
static THREAD: OnceLock<u32> = OnceLock::new();
static DESKTOP_TYPING: AtomicBool = AtomicBool::new(true);
/// Where the trigger thread reports whether a new hotkey registered.
static HOTKEY_REPLY: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);
/// Custom shortcuts waiting to be registered, and where to report which worked.
#[allow(clippy::type_complexity)]
static SHORTCUTS: Mutex<Option<(Vec<Option<(HOT_KEY_MODIFIERS, u32)>>, mpsc::Sender<Vec<bool>>)>> = Mutex::new(None);

const WM_SET_HOTKEY: u32 = WM_APP + 1;
const WM_REFRESH_HOOK: u32 = WM_APP + 2;
const WM_SET_SHORTCUTS: u32 = WM_APP + 3;
/// Hotkey ids: 1 opens search, SHORTCUT_BASE + n is custom shortcut n.
const SHORTCUT_BASE: usize = 100;
const MAX_SHORTCUTS: usize = 64;

thread_local! {
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
}

/// Starts the trigger thread. Returns whether the hotkey could be registered.
pub fn start(config: Config, sink: impl Fn(Trigger) + Send + Sync + 'static) -> bool {
    let _ = SINK.set(Box::new(sink));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("trigger".into())
        .spawn(move || unsafe { run(config, tx) })
        .unwrap();
    rx.recv().unwrap_or(false)
}

/// Swaps the hotkey. False if another app already owns that combination,
/// in which case nothing is registered and the caller should restore the old one.
pub fn set_hotkey(hotkey: (HOT_KEY_MODIFIERS, u32)) -> bool {
    let Some(&tid) = THREAD.get() else { return false };
    let (tx, rx) = mpsc::channel();
    *HOTKEY_REPLY.lock().unwrap() = Some(tx);
    unsafe {
        if PostThreadMessageW(tid, WM_SET_HOTKEY, WPARAM(hotkey.0 .0 as usize), LPARAM(hotkey.1 as isize)).is_err() {
            return false;
        }
    }
    rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or(false)
}

/// Replaces the custom shortcuts. Returns, per shortcut, whether it could be
/// registered (false: invalid, or another app already owns it).
pub fn set_shortcuts(list: Vec<Option<(HOT_KEY_MODIFIERS, u32)>>) -> Vec<bool> {
    let n = list.len();
    let Some(&tid) = THREAD.get() else { return vec![false; n] };
    let (tx, rx) = mpsc::channel();
    *SHORTCUTS.lock().unwrap() = Some((list, tx));
    unsafe {
        if PostThreadMessageW(tid, WM_SET_SHORTCUTS, WPARAM(0), LPARAM(0)).is_err() {
            return vec![false; n];
        }
    }
    rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or_else(|_| vec![false; n])
}

unsafe fn register_shortcuts(list: &[Option<(HOT_KEY_MODIFIERS, u32)>]) -> Vec<bool> {
    for id in SHORTCUT_BASE..SHORTCUT_BASE + MAX_SHORTCUTS {
        let _ = UnregisterHotKey(None, id as i32);
    }
    list.iter()
        .take(MAX_SHORTCUTS)
        .enumerate()
        .map(|(i, k)| match k {
            Some((m, vk)) => RegisterHotKey(None, (SHORTCUT_BASE + i) as i32, *m | MOD_NOREPEAT, *vk).is_ok(),
            None => false,
        })
        .collect()
}

pub fn set_desktop_typing(on: bool) {
    DESKTOP_TYPING.store(on, Ordering::SeqCst);
    if let Some(&tid) = THREAD.get() {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_REFRESH_HOOK, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe fn register(hotkey: Option<(HOT_KEY_MODIFIERS, u32)>) -> bool {
    let _ = UnregisterHotKey(None, 1);
    match hotkey {
        Some((m, vk)) if vk != 0 => RegisterHotKey(None, 1, m | MOD_NOREPEAT, vk).is_ok(),
        _ => true,
    }
}

unsafe fn run(config: Config, ready: std::sync::mpsc::Sender<bool>) {
    let _ = THREAD.set(GetCurrentThreadId());
    DESKTOP_TYPING.store(config.desktop_typing, Ordering::SeqCst);
    let ok = register(config.hotkey);
    register_shortcuts(&config.shortcuts);
    SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        None,
        Some(on_foreground),
        0,
        0,
        WINEVENT_OUTOFCONTEXT,
    );
    update_hook(GetForegroundWindow());
    super::tray::create(&config.hotkey_label);
    let _ = ready.send(ok);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        match msg.message {
            WM_HOTKEY if msg.wParam.0 >= SHORTCUT_BASE => emit(Trigger::Shortcut(msg.wParam.0 - SHORTCUT_BASE)),
            WM_HOTKEY => emit(Trigger::Hotkey),
            WM_SET_SHORTCUTS => {
                if let Some((list, tx)) = SHORTCUTS.lock().unwrap().take() {
                    let _ = tx.send(register_shortcuts(&list));
                }
            }
            WM_SET_HOTKEY => {
                let ok = register(Some((HOT_KEY_MODIFIERS(msg.wParam.0 as u32), msg.lParam.0 as u32)));
                if let Some(tx) = HOTKEY_REPLY.lock().unwrap().take() {
                    let _ = tx.send(ok);
                }
            }
            WM_REFRESH_HOOK => update_hook(GetForegroundWindow()),
            _ => {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

pub fn emit(t: Trigger) {
    if let Some(s) = SINK.get() {
        s(t);
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// The bare desktop: Explorer's Progman/WorkerW windows, not renaming an icon.
fn desktop_has_focus(fg: HWND) -> bool {
    if fg.is_invalid() || !matches!(class_name(fg).as_str(), "Progman" | "WorkerW") {
        return false;
    }
    unsafe {
        let tid = GetWindowThreadProcessId(fg, None);
        let mut gti = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(tid, &mut gti).is_ok() && !gti.hwndFocus.is_invalid() {
            return class_name(gti.hwndFocus) != "Edit";
        }
    }
    true
}

unsafe extern "system" fn on_foreground(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    update_hook(hwnd);
    emit(Trigger::ForegroundChanged(hwnd.0 as isize));
}

unsafe fn update_hook(fg: HWND) {
    let want = DESKTOP_TYPING.load(Ordering::SeqCst) && desktop_has_focus(fg);
    HOOK.with(|h| match (h.get(), want) {
        (None, true) => {
            h.set(SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), None, 0).ok());
        }
        (Some(hook), false) => {
            let _ = UnhookWindowsHookEx(hook);
            h.set(None);
        }
        _ => {}
    });
}

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && (wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN) {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let shortcut = down(VK_CONTROL) || down(VK_MENU) || down(VK_LWIN) || down(VK_RWIN);
        if kb.flags.0 & LLKHF_INJECTED.0 == 0 && !shortcut && desktop_has_focus(GetForegroundWindow()) {
            if let Some(text) = typed_text(kb) {
                emit(Trigger::Typed(text));
                return LRESULT(1); // swallow: the search box shows it instead
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// The character this key produces in the desktop's current keyboard layout
/// (so Persian, English, etc. all work), if it is a printable one.
unsafe fn typed_text(kb: &KBDLLHOOKSTRUCT) -> Option<String> {
    let mut state = [0u8; 256];
    if down(VK_SHIFT) {
        state[VK_SHIFT.0 as usize] = 0x80;
    }
    if GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 {
        state[VK_CAPITAL.0 as usize] = 0x01;
    }
    let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
    let layout = GetKeyboardLayout(fg_thread);
    let mut buf = [0u16; 8];
    // Flag 4: don't disturb the keyboard's dead-key state.
    let n = ToUnicodeEx(kb.vkCode, kb.scanCode, &state, &mut buf, 4, Some(layout));
    if n <= 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..n as usize]);
    (s.chars().all(|c| !c.is_control()) && !s.trim().is_empty()).then_some(s)
}

/// Parses "Alt+Space", "Ctrl+Shift+F", "Win+Q" and similar.
pub fn parse_hotkey(s: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut vk = None;
    for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "windows" => mods |= MOD_WIN,
            "space" => vk = Some(0x20),
            "enter" => vk = Some(0x0D),
            "tab" => vk = Some(0x09),
            "`" | "backquote" => vk = Some(0xC0),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => {
                vk = Some(k.as_bytes()[0].to_ascii_uppercase() as u32)
            }
            k if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
                vk = Some(0x70 + k[1..].parse::<u32>().unwrap() - 1)
            }
            _ => return None,
        }
    }
    vk.map(|v| (mods, v))
}
