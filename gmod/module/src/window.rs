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
    fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    fn GetForegroundWindow() -> Hwnd;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetCurrentProcess() -> isize;
    fn TerminateProcess(process: isize, code: u32) -> i32;
}

const SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_OBJECT_0: u32 = 0;
const WM_CLOSE: u32 = 0x0010;

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

/// The largest visible top-level window of process `pid` (Skyrim's game window).
fn main_window_of(pid: u32) -> Option<Hwnd> {
    unsafe extern "system" fn each(hwnd: Hwnd, param: isize) -> i32 {
        let best = &mut *(param as *mut (u32, usize, i64));
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == best.0 && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            let mut r = Rect::default();
            GetWindowRect(hwnd, &mut r);
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if area > best.2 {
                *best = (best.0, hwnd as usize, area);
            }
        }
        1
    }
    if pid == 0 {
        return None;
    }
    let mut best = (pid, 0usize, 0i64);
    unsafe { EnumWindows(each, &mut best as *mut (u32, usize, i64) as isize) };
    (best.1 != 0).then_some(best.1 as Hwnd)
}

/// After test 62: Garry's Mod's console in its own window. `front`: the game window is put in the
/// middle of Skyrim's window (process `skyrim_pid`) and brought to the front, so Garry's Mod's own
/// console gets the keyboard; otherwise it goes back past the screens and Skyrim's window is
/// brought to the front. Where it was before step 2d-2 moved it (for the overlay switching off)
/// is kept. A line for the log.
pub fn console_front(front: bool, skyrim_pid: u32) -> String {
    let Some(h) = game_window() else {
        return format!("no Garry's Mod window found (no visible {GAME_CLASS} window in this process)");
    };
    let r = rect(h);
    let (w, ht) = (r.right - r.left, r.bottom - r.top);
    let sky = main_window_of(skyrim_pid);
    if front {
        let (x, y) = match sky {
            Some(s) => {
                let sr = rect(s);
                ((sr.left + sr.right - w) / 2, (sr.top + sr.bottom - ht) / 2)
            }
            None => (0, 0),
        };
        // HWND_TOP = null; activates it (no SWP_NOACTIVATE).
        let moved = unsafe { SetWindowPos(h, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE) } != 0;
        let fore = unsafe { SetForegroundWindow(h) } != 0;
        let now_front = unsafe { GetForegroundWindow() } == h;
        format!("Garry's Mod's window ({w}x{ht}) {} at ({x}, {y}) over Skyrim's window ({}), brought to the front: {} (in front now: {})",
            if moved { "moved" } else { "could NOT be moved" }, if sky.is_some() { "found" } else { "NOT found" }, fore, now_front)
    } else {
        let right_edge = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let moved = unsafe { SetWindowPos(h, std::ptr::null_mut(), right_edge + MARGIN, r.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) } != 0;
        let fore = sky.map_or(false, |s| unsafe { SetForegroundWindow(s) } != 0);
        format!("Garry's Mod's window {} off-screen (x = {}); Skyrim's window {} to the front: {}",
            if moved { "moved" } else { "could NOT be moved" }, right_edge + MARGIN, if sky.is_some() { "brought" } else { "NOT found, not brought" }, fore)
    }
}

/// After test 63 (`showconsole` from Lua is refused): presses Garry's Mod's console key (~, scan code
/// 0x29, `toggleconsole` by default) by posting the key's down and up messages to its window only.
/// A line for the log.
pub fn press_console_key() -> String {
    const WM_KEYDOWN: u32 = 0x100;
    const WM_KEYUP: u32 = 0x101;
    const VK_OEM_3: usize = 0xC0;
    let Some(h) = game_window() else {
        return format!("no Garry's Mod window found (no visible {GAME_CLASS} window in this process)");
    };
    let lp = 1isize | (0x29isize << 16);
    let down = unsafe { PostMessageW(h, WM_KEYDOWN, VK_OEM_3, lp) } != 0;
    let up = unsafe { PostMessageW(h, WM_KEYUP, VK_OEM_3, lp | (1 << 30) | (1 << 31)) } != 0;
    format!("console key (~) posted to Garry's Mod's window: down {down}, up {up}")
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

/// The Skyrim process being watched: (process id, handle that can wait on it).
static WATCHED: Mutex<Option<(u32, usize)>> = Mutex::new(None);
/// The last Skyrim process reported as ended: never watched again (test 127: the shared memory
/// keeps its id after it ends, so it was opened and reported again every second).
static REPORTED: Mutex<u32> = Mutex::new(0);

/// Closing Garry's Mod with Skyrim (the user, after test 126). `pid_now` is Skyrim's process id
/// from the shared memory (0 when it isn't connected). The first time a Skyrim process is seen
/// it is watched; once that process has ended, its id is returned (once). A newer Skyrim process
/// replaces the watched one. Only ever looks at the process; never touches it.
pub fn skyrim_exited(pid_now: u32) -> Option<u32> {
    let mut watched = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
    let mut reported = REPORTED.lock().unwrap_or_else(|e| e.into_inner());
    if pid_now != 0 && pid_now != *reported && watched.map(|w| w.0) != Some(pid_now) {
        if let Some((_, h)) = watched.take() {
            unsafe { CloseHandle(h as *mut c_void) };
        }
        let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid_now) };
        if !h.is_null() {
            *watched = Some((pid_now, h as usize));
        }
    }
    let (pid, h) = (*watched)?;
    if unsafe { WaitForSingleObject(h as *mut c_void, 0) } != WAIT_OBJECT_0 {
        return None;
    }
    unsafe { CloseHandle(h as *mut c_void) };
    *watched = None;
    *reported = pid;
    Some(pid)
}

/// Asks Garry's Mod's game window to close, as its close button does. A line for the log.
pub fn close_game() -> String {
    let Some(h) = game_window() else {
        return format!("no Garry's Mod window found (no visible {GAME_CLASS} window in this process): not closed");
    };
    let ok = unsafe { PostMessageW(h, WM_CLOSE, 0, 0) } != 0;
    format!("Garry's Mod's window {} to close", if ok { "asked" } else { "could NOT be asked" })
}

/// Ends this process (Garry's Mod) at once, without Garry's Mod's own shutdown. Only used when
/// the close message didn't work (test 127: Garry's Mod ignored it).
pub fn end_process() {
    unsafe { TerminateProcess(GetCurrentProcess(), 0) };
}

#[cfg(test)]
mod watch_tests {
    #[test]
    fn skyrim_exited_reports_an_ended_process_once() {
        let mut child = std::process::Command::new("cmd").args(["/c", "ping -n 2 127.0.0.1 >nul"]).spawn().unwrap();
        let pid = child.id();
        assert_eq!(super::skyrim_exited(pid), None, "still running");
        child.wait().unwrap();
        assert_eq!(super::skyrim_exited(0), Some(pid), "ended, pid gone from the header");
        assert_eq!(super::skyrim_exited(0), None, "reported once");
        assert_eq!(super::skyrim_exited(pid), None, "its id still in the header: still reported only once");
    }
}
