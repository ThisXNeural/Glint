//! Instant search for Windows. Sleeps in the background; opens when you type
//! on the bare desktop or press the hotkey anywhere else.
#![cfg_attr(windows, windows_subsystem = "windows")]

slint::include_modules!();

mod engine;
mod settings;
mod settings_ui;
#[cfg(windows)]
mod win;

use engine::{Category, Engine, Row, RowKind};
use settings::Settings;
use slint::{ComponentHandle, Model, ModelRc, SharedPixelBuffer, VecModel};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Icon pixels that can cross threads; turned into a `slint::Image` on the UI thread.
#[derive(Clone)]
pub struct Icon {
    pub size: u32,
    pub rgba: Arc<Vec<u8>>,
}

struct Query {
    generation: u64,
    text: String,
    category: Category,
}

/// Shared between the UI thread, the search worker and the indexers.
pub struct Shared {
    engine: Engine,
    /// Bumped on every keystroke; stale results are dropped.
    generation: AtomicU64,
    pub status: Mutex<String>,
    usage: Mutex<HashMap<String, u32>>,
}

fn main() {
    install_crash_log();
    // Autostart passes --background; any other launch shows the window.
    let background = std::env::args().any(|a| a == "--background");
    #[cfg(windows)]
    let show_event = {
        use win::shell;
        if shell::signal_show() {
            return; // already running; it shows itself
        }
        // Admin install: start the full-rights copy instead of a limited one.
        if !shell::is_elevated() && shell::has_elevated_task() && shell::run_elevated_task() {
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if background || shell::signal_show() {
                    return;
                }
            }
        }
        match shell::claim_single_instance() {
            Some(ev) => ev,
            None => return,
        }
    };
    // A CPU-drawn window this small renders in about a millisecond and avoids
    // the tens of megabytes a GPU context would keep resident.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }
    let settings = Settings::load();
    SETTINGS.with(|s| *s.borrow_mut() = settings.clone());
    let shared = Arc::new(Shared {
        engine: Engine::default(),
        generation: AtomicU64::new(0),
        status: Mutex::new("Indexing…".into()),
        usage: Mutex::new(settings::load_usage()),
    });

    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    let ui = SearchWindow::new().expect("create window");
    let rows: Rc<VecModel<ResultItem>> = Rc::new(VecModel::default());
    let current_rows: Rc<std::cell::RefCell<Vec<Row>>> = Default::default();
    ui.set_results(ModelRc::from(rows.clone()));

    let queries = start_search_worker(shared.clone(), ui.as_weak());
    start_indexing(shared.clone());

    let send_query = {
        let shared = shared.clone();
        let ui = ui.as_weak();
        move || {
            let ui = ui.unwrap();
            let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = queries.send(Query {
                generation,
                text: ui.get_query().to_string(),
                category: Category::from_index(ui.get_category()),
            });
        }
    };
    let send_query = Rc::new(send_query);

    {
        let sq = send_query.clone();
        ui.on_query_changed(move |_| sq());
    }
    {
        let sq = send_query.clone();
        ui.on_category_changed(move |_| sq());
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_dismiss(move || hide(&ui_weak.unwrap()));
    }
    {
        let ui_weak = ui.as_weak();
        let shared = shared.clone();
        ui.on_open_settings(move || {
            hide(&ui_weak.unwrap());
            settings_ui::open(&shared);
        });
    }
    {
        let current_rows = current_rows.clone();
        ui.on_step(move |from, delta| step(&current_rows.borrow(), from, delta));
    }
    {
        let ui_weak = ui.as_weak();
        let shared = shared.clone();
        let current_rows = current_rows.clone();
        ui.on_activate(move |i, containing_folder| {
            let Some(row) = current_rows.borrow().get(i as usize).cloned() else { return };
            if row.kind == RowKind::Header {
                return;
            }
            hide(&ui_weak.unwrap());
            open_row(&shared, &row, containing_folder);
        });
    }

    // Results arrive from the worker through this callback on the UI thread.
    RESULTS.with(|r| {
        let rows = rows.clone();
        let current_rows = current_rows.clone();
        let ui_weak = ui.as_weak();
        let shared = shared.clone();
        *r.borrow_mut() = Some(Box::new(move |generation, list: Vec<(Row, Option<Icon>)>| {
            if generation != shared.generation.load(Ordering::SeqCst) {
                return;
            }
            let mut y = 0.0;
            let items: Vec<ResultItem> = list
                .iter()
                .map(|(row, icon)| {
                    let item = to_item(row, icon.as_ref(), y);
                    y += item.height;
                    item
                })
                .collect();
            rows.set_vec(items);
            *current_rows.borrow_mut() = list.into_iter().map(|(r, _)| r).collect();
            if let Some(ui) = ui_weak.upgrade() {
                let cur = ui.get_current();
                let selectable = |i: i32| current_rows.borrow().get(i as usize).is_some_and(|r| r.kind != RowKind::Header);
                if !selectable(cur) {
                    ui.set_current(step(&current_rows.borrow(), -1, 1));
                }
            }
        }));
    });

    // Exact icons that finished loading after the first paint.
    ICON_UPDATES.with(|r| {
        let rows = rows.clone();
        let shared = shared.clone();
        *r.borrow_mut() = Some(Box::new(move |generation, index, icon: Icon| {
            if generation != shared.generation.load(Ordering::SeqCst) {
                return;
            }
            if let Some(mut item) = rows.row_data(index) {
                item.icon = to_image(&icon);
                rows.set_row_data(index, item);
            }
        }));
    });

    #[cfg(windows)]
    win::shell::listen_for_show(show_event, || win::trigger::emit(win::trigger::Trigger::Show));
    platform_start(&ui, &settings, send_query, background);
    slint::run_event_loop_until_quit().expect("event loop");
    #[cfg(windows)]
    win::tray::remove();
}

