//! Opening results, settings storage, and small process-wide helpers.

use super::{pcwstr, wide};
use std::os::windows::process::CommandExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, OpenEventW, OpenProcessToken, SetEvent, WaitForSingleObject,
    EVENT_MODIFY_STATE, INFINITE,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL};

pub fn open(target: &str) {
    let op = wide("open");
    let file = wide(target);
    unsafe {
        ShellExecuteW(None, pcwstr(&op), pcwstr(&file), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Opens Explorer with the item selected.
pub fn open_containing_folder(path: &str) {
    let op = wide("open");
    let exe = wide("explorer.exe");
    let args = wide(&format!("/select,\"{path}\""));
    unsafe {
        ShellExecuteW(None, pcwstr(&op), pcwstr(&exe), pcwstr(&args), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

const SHOW_EVENT: &str = "Local\\Glint.Show";

/// The running copy's "show yourself" event, or `None` if another copy
/// already runs. The event lets a second launch (from the Start menu, say)
/// bring up the window of the copy that is already running.
pub fn claim_single_instance() -> Option<HANDLE> {
    let name = wide("Local\\Glint.Instance");
    unsafe {
        let handle = CreateMutexW(None, true, pcwstr(&name));
        // Leaked on purpose: held for the life of the process.
        if handle.is_err() || GetLastError() == ERROR_ALREADY_EXISTS {
            return None;
        }
        // Signalable by a non-elevated launch even when we run elevated:
        // everyone may use it, and the low integrity label allows writes from below.
        let sddl = wide("D:(A;;GA;;;WD)S:(ML;;NW;;;LW)");
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let _ = ConvertStringSecurityDescriptorToSecurityDescriptorW(pcwstr(&sddl), SDDL_REVISION_1, &mut sd, None);
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let ev = wide(SHOW_EVENT);
        CreateEventW(Some(&sa), false, false, pcwstr(&ev)).ok()
    }
}

/// Asks the running copy to show its window. False if none is running.
pub fn signal_show() -> bool {
    let ev = wide(SHOW_EVENT);
    unsafe {
        let Ok(h) = OpenEventW(EVENT_MODIFY_STATE, false, pcwstr(&ev)) else { return false };
        // We were just launched by the user, so we may hand them the foreground.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let ok = SetEvent(h).is_ok();
        let _ = CloseHandle(h);
        ok
    }
}

/// Calls `on_show` each time another launch asks us to show the window.
pub fn listen_for_show(event: HANDLE, on_show: impl Fn() + Send + 'static) {
    let raw = event.0 as isize;
    std::thread::Builder::new()
        .name("show".into())
        .stack_size(64 << 10)
        .spawn(move || loop {
            let h = HANDLE(raw as *mut _);
            if unsafe { WaitForSingleObject(h, INFINITE) } != WAIT_OBJECT_0 {
                return;
            }
            on_show();
        })
        .unwrap();
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Name of the logon task the administrator installer creates. It starts the
/// app with full rights without a UAC prompt on every sign-in.
pub const ELEVATED_TASK: &str = "Glint";

/// Whether the administrator installer set this copy up (it leaves a marker).
pub fn has_elevated_task() -> bool {
    std::env::current_exe().is_ok_and(|exe| exe.with_file_name("elevated-task").exists())
}

/// Starts the elevated logon task now. True if it was started.
pub fn run_elevated_task() -> bool {
    std::process::Command::new("schtasks.exe")
        .args(["/Run", "/TN", ELEVATED_TASK])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .is_ok_and(|s| s.success())
}

fn reg_dword(key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            pcwstr(&k),
            pcwstr(&v),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

/// Whether apps should use the dark theme, per Settings > Personalization > Colors.
pub fn system_dark_mode() -> bool {
    reg_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme") == Some(0)
}

/// The accent fill Windows itself uses for the current mode, as (r, g, b):
/// the lighter "Light 2" shade in dark mode, the deeper "Dark 1" in light mode.
pub fn accent_color(dark: bool) -> (u8, u8, u8) {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent");
    let val = wide("AccentPalette");
    let mut palette = [0u8; 32];
    let mut size = palette.len() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            pcwstr(&key),
            pcwstr(&val),
            windows::Win32::System::Registry::RRF_RT_REG_BINARY,
            None,
            Some(palette.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .is_ok()
    };
    if !ok || size < 32 {
        return if dark { (0x4c, 0xc2, 0xff) } else { (0x00, 0x5f, 0xb8) };
    }
    // Eight RGBA entries: Light3, Light2, Light1, Base, Dark1, Dark2, Dark3, unused.
    let i = if dark { 1 } else { 4 } * 4;
    (palette[i], palette[i + 1], palette[i + 2])
}

/// Whether Settings > Personalization > Colors > Transparency effects is on.
pub fn transparency_enabled() -> bool {
    reg_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "EnableTransparency") != Some(0)
}

/// Windows 11 is build 22000 or later.
pub fn is_windows_11() -> bool {
    windows_build() >= 22000
}

pub fn windows_build() -> u32 {
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let val = wide("CurrentBuildNumber");
    let mut buf = [0u16; 16];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            pcwstr(&key),
            pcwstr(&val),
            windows::Win32::System::Registry::RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .is_ok()
    };
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    if !ok {
        return 0;
    }
    String::from_utf16_lossy(&buf[..n]).parse().unwrap_or(0)
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Turns starting at sign-in on or off: the logon task for an administrator
/// install, the per-user Run entry otherwise. True on success.
pub fn set_autostart(on: bool) -> bool {
    if has_elevated_task() {
        return std::process::Command::new("schtasks.exe")
            .args(["/Change", "/TN", ELEVATED_TASK, if on { "/ENABLE" } else { "/DISABLE" }])
            .creation_flags(0x0800_0000)
            .status()
            .is_ok_and(|s| s.success());
    }
    let key = wide(RUN_KEY);
    let name = wide(ELEVATED_TASK);
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return false };
            let cmd = wide(&format!("\"{}\" --background", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                pcwstr(&key),
                pcwstr(&name),
                REG_SZ.0,
                Some(cmd.as_ptr() as *const _),
                (cmd.len() * 2) as u32,
            )
            .is_ok()
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, pcwstr(&key), pcwstr(&name));
            r.is_ok() || r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
        }
    }
}
