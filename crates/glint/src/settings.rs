//! Settings and launch counts, stored as small text files in
//! %APPDATA%\Glint so they are easy to read and edit by hand.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Shortcut that opens search from any app, e.g. "Alt+Space".
    pub hotkey: String,
    /// Typing on the bare desktop opens search.
    pub desktop_typing: bool,
    /// "top" or "center".
    pub position: String,
    /// Category selected when the window opens: 0 all, 1 apps, 2 folders, 3 files.
    pub default_category: i32,
    /// Starts hidden at sign-in.
    pub start_with_windows: bool,
    /// Index into [`OPEN_LOCATION_KEYS`].
    pub open_location_key: i32,
    /// Index into [`CATEGORY_KEYS`].
    pub category_key: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: "Alt+Space".into(), desktop_typing: true, position: "top".into(), default_category: 1, start_with_windows: true, open_location_key: 0, category_key: 0 }
    }
}

/// `show_first` values, in tab order.
const CATEGORY_NAMES: [&str; 4] = ["all", "apps", "folders", "files"];

/// Choices for the keys used inside the search window. The settings window
/// lists them in this order.
pub const OPEN_LOCATION_KEYS: [&str; 3] = ["Ctrl+Enter", "Shift+Enter", "Alt+Enter"];
pub const CATEGORY_KEYS: [&str; 3] = ["Tab", "Ctrl+Tab", "Ctrl+Right"];

fn key_index(keys: &[&str], v: &str) -> i32 {
    keys.iter().position(|k| k.eq_ignore_ascii_case(v)).map_or(0, |i| i as i32)
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("Glint");
    // Versions before the rename kept their files in SiloSearch.
    let old = base.join("SiloSearch");
    if !dir.exists() && old.exists() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

fn settings_path() -> PathBuf {
    dir().join("settings.ini")
}

fn usage_path() -> PathBuf {
    dir().join("usage.txt")
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Ok(text) = std::fs::read_to_string(settings_path()) else {
            s.save();
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "hotkey" => s.hotkey = v.to_string(),
                "desktop_typing" => s.desktop_typing = v != "false",
                "position" => s.position = v.to_string(),
                "show_first" => {
                    s.default_category = CATEGORY_NAMES.iter().position(|n| *n == v).map_or(1, |i| i as i32)
                }
                // Before "All" existed: 0 apps, 1 folders, 2 files.
                "default_category" => s.default_category = v.parse::<i32>().map_or(1, |c| c.clamp(0, 2) + 1),
                "start_with_windows" => s.start_with_windows = v != "false",
                "open_location_key" => s.open_location_key = key_index(&OPEN_LOCATION_KEYS, v),
                "category_key" => s.category_key = key_index(&CATEGORY_KEYS, v),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(dir());
        let text = format!(
            "# Glint settings. Easiest to change from the settings window (Ctrl+, in search).\n\
             hotkey={}\ndesktop_typing={}\nposition={}\nshow_first={}\nstart_with_windows={}\n\
             open_location_key={}\ncategory_key={}\n",
            self.hotkey,
            self.desktop_typing,
            self.position,
            CATEGORY_NAMES.get(self.default_category as usize).unwrap_or(&"apps"),
            self.start_with_windows,
            OPEN_LOCATION_KEYS[self.open_location_key as usize],
            CATEGORY_KEYS[self.category_key as usize],
        );
        let _ = std::fs::write(settings_path(), text);
    }
}

/// Launch target -> times launched.
pub fn load_usage() -> HashMap<String, u32> {
    std::fs::read_to_string(usage_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (n, t) = l.split_once('\t')?;
            Some((t.to_string(), n.parse().ok()?))
        })
        .collect()
}

pub fn save_usage(usage: &HashMap<String, u32>) {
    let _ = std::fs::create_dir_all(dir());
    let text: String = usage.iter().map(|(t, n)| format!("{n}\t{t}\n")).collect();
    let _ = std::fs::write(usage_path(), text);
}