thread_local! {
    #[allow(clippy::type_complexity)]
    static RESULTS: std::cell::RefCell<Option<Box<dyn Fn(u64, Vec<(Row, Option<Icon>)>)>>> = Default::default();
    static ICON_UPDATES: std::cell::RefCell<Option<Box<dyn Fn(u64, usize, Icon)>>> = Default::default();
}

/// Panics end up in %APPDATA%\\Glint\\crash.log, since the app has no console.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::create_dir_all(settings::dir());
        let bt = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::write(settings::dir().join("crash.log"), format!("{info}\n\n{bt}"));
        default(info);
    }));
}

const HEADER_HEIGHT: f32 = 30.0;
/// 48px row plus a 2px gap.
const ROW_HEIGHT: f32 = 50.0;

fn to_item(row: &Row, icon: Option<&Icon>, y: f32) -> ResultItem {
    let (before, hit, after) = match row.matched {
        Some((a, b)) => (&row.title[..a], &row.title[a..b], &row.title[b..]),
        None => ("", "", row.title.as_str()),
    };
    let is_header = row.kind == RowKind::Header;
    ResultItem {
        is_header,
        title_before: before.into(),
        title_match: hit.into(),
        title_after: after.into(),
        subtitle: row.subtitle.as_str().into(),
        tag: row.tag().into(),
        icon: icon.map(to_image).unwrap_or_default(),
        y,
        height: if is_header { HEADER_HEIGHT } else { ROW_HEIGHT },
    }
}

/// Index of the next selectable row after `from`, moving `delta` (±1)
/// and wrapping around; section headers are skipped.
fn step(rows: &[Row], from: i32, delta: i32) -> i32 {
    let n = rows.len() as i32;
    if n == 0 {
        return 0;
    }
    let mut i = from;
    for _ in 0..n {
        i = (i + delta).rem_euclid(n);
        if rows[i as usize].kind != RowKind::Header {
            return i;
        }
    }
    0
}

fn to_image(icon: &Icon) -> slint::Image {
    let buf = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&icon.rgba, icon.size, icon.size);
    slint::Image::from_rgba8(buf)
}

