//! Lists everything in the shell's Apps folder: Start menu shortcuts and
//! Store apps, the same set the Start menu's "All apps" shows.

use glint_core::App;
use windows::core::PWSTR;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
    KF_FLAG_DEFAULT, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

/// Launch target for an Apps folder entry.
pub fn target(parsing_name: &str) -> String {
    format!("shell:AppsFolder\\{parsing_name}")
}

pub fn list() -> Vec<App> {
    let apps = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        list_inner().unwrap_or_default()
    };
    if apps.is_empty() { start_menu_shortcuts() } else { apps }
}

/// Fallback when the Apps folder can't be listed: the Start menu's shortcut
/// files for all users and for this user.
fn start_menu_shortcuts() -> Vec<App> {
    let roots = [
        std::env::var_os("ProgramData").map(|p| std::path::PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
        std::env::var_os("APPDATA").map(|p| std::path::PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
    ];
    let mut apps = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = roots.into_iter().flatten().collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_lnk = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk"));
            let Some(name) = path.file_stem().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let target = path.to_string_lossy().into_owned();
            if is_lnk && !is_noise(&name, &target) {
                apps.push(App { name, target, launches: 0 });
            }
        }
    }
    apps
}

unsafe fn list_inner() -> windows::core::Result<Vec<App>> {
    let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
    let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
    let mut apps = Vec::new();
    loop {
        let mut batch: [Option<IShellItem>; 32] = Default::default();
        let mut fetched = 0u32;
        if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
            break;
        }
        for item in batch.iter().take(fetched as usize).flatten() {
            let (Some(name), Some(parsing)) = (display_name(item, SIGDN_NORMALDISPLAY), display_name(item, SIGDN_PARENTRELATIVEPARSING)) else {
                continue;
            };
            if is_noise(&name, &parsing) {
                continue;
            }
            apps.push(App { name, target: target(&parsing), launches: 0 });
        }
    }
    Ok(apps)
}

unsafe fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    let p: PWSTR = item.GetDisplayName(kind).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.as_ptr() as *const _));
    s
}

/// Help files, web links and uninstallers clutter app results.
fn is_noise(name: &str, parsing: &str) -> bool {
    let p = parsing.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    [".url", ".chm", ".txt", ".pdf", ".html", ".htm", ".rtf"].iter().any(|e| p.ends_with(e))
        || p.starts_with("http")
        || n.starts_with("uninstall")
}
