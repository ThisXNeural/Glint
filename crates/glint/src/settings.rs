//! Settings and launch counts, stored as small text files in
//! %APPDATA%\Glint so they are easy to read and edit by hand.

use std::collections::HashMap;
use std::path::PathBuf;

/// A global shortcut that runs a command, e.g. Ctrl+Alt+T for a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub hotkey: String,
    pub command: String,
    pub admin: bool,
    /// Name of the app picked from the list, shown instead of the command.
    pub app: String,
}

impl Shortcut {
    /// `Ctrl+Alt+T | wt.exe | admin`, or with a picked app
    /// `Ctrl+Alt+C | "shell:AppsFolder\Chrome" | | Google Chrome`.
    fn parse(v: &str) -> Option<Self> {
        let mut parts = v.splitn(4, '|').map(str::trim);
        let hotkey = parts.next()?.to_string();
        let command = parts.next()?.to_string();
        let admin = parts.next().is_some_and(|a| a.eq_ignore_ascii_case("admin"));
        let app = parts.next().unwrap_or_default().to_string();
        (!command.is_empty()).then_some(Self { hotkey, command, admin, app })
    }

    fn line(&self) -> String {
        let mut line = format!("shortcut={} | {}", self.hotkey, self.command);
        if self.admin || !self.app.is_empty() {
            line += if self.admin { " | admin" } else { " |" };
        }
        if !self.app.is_empty() {
            line += &format!(" | {}", self.app);
        }
        line + "\n"
    }

    pub fn terminal_as_admin() -> Self {
        Self { hotkey: "Ctrl+Alt+T".into(), command: "wt.exe".into(), admin: true, app: String::new() }
    }
}

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
    /// Global shortcuts that run commands.
    pub shortcuts: Vec<Shortcut>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: "Alt+Space".into(), desktop_typing: true, position: "top".into(), default_category: 1, start_with_windows: true, open_location_key: 0, category_key: 0, shortcuts: vec![Shortcut::terminal_as_admin()] }
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
        let mut shortcuts: Option<Vec<Shortcut>> = None;
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
                // A bare "shortcut=" means the list was emptied on purpose;
                // files without any keep the default terminal shortcut.
                "shortcut" => shortcuts.get_or_insert_with(Vec::new).extend(Shortcut::parse(v)),
                _ => {}
            }
        }
        if let Some(list) = shortcuts {
            s.shortcuts = list;
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
        ) + &if self.shortcuts.is_empty() {
            "shortcut=\n".to_string()
        } else {
            self.shortcuts.iter().map(Shortcut::line).collect()
        };
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

#[cfg(test)]
mod tests {
    use super::Shortcut;

    #[test]
    fn shortcut_lines() {
        let s = Shortcut::terminal_as_admin();
        let line = s.line();
        assert_eq!(line, "shortcut=Ctrl+Alt+T | wt.exe | admin\n");
        assert_eq!(Shortcut::parse(line.trim_end().strip_prefix("shortcut=").unwrap()), Some(s));
        let chrome = Shortcut { hotkey: "Ctrl+Alt+C".into(), command: "\"C:\\x\\Chrome.lnk\"".into(), admin: false, app: "Google Chrome".into() };
        let line = chrome.line();
        assert_eq!(line, "shortcut=Ctrl+Alt+C | \"C:\\x\\Chrome.lnk\" | | Google Chrome\n");
        assert_eq!(Shortcut::parse(line.trim_end().strip_prefix("shortcut=").unwrap()), Some(chrome));
        assert_eq!(Shortcut::parse(""), None);
    }
}