/// Runs queries off the UI thread, always skipping to the newest one.
fn start_search_worker(shared: Arc<Shared>, _ui: slint::Weak<SearchWindow>) -> mpsc::Sender<Query> {
    let (tx, rx) = mpsc::channel::<Query>();
    std::thread::Builder::new()
        .name("search".into())
        .spawn(move || {
            #[cfg(windows)]
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            }
            while let Ok(mut q) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    q = newer;
                }
                let rows = shared.engine.search(&q.text, q.category);
                let with_icons: Vec<(Row, Option<Icon>)> = rows.into_iter().map(|r| {
                    let icon = icon_fast(&r);
                    (r, icon)
                }).collect();
                // Rows whose real icon may differ from the quick one (folders with
                // their own icon, like Downloads or a OneDrive folder).
                let pending: Vec<(usize, Row)> = with_icons
                    .iter()
                    .enumerate()
                    .filter(|(_, (r, _))| r.kind == RowKind::Folder)
                    .map(|(i, (r, _))| (i, r.clone()))
                    .collect();
                let generation = q.generation;
                let _ = slint::invoke_from_event_loop(move || {
                    RESULTS.with(|r| {
                        if let Some(f) = r.borrow().as_ref() {
                            f(generation, with_icons);
                        }
                    })
                });
                for (index, row) in pending {
                    if shared.generation.load(Ordering::SeqCst) != generation {
                        break; // the user typed more; don't finish stale work
                    }
                    let Some(icon) = icon_exact(&row) else { continue };
                    let _ = slint::invoke_from_event_loop(move || {
                        ICON_UPDATES.with(|r| {
                            if let Some(f) = r.borrow().as_ref() {
                                f(generation, index, icon);
                            }
                        })
                    });
                }
            }
        })
        .unwrap();
    tx
}

/// Icon shown with the first paint. Apps and executables get their own icon
/// (cached after the first time); other files get their file type's icon.
#[cfg(windows)]
fn icon_fast(row: &Row) -> Option<Icon> {
    let px = match row.kind {
        RowKind::Header => return None,
        RowKind::App => win::icons::for_item(&row.target),
        RowKind::Folder => win::icons::for_kind(true, &row.title),
        RowKind::File => win::icons::for_kind(false, &row.title).or_else(|| win::icons::for_item(&row.target)),
    }?;
    Some(Icon { size: px.size, rgba: px.rgba })
}

/// The item's own icon, as Explorer shows it.
#[cfg(windows)]
fn icon_exact(row: &Row) -> Option<Icon> {
    let px = win::icons::for_item(&row.target)?;
    Some(Icon { size: px.size, rgba: px.rgba })
}

/// Previews read stand-in icons from $GLINT_DEMO_ICONS/<title>.rgba (48x48 RGBA).
#[cfg(not(windows))]
fn icon_fast(row: &Row) -> Option<Icon> {
    let dir = std::env::var("GLINT_DEMO_ICONS").ok()?;
    let key = match row.kind {
        RowKind::Header => return None,
        RowKind::Folder => "folder".to_string(),
        RowKind::File => row.title.rsplit_once('.').map_or("file".into(), |(_, e)| e.to_ascii_lowercase()),
        RowKind::App => row.title.clone(),
    };
    let rgba = std::fs::read(format!("{dir}/{key}.rgba")).ok()?;
    Some(Icon { size: 48, rgba: Arc::new(rgba) })
}

#[cfg(not(windows))]
fn icon_exact(_: &Row) -> Option<Icon> {
    None
}

