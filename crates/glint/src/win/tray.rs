//! Icon in the notification area (next to the clock, or under its arrow)
//! so it's visible that Search is running. Click opens search; right-click
//! offers Open, Settings and Quit. Lives on the trigger thread's message loop.

use super::trigger::{emit, Trigger};
use super::wide;
use std::sync::OnceLock;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, GetCursorPos, GetSystemMetrics,
    LoadImageW, PostMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, HICON,
    IMAGE_ICON, LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, SM_CXSMICON, SM_CYSMICON, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
    WNDCLASSW,
};

const WM_TRAY: u32 = WM_APP + 10;
const CMD_OPEN: usize = 1;
const CMD_SETTINGS: usize = 2;
const CMD_QUIT: usize = 3;

static HWND_RAW: OnceLock<isize> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
static HOTKEY_LABEL: OnceLock<String> = OnceLock::new();

/// Adds the icon. Call on a thread that runs a message loop.
pub fn create(hotkey: &str) {
    let _ = HOTKEY_LABEL.set(hotkey.to_string());
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("GlintTray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        // A hidden top-level window (not message-only) so it also hears
        // "TaskbarCreated" when Explorer restarts and re-adds the icon.
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Glint"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            return;
        };
        let _ = HWND_RAW.set(hwnd.0 as isize);
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        add(hwnd);
    }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

unsafe fn add(hwnd: HWND) {
    let mut nid = data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = app_icon();
    let tip: Vec<u16> = "Glint".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    let _ = Shell_NotifyIconW(NIM_ADD, &nid);
}

/// Removes the icon; call before the process exits so no ghost icon remains.
pub fn remove() {
    if let Some(&raw) = HWND_RAW.get() {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data(HWND(raw as *mut _)));
        }
    }
}

/// The app icon embedded in the exe (resource 1), at the small-icon size.
unsafe fn app_icon() -> HICON {
    let Ok(module) = GetModuleHandleW(None) else { return HICON::default() };
    LoadImageW(
        Some(module.into()),
        PCWSTR(1 as *const u16),
        IMAGE_ICON,
        GetSystemMetrics(SM_CXSMICON),
        GetSystemMetrics(SM_CYSMICON),
        LR_DEFAULTCOLOR,
    )
    .map(|h| HICON(h.0))
    .unwrap_or_default()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        match lparam.0 as u32 & 0xFFFF {
            WM_LBUTTONUP => emit(Trigger::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }
    if TASKBAR_CREATED.get() == Some(&msg) {
        add(hwnd);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let open = wide(&format!("Open search\t{}", HOTKEY_LABEL.get().map_or("", |s| s.as_str())));
    let settings = wide("Settings");
    let quit = wide("Quit");
    let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN, PCWSTR(open.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, PCWSTR(settings.as_ptr()));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, PCWSTR(quit.as_ptr()));
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Required so the menu closes when clicking elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), hwnd, None);
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    match cmd.0 as usize {
        CMD_OPEN => emit(Trigger::Show),
        CMD_SETTINGS => emit(Trigger::Settings),
        CMD_QUIT => {
            remove();
            emit(Trigger::Quit);
        }
        _ => {}
    }
}
