//! The settings window: edits [`Settings`] and applies each change at once.

use crate::{current_settings, Shared, SettingsWindow, Theme, SETTINGS, SETTINGS_WINDOW};
use slint::{ComponentHandle, SharedString};
use std::sync::Arc;

/// "Ctrl+Shift+F" style text for a key press, or "" if it can't be a
/// shortcut. A shortcut needs Ctrl, Alt or Win so plain typing still works.
pub fn describe_hotkey(text: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> String {
    use slint::platform::Key;
    if !(ctrl || alt || win) {
        return String::new();
    }
    let c = text.chars().next().unwrap_or('\0');
    let fkeys = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12];
    let key = if c == ' ' {
        "Space".to_string()
    } else if c == '`' {
        "`".to_string()
    } else if c.is_ascii_alphanumeric() && text.chars().count() == 1 {
        c.to_ascii_uppercase().to_string()
    } else if let Some(i) = fkeys.iter().position(|k| char::from(k.clone()) == c) {
        format!("F{}", i + 1)
    } else {
        return String::new(); // e.g. a Persian letter: the shortcut is stored by English key name
    };
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if win {
        parts.push("Win");
    }
    parts.push(&key);
    parts.join("+")
}

pub fn open(shared: &Arc<Shared>) {
    SETTINGS_WINDOW.with(|cell| {
        let mut cell = cell.borrow_mut();
        let w = cell.get_or_insert_with(|| create(shared.clone()));
        let s = current_settings();
        w.set_hotkey(s.hotkey.as_str().into());
        w.set_hotkey_error(SharedString::new());
        w.set_recording(false);
        w.set_desktop_typing(s.desktop_typing);
        w.set_position(if s.position == "center" { 1 } else { 0 });
        w.set_default_category(s.default_category);
        w.set_start_with_windows(s.start_with_windows);
        w.set_open_location_key(s.open_location_key);
        w.set_category_key(s.category_key);
        w.set_index_status(shared.status.lock().unwrap().as_str().into());
        w.set_version(format!("Glint {} · Made with Slint", env!("CARGO_PKG_VERSION")).into());
        apply_theme(w);
        #[cfg(windows)]
        if !w.window().is_visible() {
            let area = crate::win::window::work_area_under_cursor();
            let scale = w.window().scale_factor();
            let (ww, wh) = (640.0 * scale, 720.0 * scale);
            let x = area.left as f32 + ((area.right - area.left) as f32 - ww) / 2.0;
            let y = area.top as f32 + (((area.bottom - area.top) as f32 - wh) / 2.0).max(0.0);
            w.window().set_position(slint::PhysicalPosition::new(x as i32, y as i32));
        }
        let _ = w.show();
        #[cfg(windows)]
        if let Some(h) = crate::win::window::hwnd(w.window()) {
            crate::win::window::set_dark(h, w.global::<Theme>().get_dark());
            crate::win::window::force_foreground(h);
        }
    });
}

fn apply_theme(w: &SettingsWindow) {
    let theme = w.global::<Theme>();
    #[cfg(windows)]
    {
        let dark = crate::win::shell::system_dark_mode();
        let (r, g, b) = crate::win::shell::accent_color(dark);
        theme.set_dark(dark);
        theme.set_accent(slint::Color::from_rgb_u8(r, g, b));
        theme.set_font(if crate::win::shell::is_windows_11() { "Segoe UI Variable Text" } else { "Segoe UI" }.into());
    }
    #[cfg(not(windows))]
    theme.set_dark(std::env::var("GLINT_DARK").map_or(true, |v| v != "0"));
    w.invoke_sync_theme();
}

fn create(_shared: Arc<Shared>) -> SettingsWindow {
    let w = SettingsWindow::new().expect("create settings window");
    w.on_describe_hotkey(|text, ctrl, alt, shift, win| describe_hotkey(&text, ctrl, alt, shift, win).into());

    let weak = w.as_weak();
    w.on_hotkey_chosen(move |combo| {
        let Some(w) = weak.upgrade() else { return };
        let old = current_settings().hotkey;
        if combo.as_str() == old {
            return;
        }
        if apply_hotkey(&combo) {
            update(|s| s.hotkey = combo.to_string());
            w.set_hotkey(combo);
            w.set_hotkey_error(SharedString::new());
        } else {
            apply_hotkey(&old);
            w.set_hotkey_error(format!("{combo} is already used by another app").into());
        }
    });

    let weak = w.as_weak();
    w.on_changed(move || {
        let Some(w) = weak.upgrade() else { return };
        let before = current_settings();
        update(|s| {
            s.desktop_typing = w.get_desktop_typing();
            s.position = if w.get_position() == 1 { "center" } else { "top" }.into();
            s.default_category = w.get_default_category();
            s.start_with_windows = w.get_start_with_windows();
            s.open_location_key = w.get_open_location_key();
            s.category_key = w.get_category_key();
        });
        let after = current_settings();
        #[cfg(windows)]
        {
            if before.desktop_typing != after.desktop_typing {
                crate::win::trigger::set_desktop_typing(after.desktop_typing);
            }
            if before.start_with_windows != after.start_with_windows
                && !crate::win::shell::set_autostart(after.start_with_windows)
            {
                update(|s| s.start_with_windows = before.start_with_windows);
                w.set_start_with_windows(before.start_with_windows);
            }
        }
        #[cfg(not(windows))]
        let _ = (before, after);
    });
    w
}

#[cfg(windows)]
fn apply_hotkey(combo: &str) -> bool {
    crate::win::trigger::parse_hotkey(combo).is_some_and(crate::win::trigger::set_hotkey)
}

#[cfg(not(windows))]
fn apply_hotkey(_: &str) -> bool {
    true
}

fn update(f: impl FnOnce(&mut crate::settings::Settings)) {
    SETTINGS.with(|s| {
        let mut s = s.borrow_mut();
        f(&mut s);
        s.save();
    });
}

#[cfg(test)]
mod tests {
    use super::describe_hotkey;

    #[test]
    fn hotkeys() {
        assert_eq!(describe_hotkey(" ", false, true, false, false), "Alt+Space");
        assert_eq!(describe_hotkey("f", true, false, true, false), "Ctrl+Shift+F");
        assert_eq!(describe_hotkey("f", false, false, true, false), "");
        assert_eq!(describe_hotkey("ک", true, false, false, false), "");
    }
}
