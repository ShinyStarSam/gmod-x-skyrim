//! Step 2d-2 (DESIGN.md "Step 2d in detail"): the Garry's Mod window is moved off-screen while the
//! overlay is on (the user's choice), and put back where it was when it switches off. Off-screen,
//! not minimized: a minimized Source game stops drawing. Moved with SetWindowPos: no resize, no
//! activation, no change of which window is in front.

use std::ffi::c_void;
use std::sync::Mutex;

type Hwnd = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(f: unsafe extern "system" fn(Hwnd, isize) -> i32, param: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn GetWindow(hwnd: Hwnd, cmd: u32) -> Hwnd;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn IsWindow(hwnd: Hwnd) -> i32;
    fn GetClassNameW(hwnd: Hwnd, name: *mut u16, len: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
}

const GW_OWNER: u32 = 4;
const SWP_NOSIZE: u32 = 0x1;
const SWP_NOZORDER: u32 = 0x4;
const SWP_NOACTIVATE: u32 = 0x10;
const SM_XVIRTUALSCREEN: i32 = 76;
const SM_CXVIRTUALSCREEN: i32 = 78;
/// How far past the right edge of all screens.
const MARGIN: i32 = 64;

/// The window we moved and where it was: (window, old left, old top).
static MOVED: Mutex<Option<(usize, i32, i32)>> = Mutex::new(None);

/// Source's game window class. Only such a window is ever moved (never e.g. a console window that
/// Windows reports as belonging to this process).
const GAME_CLASS: &str = "Valve001";

fn class_of(hwnd: Hwnd) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), 64) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// This process's largest visible top-level window of Source's class: Garry's Mod's game window.
fn game_window() -> Option<Hwnd> {
    unsafe extern "system" fn each(hwnd: Hwnd, param: isize) -> i32 {
        let best = &mut *(param as *mut (usize, i64));
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() && class_of(hwnd) == GAME_CLASS {
            let mut r = Rect::default();
            GetWindowRect(hwnd, &mut r);
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if area > best.1 {
                *best = (hwnd as usize, area);
            }
        }
        1
    }
    let mut best = (0usize, 0i64);
    unsafe { EnumWindows(each, &mut best as *mut (usize, i64) as isize) };
    (best.0 != 0).then_some(best.0 as Hwnd)
}

fn rect(hwnd: Hwnd) -> Rect {
    let mut r = Rect::default();
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// Moves `hwnd` just past the right edge of all screens (same height on screen). Remembers where
/// it was, unless it was already moved by us. A line for the log.
fn move_off(hwnd: Hwnd) -> String {
    let mut moved = MOVED.lock().unwrap_or_else(|e| e.into_inner());
    let r = rect(hwnd);
    let right_edge = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    if moved.map(|m| m.0) == Some(hwnd as usize) && r.left >= right_edge {
        return format!("Garry's Mod's window already off-screen at ({}, {})", r.left, r.top);
    }
    let ok = unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), right_edge + MARGIN, r.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) } != 0;
    if !ok {
        return "Garry's Mod's window could NOT be moved (SetWindowPos failed)".into();
    }
    *moved = Some((hwnd as usize, r.left, r.top));
    let now = rect(hwnd);
    format!("Garry's Mod's window ({}x{}) moved off-screen from ({}, {}) to ({}, {}); screens end at x = {}",
        r.right - r.left, r.bottom - r.top, r.left, r.top, now.left, now.top, right_edge)
}

/// Puts the window we moved back where it was. A line for the log.
fn restore() -> String {
    let mut moved = MOVED.lock().unwrap_or_else(|e| e.into_inner());
    let Some((hwnd, x, y)) = moved.take() else {
        return "Garry's Mod's window wasn't moved".into();
    };
    let hwnd = hwnd as Hwnd;
    if unsafe { IsWindow(hwnd) } == 0 {
        return "Garry's Mod's window is gone".into();
    }
    let ok = unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) } != 0;
    format!("Garry's Mod's window {} ({}, {})", if ok { "put back at" } else { "could NOT be put back at" }, x, y)
}

/// gmsky.OverlayWindow(off_screen) -> line for the log.
pub fn set_off_screen(off: bool) -> String {
    if !off {
        return restore();
    }
    match game_window() {
        Some(h) => move_off(h),
        None => format!("no Garry's Mod window found (no visible {GAME_CLASS} window in this process)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "user32")]
    extern "system" {
        fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
            parent: Hwnd, menu: Hwnd, inst: Hwnd, param: Hwnd) -> Hwnd;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
    }

    /// On a hidden window of our own: moved past all screens without resizing, put back exactly;
    /// a second move keeps the first old position; restoring twice is harmless.
    #[test]
    fn moves_off_screen_and_back() {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe { CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 100, 120, 320, 200,
            std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        assert!(!hwnd.is_null());
        let before = rect(hwnd);
        let line = move_off(hwnd);
        let after = rect(hwnd);
        let edge = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        println!("{line}");
        assert_eq!(after.left, edge + MARGIN);
        assert_eq!((after.right - after.left, after.bottom - after.top), (before.right - before.left, before.bottom - before.top));
        assert!(move_off(hwnd).contains("already off-screen"));
        println!("{}", restore());
        assert_eq!(rect(hwnd), before);
        assert_eq!(restore(), "Garry's Mod's window wasn't moved");
        unsafe { DestroyWindow(hwnd) };
    }
}
