//! Notification-area (system tray) icon: the window hides here instead of closing, click the icon to bring it back,
//! right-click for Show / Quit. Plain Win32 on its own thread with a hidden window.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
const ID_SHOW: usize = 1;
const ID_QUIT: usize = 2;
const TITLE: &str = "deckd";

static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();
static TRAY_WND: AtomicUsize = AtomicUsize::new(0);
static ADDED: AtomicBool = AtomicBool::new(false);
/// The tray icon exists, so closing the window may hide it.
pub static AVAILABLE: AtomicBool = AtomicBool::new(false);
/// "Quit" was chosen: the next close request really closes.
pub static QUITTING: AtomicBool = AtomicBool::new(false);
/// The tray asked for the window; the UI thread acts on it.
pub static SHOW_REQUEST: AtomicBool = AtomicBool::new(false);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 32x32 RGBA of the app icon: a dark rounded square with a grid of colored keys. Used for the tray and the window.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let colors = [[0x4c, 0x9a, 0xff], [0xff, 0x9f, 0x43], [0x5f, 0xd3, 0x8d], [0xb5, 0x7b, 0xff], [0xff, 0x5f, 0x7a], [0x4c, 0xd4, 0xe0]];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            // rounded square
            let (dx, dy) = ((fx - 0.5).abs() - 0.5 + 0.18, (fy - 0.5).abs() - 0.5 + 0.18);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) - 0.18;
            if outside > 0.0 {
                continue;
            }
            let mut c = [0x1c, 0x1f, 0x26];
            // 3x2 grid of keys
            let (gx, gy) = ((fx - 0.14) / 0.72 * 3.0, (fy - 0.25) / 0.5 * 2.0);
            if (0.0..3.0).contains(&gx) && (0.0..2.0).contains(&gy) && gx.fract() > 0.12 && gx.fract() < 0.88 && gy.fract() > 0.14 && gy.fract() < 0.86 {
                c = colors[(gy as usize) * 3 + gx as usize];
            }
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    px
}

fn make_icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    let rgba = icon_rgba(32);
    let mut bgra = Vec::with_capacity(rgba.len());
    for p in rgba.chunks(4) {
        bgra.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    let mask = vec![0u8; 32 * 32 / 8];
    unsafe { CreateIcon(GetModuleHandleW(std::ptr::null()), 32, 32, 1, 32, mask.as_ptr(), bgra.as_ptr()) }
}

fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut n: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    n.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n
}

fn add_icon(hwnd: HWND) {
    let mut n = nid(hwnd);
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    n.uCallbackMessage = WM_TRAY;
    n.hIcon = make_icon();
    for (i, c) in "deckd".encode_utf16().enumerate() {
        n.szTip[i] = c;
    }
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &n) } != 0;
    ADDED.store(ok, Ordering::Relaxed);
}

/// Remove the icon (called on exit so no ghost icon stays behind).
pub fn remove() {
    let h = TRAY_WND.load(Ordering::Relaxed);
    if h != 0 && ADDED.swap(false, Ordering::Relaxed) {
        let n = nid(h as HWND);
        unsafe { Shell_NotifyIconW(NIM_DELETE, &n) };
    }
}

/// The main window of the running deckd (also found from a second copy of the program). FindWindow did not find it
/// while hidden, so walk the top-level windows and compare titles.
fn main_window() -> HWND {
    unsafe extern "system" fn each(h: HWND, found: LPARAM) -> i32 {
        let mut buf = [0u16; 16];
        let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) } as usize;
        if String::from_utf16_lossy(&buf[..n]) == TITLE {
            unsafe { *(found as *mut HWND) = h };
            return 0;
        }
        1
    }
    let mut found: HWND = std::ptr::null_mut();
    unsafe { EnumWindows(Some(each), &mut found as *mut HWND as LPARAM) };
    found
}

/// Bring the main window back (works while it is hidden or minimized).
pub fn show_main() {
    let h = main_window();
    if !h.is_null() {
        unsafe {
            ShowWindow(h, SW_SHOW);
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
    }
    SHOW_REQUEST.store(true, Ordering::Relaxed);
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

fn quit() {
    QUITTING.store(true, Ordering::Relaxed);
    if let Some(c) = CTX.get() {
        c.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        c.request_repaint();
    }
    let h = main_window();
    if !h.is_null() {
        unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
    if msg == WM_TRAY {
        match lp as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_main(),
            WM_RBUTTONUP => unsafe {
                let menu = CreatePopupMenu();
                AppendMenuW(menu, MF_STRING, ID_SHOW, wide("Show deckd").as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                AppendMenuW(menu, MF_STRING, ID_QUIT, wide("Quit").as_ptr());
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                SetForegroundWindow(hwnd); // otherwise the menu does not close when you click elsewhere
                let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
                PostMessageW(hwnd, WM_NULL, 0, 0);
                DestroyMenu(menu);
                match cmd {
                    ID_SHOW => show_main(),
                    ID_QUIT => quit(),
                    _ => {}
                }
            },
            _ => {}
        }
        return 0;
    }
    if msg == *TASKBAR_CREATED.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) }) {
        add_icon(hwnd); // Explorer restarted
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Start the tray icon on its own thread.
pub fn start(ctx: eframe::egui::Context) {
    let _ = CTX.set(ctx);
    let (ready, wait) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("deckd_tray");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(proc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("deckd tray").as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        if hwnd.is_null() {
            let _ = ready.send(());
            return;
        }
        TRAY_WND.store(hwnd as usize, Ordering::Relaxed);
        add_icon(hwnd);
        AVAILABLE.store(ADDED.load(Ordering::Relaxed), Ordering::Relaxed);
        let _ = ready.send(());
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    let _ = wait.recv_timeout(std::time::Duration::from_secs(3)); // so the UI knows whether the tray icon exists
}

/// Only one window at a time (two would fight over the deck). Returns false when another deckd window is already running,
/// after asking that one to come forward.
pub fn single_instance() -> bool {
    unsafe {
        let _ = CreateMutexW(std::ptr::null(), 0, wide("Local\\deckd-gui-instance").as_ptr()); // lives until the process ends
        if GetLastError() == ERROR_ALREADY_EXISTS {
            show_main();
            return false;
        }
    }
    true
}
