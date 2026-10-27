//! Makes the Slint window look and behave like a Windows 11 flyout: no
//! taskbar button, rounded corners, acrylic see-through background, placed
//! on the monitor under the mouse, and actually brought to the front.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmEnableBlurBehindWindow, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_NONE,
    DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, HGDIOBJ};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_MOUSE};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, GWL_STYLE, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_STYLECHANGING,
    WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
    WS_THICKFRAME,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    SetForegroundWindow, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

pub fn hwnd(window: &slint::Window) -> Option<HWND> {
    match window.window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Keeps the window a captionless popup with no taskbar button. winit
/// rewrites the styles (adding a caption) on every show and hide, so the
/// styles are corrected as they change rather than once.
unsafe extern "system" fn keep_popup_style(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_STYLECHANGING {
        let ss = &mut *(lparam.0 as *mut STYLESTRUCT);
        if wparam.0 as i32 == GWL_STYLE.0 {
            let strip = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_BORDER | WS_DLGFRAME;
            ss.styleNew = (ss.styleNew & !strip.0) | WS_POPUP.0;
        } else if wparam.0 as i32 == GWL_EXSTYLE.0 {
            ss.styleNew = (ss.styleNew | WS_EX_TOOLWINDOW.0) & !(WS_EX_APPWINDOW.0 | WS_EX_WINDOWEDGE.0);
        }
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// One-time styling after the native window exists.
pub fn style(hwnd: HWND, dark: bool) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(keep_popup_style), 1, 0);
        // Re-set both styles so they pass through the filter above.
        SetWindowLongPtrW(hwnd, GWL_STYLE, GetWindowLongPtrW(hwnd, GWL_STYLE));
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, GetWindowLongPtrW(hwnd, GWL_EXSTYLE));
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
        // Extending the frame over the whole window gives a borderless window
        // the system drop shadow and lets the backdrop material show through.
        let m = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
        // Honor per-pixel alpha from our renderer (an empty blur region blurs nothing).
        let rgn = CreateRectRgn(0, 0, -1, -1);
        let bb = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION, fEnable: true.into(), hRgnBlur: rgn, ..Default::default() };
        let _ = DwmEnableBlurBehindWindow(hwnd, &bb);
        let _ = DeleteObject(HGDIOBJ(rgn.0));
    }
    set_dark(hwnd, dark);
}

pub fn set_dark(hwnd: HWND, dark: bool) {
    let v: i32 = dark.into();
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v as *const _ as *const _, 4);
    }
}

/// Work area (physical pixels) of the monitor under the mouse.
pub fn work_area_under_cursor() -> RECT {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

/// Windows only lets the app the user is interacting with take the
/// foreground. A zero-size synthetic input makes us that app, the same
/// workaround PowerToys Run uses; AttachThreadInput covers the rest.
pub fn force_foreground(hwnd: HWND) {
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
            return;
        }
        let input = INPUT { r#type: INPUT_MOUSE, ..Default::default() };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let me = GetCurrentThreadId();
        let attached = fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
    }
}

/// Turns the acrylic material on or off and returns whether it is on.
/// Windows 11 22H2+ has it built in; Windows 10 and early 11 get the same
/// blur through the accent policy the Start menu and taskbar use.
pub fn set_acrylic(hwnd: HWND, on: bool, dark: bool, build: u32) -> bool {
    unsafe {
        if build >= 22523 {
            let kind = if on { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE };
            let ok = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as *const _, 4).is_ok();
            return on && ok;
        }
        // Tint as 0xAABBGGRR, matching the Windows 10 Start menu.
        let tint: u32 = if dark { 0xCC20_2020 } else { 0xCCF3_F3F3 };
        set_accent_policy(hwnd, if on { ACCENT_ENABLE_ACRYLICBLURBEHIND } else { ACCENT_DISABLED }, tint) && on
    }
}

const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
const WCA_ACCENT_POLICY: u32 = 19;

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct CompositionAttribData {
    attrib: u32,
    data: *mut core::ffi::c_void,
    size: usize,
}

/// `SetWindowCompositionAttribute` is exported by user32 but not in the SDK headers.
unsafe fn set_accent_policy(hwnd: HWND, state: u32, tint: u32) -> bool {
    type SetWca = unsafe extern "system" fn(HWND, *mut CompositionAttribData) -> i32;
    let Ok(user32) = GetModuleHandleW(windows::core::w!("user32.dll")) else { return false };
    let Some(f) = GetProcAddress(user32, windows::core::s!("SetWindowCompositionAttribute")) else { return false };
    let f: SetWca = std::mem::transmute(f);
    let mut policy = AccentPolicy { state, flags: 2, gradient_color: tint, animation_id: 0 };
    let mut data = CompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        data: &mut policy as *mut _ as *mut _,
        size: std::mem::size_of::<AccentPolicy>(),
    };
    f(hwnd, &mut data) != 0
}
