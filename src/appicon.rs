//! A button that launches a program shows that program's own icon unless the owner set a picture or symbol.
//! The icon comes from the Windows shell (the same 256 px "jumbo" icon Explorer shows), cropped to its visible part.

use crate::config::{Action, Button};
use image::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The program a button's tap action starts, if it is a plain app (`run` of an exe, or `open` of an exe / shortcut).
pub fn target(b: &Button) -> Option<PathBuf> {
    match b.action.as_ref()? {
        Action::Run { command, .. } => resolve(command),
        Action::Open { target } => {
            let p = resolve_file(target.trim().trim_matches('"'))?;
            let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
            matches!(ext.as_str(), "exe" | "lnk").then_some(p)
        }
        _ => None,
    }
}

fn resolve_file(s: &str) -> Option<PathBuf> {
    let expanded = expand_env(s);
    let p = PathBuf::from(&expanded);
    p.is_file().then_some(p)
}

/// "%ProgramFiles%\\x.exe" -> real path (only the %NAME% form).
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        match tail.find('%') {
            Some(j) => match std::env::var(&tail[..j]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &tail[j + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = tail;
                }
            },
            None => {
                out.push('%');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A `run` command: a path, or a bare name like "notepad" / "notepad.exe" found on PATH.
fn resolve(command: &str) -> Option<PathBuf> {
    let c = command.trim().trim_matches('"');
    if c.is_empty() {
        return None;
    }
    if let Some(p) = resolve_file(c) {
        return Some(p);
    }
    if c.contains(['\\', '/']) {
        return resolve_file(&format!("{c}.exe"));
    }
    let names: Vec<String> = if Path::new(c).extension().is_some() { vec![c.to_string()] } else { vec![format!("{c}.exe"), format!("{c}.lnk")] };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for n in &names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The icon of the program a button starts, cropped to its visible part and padded to a square. None if it cannot be read.
pub fn image_for(b: &Button) -> Option<Arc<RgbaImage>> {
    let path = target(b)?;
    static CACHE: Mutex<Option<HashMap<PathBuf, Option<Arc<RgbaImage>>>>> = Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = map.get(&path) {
        return hit.clone();
    }
    let img = shell_icon(&path).and_then(square_crop).map(Arc::new);
    map.insert(path, img.clone());
    img
}

/// Store-style icons are a glyph on an opaque square plate. If all four corners are opaque and (nearly) the same color, that
/// color: the tile can take it as its background so no square shows.
pub fn plate_color(img: &RgbaImage) -> Option<[u8; 3]> {
    let (w, h) = img.dimensions();
    let c = [img.get_pixel(1, 1), img.get_pixel(w - 2, 1), img.get_pixel(1, h - 2), img.get_pixel(w - 2, h - 2)];
    if c.iter().any(|p| p.0[3] < 240) {
        return None;
    }
    let near = |a: &image::Rgba<u8>, b: &image::Rgba<u8>| (0..3).all(|i| (a.0[i] as i32 - b.0[i] as i32).abs() <= 14);
    c.iter().all(|p| near(p, c[0])).then(|| {
        let avg = |i: usize| (c.iter().map(|p| p.0[i] as u32).sum::<u32>() / 4) as u8;
        [avg(0), avg(1), avg(2)]
    })
}

/// Crop to the non-transparent bounding box and pad to a square (some icons sit small inside a big transparent canvas).
fn square_crop(img: RgbaImage) -> Option<RgbaImage> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 8 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 || y1 < y0 {
        return None;
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let side = cw.max(ch);
    let mut out = RgbaImage::new(side, side);
    image::imageops::overlay(&mut out, &image::imageops::crop_imm(&img, x0, y0, cw, ch).to_image(), ((side - cw) / 2) as i64, ((side - ch) / 2) as i64);
    Some(out)
}

fn shell_icon(path: &Path) -> Option<RgbaImage> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHIL_JUMBO};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // fine if the thread already did it
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(PCWSTR(wide.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX);
        if ok == 0 {
            return None;
        }
        let list: IImageList = SHGetImageList(SHIL_JUMBO as i32).ok()?;
        let icon = list.GetIcon(info.iIcon, ILD_TRANSPARENT.0 as u32).ok()?;
        let mut ii = ICONINFO::default();
        if GetIconInfo(icon, &mut ii).is_err() {
            let _ = DestroyIcon(icon);
            return None;
        }
        let read = |bmp: HBITMAP| -> Option<(u32, u32, Vec<u8>)> {
            let mut bm = BITMAP::default();
            if GetObjectW(bmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w as i32;
            bi.bmiHeader.biHeight = -(h as i32); // top-down
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let dc = GetDC(None);
            let lines = GetDIBits(dc, bmp, 0, h, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
            ReleaseDC(None, dc);
            (lines != 0).then_some((w, h, buf))
        };
        let color = read(ii.hbmColor);
        let mask = if ii.hbmMask.is_invalid() { None } else { read(ii.hbmMask) };
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }
        let _ = DestroyIcon(icon);
        let (w, h, mut px) = color?;
        // BGRA -> RGBA
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        if px.chunks_exact(4).all(|p| p[3] == 0) {
            // no alpha channel in the color bitmap: the mask says which pixels are visible (black = opaque)
            match mask {
                Some((mw, mh, m)) if mw == w && mh == h => {
                    for (p, mp) in px.chunks_exact_mut(4).zip(m.chunks_exact(4)) {
                        p[3] = if mp[0] == 0 { 255 } else { 0 };
                    }
                }
                _ => px.chunks_exact_mut(4).for_each(|p| p[3] = 255),
            }
        }
        RgbaImage::from_raw(w, h, px)
    }
}