fn open_row(shared: &Shared, row: &Row, containing_folder: bool) {
    if row.kind == RowKind::App {
        shared.engine.record_launch(row);
        let mut usage = shared.usage.lock().unwrap();
        *usage.entry(row.target.clone()).or_default() += 1;
        settings::save_usage(&usage);
    }
    #[cfg(windows)]
    {
        if containing_folder && row.kind != RowKind::App {
            win::shell::open_containing_folder(&row.target);
        } else {
            win::shell::open(&row.target);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = containing_folder;
        eprintln!("open {}", row.target);
    }
}

fn hide(ui: &SearchWindow) {
    let _ = ui.hide();
}

fn set_status(shared: &Shared, text: String) {
    *shared.status.lock().unwrap() = text;
}

fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Fills the app list and the file index in the background.
fn start_indexing(shared: Arc<Shared>) {
    std::thread::Builder::new()
        .name("index".into())
        .spawn(move || {
            refresh_apps(&shared);
            rerun_query();
            index_files(&shared);
            rerun_query();
        })
        .unwrap();
}

fn refresh_apps(shared: &Shared) {
    #[cfg(windows)]
    let mut apps = win::apps::list();
    #[cfg(not(windows))]
    let mut apps = demo_apps();
    let usage = shared.usage.lock().unwrap();
    for a in &mut apps {
        a.launches = usage.get(&a.target).copied().unwrap_or(0);
    }
    *shared.engine.apps.write().unwrap() = glint_core::AppIndex::new(apps);
}

#[cfg(windows)]
fn index_files(shared: &Arc<Shared>) {
    use glint_core::FileIndex;
    let started = std::time::Instant::now();
    let mut index = FileIndex::with_capacity(1 << 20, 24 << 20);
    let mut volumes = Vec::new();
    for drive in win::ntfs::ntfs_drives() {
        if let Ok(v) = win::ntfs::index_volume(&drive, &mut index) {
            volumes.push(v);
        }
    }
    let live = !volumes.is_empty();
    if !live {
        // Not elevated: walk the user's own folders instead (no live updates).
        if let Some(home) = std::env::var_os("USERPROFILE") {
            walk(&mut index, std::path::Path::new(&home));
        }
    }
    index.shrink_to_fit();
    let count = index.len();
    *shared.engine.files.write().unwrap() = index;
    set_status(
        shared,
        format!(
            "{} items · {:.1}s{}",
            format_count(count),
            started.elapsed().as_secs_f32(),
            if live { "" } else { " · run as administrator for all drives" }
        ),
    );
    for v in volumes {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("usn".into())
            .stack_size(256 << 10)
            .spawn(move || win::ntfs::watch(v, &shared.engine.files))
            .unwrap();
    }
}

#[cfg(not(windows))]
fn index_files(shared: &Arc<Shared>) {
    let mut index = glint_core::FileIndex::default();
    walk(&mut index, std::path::Path::new(&std::env::var("HOME").unwrap_or("/".into())));
    // Previews show a Windows-looking path instead of the real root.
    if let Ok(label) = std::env::var("GLINT_ROOT_LABEL") {
        index.rename(0, &label, glint_core::files::NO_PARENT);
    }
    let count = index.len();
    *shared.engine.files.write().unwrap() = index;
    set_status(shared, format!("{} items", format_count(count)));
}

/// Plain recursive walk, used when the NTFS fast path isn't available.
fn walk(index: &mut glint_core::FileIndex, root: &std::path::Path) {
    let root_id = index.push(&root.to_string_lossy(), glint_core::files::NO_PARENT, true);
    let mut stack = vec![(root.to_path_buf(), root_id)];
    while let Some((dir, id)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let child = index.push(&name, id, ft.is_dir());
            if ft.is_dir() {
                stack.push((e.path(), child));
            }
        }
    }
}

#[cfg(not(windows))]
fn demo_apps() -> Vec<glint_core::App> {
    ["Calculator", "Calendar", "Camera", "Google Chrome", "Microsoft Edge", "Notepad", "Paint", "Photos",
     "Settings", "Terminal", "Visual Studio Code", "Spotify", "Telegram", "File Explorer", "Snipping Tool",
     "Remote Desktop Connection", "Sound Recorder", "Windows PowerShell"]
        .iter()
        .map(|n| glint_core::App { name: n.to_string(), target: n.to_string(), launches: 0 })
        .collect()
}

