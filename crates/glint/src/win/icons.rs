//! Shell icons as RGBA pixels, cached so each icon is extracted once.

use super::{pcwstr, wide};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
    SHGFI_USEFILEATTRIBUTES, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

/// Straight (not premultiplied) RGBA, row-major, `size` x `size`.
#[derive(Clone)]
pub struct Pixels {
    pub size: u32,
    pub rgba: Arc<Vec<u8>>,
}

// 24px rows stay sharp up to 200% display scaling.
pub const SIZE_PX: i32 = 48;

fn cache() -> &'static Mutex<HashMap<String, Option<Pixels>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Pixels>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn cached(key: String, load: impl FnOnce() -> Option<Pixels>) -> Option<Pixels> {
    if let Some(p) = cache().lock().unwrap().get(&key) {
        return p.clone();
    }
    let p = load();
    let mut c = cache().lock().unwrap();
    // Per-folder icons add up; start over rather than grow without bound.
    if c.len() > 1500 {
        c.clear();
    }
    c.insert(key, p.clone());
    p
}

/// Icon of an app or a specific file, via the shell's image factory.
/// The calling thread must have COM initialized.
pub fn for_item(parsing_name: &str) -> Option<Pixels> {
    cached(parsing_name.to_string(), || unsafe {
        let w = wide(parsing_name);
        let from_factory = || {
            let factory: IShellItemImageFactory = SHCreateItemFromParsingName(pcwstr(&w), None).ok()?;
            let bmp = factory.GetImage(SIZE { cx: SIZE_PX, cy: SIZE_PX }, SIIGBF_ICONONLY).ok()?;
            let p = hbitmap_pixels(bmp, true);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            p
        };
        // The classic API covers files and shortcuts the factory can't draw.
        let from_file_info = || {
            let mut info = SHFILEINFOW::default();
            let ok = SHGetFileInfoW(
                pcwstr(&w),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            );
            if ok == 0 || info.hIcon.is_invalid() {
                return None;
            }
            let p = hicon_pixels(info.hIcon);
            let _ = DestroyIcon(info.hIcon);
            p
        };
        from_factory().or_else(from_file_info)
    })
}

/// Generic icon for a folder or a file type, by extension. Cheap: no disk access.
pub fn for_kind(is_dir: bool, name: &str) -> Option<Pixels> {
    let ext = if is_dir {
        String::new()
    } else {
        name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
    };
    // Executables, shortcuts and icons each look different: use the real file.
    if matches!(ext.as_str(), "exe" | "lnk" | "ico" | "url" | "appref-ms") {
        return None;
    }
    let key = if is_dir { "<dir>".to_string() } else { format!("*.{ext}") };
    cached(key, || unsafe {
        let w = wide(if is_dir { "folder" } else { &name });
        let mut info = SHFILEINFOW::default();
        let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
        let ok = SHGetFileInfoW(
            pcwstr(&w),
            attrs,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let p = hicon_pixels(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        p
    })
}

unsafe fn hicon_pixels(icon: HICON) -> Option<Pixels> {
    let mut ii = ICONINFO::default();
    GetIconInfo(icon, &mut ii).ok()?;
    let p = hbitmap_pixels(ii.hbmColor, true);
    let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
    let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
    p
}

/// Reads a 32-bit bitmap. Shell bitmaps carry premultiplied BGRA.
unsafe fn hbitmap_pixels(bmp: HBITMAP, premultiplied: bool) -> Option<Pixels> {
    let mut bm = BITMAP::default();
    if GetObjectW(HGDIOBJ(bmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 || w > 256 {
        return None;
    }
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let dc = GetDC(None);
    let rows = GetDIBits(dc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    if rows == 0 {
        return None;
    }
    if buf.iter().all(|&b| b == 0) {
        return None; // nothing was drawn; better no icon than a black square
    }
    let has_alpha = buf.chunks_exact(4).any(|p| p[3] != 0);
    for p in buf.chunks_exact_mut(4) {
        p.swap(0, 2); // BGRA -> RGBA
        if !has_alpha {
            p[3] = 255;
        } else if premultiplied && p[3] != 0 && p[3] != 255 {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    // Icons are square; keep the top-left square if not.
    let size = w.min(h) as u32;
    if w != h {
        let mut sq = Vec::with_capacity((size * size * 4) as usize);
        for row in buf.chunks_exact((w * 4) as usize).take(size as usize) {
            sq.extend_from_slice(&row[..(size * 4) as usize]);
        }
        buf = sq;
    }
    Some(Pixels { size, rgba: Arc::new(buf) })
}

