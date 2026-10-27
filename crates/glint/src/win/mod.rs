//! Windows-specific parts: app list, NTFS index, triggers, window styling.

pub mod apps;
pub mod icons;
pub mod ntfs;
pub mod shell;
pub mod tray;
pub mod trigger;
pub mod window;

use windows::core::PCWSTR;

/// NUL-terminated UTF-16 for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn pcwstr(w: &[u16]) -> PCWSTR {
    PCWSTR(w.as_ptr())
}