/// Shows the window, optionally starting the query with typed text.
fn show(ui: &SearchWindow, settings: &Settings, typed: Option<String>, send_query: &dyn Fn()) {
    let visible = ui.window().is_visible();
    if visible {
        if let Some(t) = typed {
            ui.invoke_set_query(format!("{}{}", ui.get_query(), t).into());
            ui.set_current(0);
            send_query();
        }
        return;
    }
    apply_theme(ui);
    ui.set_open_location_key(settings.open_location_key);
    ui.set_category_key(settings.category_key);
    ui.set_category(settings.default_category);
    ui.set_current(0);
    ui.invoke_set_query(typed.unwrap_or_default().into());
    send_query();
    place(ui, settings);
    let _ = ui.show();
    #[cfg(windows)]
    if let Some(h) = win::window::hwnd(ui.window()) {
        if !STYLED.replace(true) {
            win::window::style(h, ui.global::<Theme>().get_dark());
            apply_theme(ui);
        }
        win::window::force_foreground(h);
    }
    ui.invoke_focus_input();
}

fn apply_theme(ui: &SearchWindow) {
    #[cfg(windows)]
    {
        let dark = win::shell::system_dark_mode();
        let (r, g, b) = win::shell::accent_color(dark);
        let win11 = win::shell::is_windows_11();
        let theme = ui.global::<Theme>();
        theme.set_dark(dark);
        theme.set_accent(slint::Color::from_rgb_u8(r, g, b));
        theme.set_font(if win11 { "Segoe UI Variable Text" } else { "Segoe UI" }.into());
        if let Some(h) = win::window::hwnd(ui.window()) {
            win::window::set_dark(h, dark);
            let on = win::shell::transparency_enabled();
            theme.set_translucent(win::window::set_acrylic(h, on, dark, win::shell::windows_build()));
        }
    }
    #[cfg(not(windows))]
    {
        // Preview knobs: GLINT_TRANSLUCENT=1 and GLINT_BACKDROP=#rrggbb let a
        // screenshot be composited over a wallpaper to show the acrylic look.
        let dark = std::env::var("GLINT_DARK").map_or(true, |v| v != "0");
        let theme = ui.global::<Theme>();
        theme.set_dark(dark);
        theme.set_translucent(std::env::var("GLINT_TRANSLUCENT").is_ok_and(|v| v == "1"));
        if let Some(hex) = std::env::var("GLINT_BACKDROP").ok().and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok()) {
            theme.set_backdrop(slint::Color::from_argb_encoded(0xff00_0000 | hex));
        }
        theme.set_accent(if dark { slint::Color::from_rgb_u8(0x4c, 0xc2, 0xff) } else { slint::Color::from_rgb_u8(0x00, 0x5f, 0xb8) });
    }
}

fn place(ui: &SearchWindow, settings: &Settings) {
    #[cfg(windows)]
    {
        let area = win::window::work_area_under_cursor();
        let scale = ui.window().scale_factor();
        let (w, h) = (680.0 * scale, 520.0 * scale);
        let aw = (area.right - area.left) as f32;
        let ah = (area.bottom - area.top) as f32;
        let x = area.left as f32 + (aw - w) / 2.0;
        let y = if settings.position == "center" { area.top as f32 + (ah - h) / 2.0 } else { area.top as f32 + ah * 0.16 };
        ui.window().set_position(slint::PhysicalPosition::new(x as i32, y as i32));
    }
    #[cfg(not(windows))]
    let _ = (ui, settings);
}

