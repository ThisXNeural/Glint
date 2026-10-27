//! The settings window: edits [`Settings`] and applies each change at once.

use crate::settings::Shortcut;
use crate::{current_settings, AppChoice, CustomShortcut, Shared, SettingsWindow, Theme, SETTINGS, SETTINGS_WINDOW};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;
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
        w.set_recording_shortcut(-1);
        show_shortcuts(w, &s.shortcuts, &[]);
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

thread_local! {
    /// Apps behind the picker's current list: (name, launch target).
    static PICKER_APPS: std::cell::RefCell<Vec<(String, String)>> = Default::default();
    /// Icons already drawn for the picker and chips, by launch target.
    static ICONS: std::cell::RefCell<std::collections::HashMap<String, slint::Image>> = Default::default();
}

fn app_icon(name: &str, target: &str) -> slint::Image {
    ICONS.with(|cache| {
        cache
            .borrow_mut()
            .entry(target.to_string())
            .or_insert_with(|| {
                let row = crate::engine::Row {
                    title: name.to_string(),
                    subtitle: String::new(),
                    target: target.to_string(),
                    kind: crate::engine::RowKind::App,
                    matched: None,
                };
                crate::icon_fast(&row).map(|i| crate::to_image(&i)).unwrap_or_default()
            })
            .clone()
    })
}

/// The launch target of a picked app, quoted so paths with spaces survive.
fn app_command(target: &str) -> String {
    format!("\"{target}\"")
}

/// Fills the picker with the apps matching `filter` (all when empty).
fn filter_apps(w: &SettingsWindow, shared: &Shared, filter: &str) {
    let apps = shared.engine.apps.read().unwrap();
    let list: Vec<(String, String)> = if filter.trim().is_empty() {
        apps.all().iter().map(|a| (a.name.clone(), a.target.clone())).collect()
    } else {
        apps.search(filter, 200).into_iter().map(|a| (a.name.clone(), a.target.clone())).collect()
    };
    drop(apps);
    let rows: Vec<AppChoice> =
        list.iter().map(|(name, target)| AppChoice { name: name.as_str().into(), icon: app_icon(name, target) }).collect();
    PICKER_APPS.with(|p| *p.borrow_mut() = list);
    w.set_app_choices(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn create(shared: Arc<Shared>) -> SettingsWindow {
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
    w.on_shortcut_hotkey_chosen(move |i, combo| {
        let Some(w) = weak.upgrade() else { return };
        edit_shortcuts(&w, |list| {
            if let Some(s) = list.get_mut(i as usize) {
                s.hotkey = combo.to_string();
            }
        });
    });
    let weak = w.as_weak();
    w.on_shortcut_edited(move |i, command, admin| {
        let Some(w) = weak.upgrade() else { return };
        edit_shortcuts(&w, |list| {
            if let Some(s) = list.get_mut(i as usize) {
                let command = command.trim().to_string();
                if command != s.command {
                    s.app.clear();
                }
                s.command = command;
                s.admin = admin;
            }
        });
    });
    let weak = w.as_weak();
    let sh = shared.clone();
    w.on_filter_apps(move |filter| {
        let Some(w) = weak.upgrade() else { return };
        filter_apps(&w, &sh, &filter);
    });
    let weak = w.as_weak();
    w.on_app_picked(move |i, k| {
        let Some(w) = weak.upgrade() else { return };
        let Some((name, target)) = PICKER_APPS.with(|p| p.borrow().get(k as usize).cloned()) else { return };
        edit_shortcuts(&w, |list| {
            if let Some(s) = list.get_mut(i as usize) {
                s.command = app_command(&target);
                s.app = name;
            }
        });
    });
    let weak = w.as_weak();
    w.on_add_shortcut(move || {
        let Some(w) = weak.upgrade() else { return };
        edit_shortcuts(&w, |list| list.push(Shortcut { hotkey: String::new(), command: String::new(), admin: false, app: String::new() }));
    });
    let weak = w.as_weak();
    w.on_remove_shortcut(move |i| {
        let Some(w) = weak.upgrade() else { return };
        edit_shortcuts(&w, |list| {
            if (i as usize) < list.len() {
                list.remove(i as usize);
            }
        });
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

/// Changes the custom shortcuts, saves them, registers them again and shows
/// which ones failed. Shortcuts without keys or a command are kept but idle.
fn edit_shortcuts(w: &SettingsWindow, f: impl FnOnce(&mut Vec<Shortcut>)) {
    let mut list = current_settings().shortcuts;
    let before = list.clone();
    f(&mut list);
    update(|s| s.shortcuts = list.clone());
    let ok = apply_shortcuts(&list);
    // Typing in a command box only needs saving; rebuilding the rows would
    // take the focus away from the box being typed in.
    if list.len() != before.len() || list.iter().zip(&before).any(|(a, b)| a.hotkey != b.hotkey || a.app != b.app) {
        show_shortcuts(w, &list, &ok);
    } else {
        let model = w.get_shortcuts();
        for (i, okay) in ok.iter().enumerate() {
            if let Some(mut row) = model.row_data(i) {
                let error = error_text(&list[i], *okay);
                if row.error != error {
                    row.error = error;
                    model.set_row_data(i, row);
                }
            }
        }
    }
}

fn error_text(s: &Shortcut, ok: bool) -> SharedString {
    if ok || s.hotkey.is_empty() || s.command.is_empty() {
        SharedString::new()
    } else {
        format!("{} is used by another app; pick other keys", s.hotkey).into()
    }
}

fn show_shortcuts(w: &SettingsWindow, list: &[Shortcut], ok: &[bool]) {
    let rows: Vec<CustomShortcut> = list
        .iter()
        .enumerate()
        .map(|(i, s)| CustomShortcut {
            hotkey: s.hotkey.as_str().into(),
            command: s.command.as_str().into(),
            admin: s.admin,
            error: error_text(s, ok.get(i).copied().unwrap_or(true)),
            app_name: s.app.as_str().into(),
            app_icon: if s.app.is_empty() {
                slint::Image::default()
            } else {
                app_icon(&s.app, s.command.trim_matches('"'))
            },
        })
        .collect();
    w.set_shortcuts(ModelRc::from(Rc::new(VecModel::from(rows))));
}

/// Registers the shortcuts that have keys and a command; returns per shortcut
/// whether it is active (true for incomplete ones, which aren't errors).
#[cfg(windows)]
pub fn apply_shortcuts(list: &[Shortcut]) -> Vec<bool> {
    use crate::win::trigger;
    let keys = list
        .iter()
        .map(|s| if s.command.is_empty() { None } else { trigger::parse_hotkey(&s.hotkey) })
        .collect::<Vec<_>>();
    let incomplete: Vec<bool> = keys.iter().map(Option::is_none).collect();
    trigger::set_shortcuts(keys).into_iter().zip(incomplete).map(|(ok, idle)| ok || idle).collect()
}

#[cfg(not(windows))]
pub fn apply_shortcuts(list: &[Shortcut]) -> Vec<bool> {
    vec![true; list.len()]
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