#[cfg(windows)]
fn platform_start(ui: &SearchWindow, settings: &Settings, send_query: Rc<dyn Fn()>, background: bool) {
    use win::trigger::{self, Trigger};

    let ui_weak = ui.as_weak();
    let ok = trigger::start(
        trigger::Config {
            desktop_typing: settings.desktop_typing,
            hotkey_label: settings.hotkey.clone(),
            hotkey: trigger::parse_hotkey(&settings.hotkey),
        },
        move |t: Trigger| {
            let ui_weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else { return };
                let settings = current_settings();
                SEND_QUERY.with(|sq| {
                    let sq = sq.borrow();
                    let Some(sq) = sq.as_ref() else { return };
                    match t {
                        Trigger::Typed(text) => show(&ui, &settings, Some(text), sq.as_ref()),
                        Trigger::Show => show(&ui, &settings, None, sq.as_ref()),
                        Trigger::Settings => {
                            hide(&ui);
                            SHARED.with(|s| {
                                if let Some(shared) = s.borrow().as_ref() {
                                    settings_ui::open(shared);
                                }
                            });
                        }
                        Trigger::Quit => {
                            let _ = slint::quit_event_loop();
                        }
                        Trigger::Hotkey => {
                            if ui.window().is_visible() {
                                hide(&ui);
                            } else {
                                show(&ui, &settings, None, sq.as_ref());
                            }
                        }
                        Trigger::ForegroundChanged(hwnd) => {
                            let ours = win::window::hwnd(ui.window()).map(|h| h.0 as isize);
                            if !PREWARMED.get() {
                                return;
                            }
                            if ui.window().is_visible() && ours != Some(hwnd) && hwnd != 0 {
                                hide(&ui);
                            }
                        }
                    }
                });
            });
        },
    );
    SEND_QUERY.with(|sq| *sq.borrow_mut() = Some(send_query.clone()));

    // Create and style the native window once, off-screen, so it never shows
    // a caption or taskbar button and the first real open is instant.
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(0), move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        // Not -32000: Windows reads that as "minimized".
        ui.window().set_position(slint::PhysicalPosition::new(-20000, -20000));
        let _ = ui.show();
        if let Some(h) = win::window::hwnd(ui.window()) {
            win::window::style(h, win::shell::system_dark_mode());
            STYLED.set(true);
        }
        let _ = ui.hide();
        PREWARMED.set(true);
        if !background {
            // Started by hand: show the window so it's clear the app is running.
            show(&ui, &current_settings(), None, send_query.as_ref());
        }
    });
    if !ok {
        ui.set_notice(format!("{} is used by another app. Press Ctrl+, to pick another shortcut.", settings.hotkey).into());
    }
}

thread_local! {
    static SEND_QUERY: std::cell::RefCell<Option<Rc<dyn Fn()>>> = Default::default();
    pub static SETTINGS: std::cell::RefCell<Settings> = Default::default();
    static SHARED: std::cell::RefCell<Option<Arc<Shared>>> = Default::default();
    pub static SETTINGS_WINDOW: std::cell::RefCell<Option<SettingsWindow>> = Default::default();
}

pub fn current_settings() -> Settings {
    SETTINGS.with(|s| s.borrow().clone())
}

/// Re-runs the current query, e.g. once an index finishes loading.
fn rerun_query() {
    let _ = slint::invoke_from_event_loop(|| {
        SEND_QUERY.with(|sq| {
            if let Some(sq) = sq.borrow().as_ref() {
                sq();
            }
        })
    });
}

#[cfg(windows)]
thread_local! {
    static STYLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PREWARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Elsewhere (development on Linux/macOS) just open the window right away.
#[cfg(not(windows))]
fn platform_start(ui: &SearchWindow, settings: &Settings, send_query: Rc<dyn Fn()>, _background: bool) {
    SEND_QUERY.with(|sq| *sq.borrow_mut() = Some(send_query.clone()));
    let initial = std::env::var("GLINT_QUERY").ok();
    let mut settings = settings.clone();
    if let Some(c) = std::env::var("GLINT_CATEGORY").ok().and_then(|c| c.parse().ok()) {
        settings.default_category = c;
    }
    if std::env::var("GLINT_SETTINGS").is_ok_and(|v| v == "1") {
        // Preview of the settings window alone.
        SHARED.with(|sh| sh.borrow().clone()).inspect(settings_ui::open);
        return;
    }
    show(ui, &settings, initial, send_query.as_ref());
}
