//! Step 2b-1 (DESIGN.md "Step 2b in detail"): a hook on Garry's Mod's Direct3D 9 `Present`, the
//! call that hands a finished frame to Windows. It runs on the thread that really draws, once per
//! finished frame, whatever `mat_queue_mode` is, so step 2b-3 will copy the picture there.
//!
//! For now it only counts: frames, which thread, and the back buffer (the picture being presented):
//! format, size, multisampling. Installed by replacing the `Present` entry (and `PresentEx` on an
//! Ex device) in the device's function table, in this process's memory; nothing on disk changes.
//! This d3d9.dll gives each device its own copy of the table (found after test 40), so normally
//! only Garry's Mod's device goes through the hook; if a table were shared, the hook counts other
//! devices' presents separately and passes everything on unchanged.
//! The module is never unloaded (the stub never frees it), so the table never points at freed code.

use crate::d3d::{self, func, RELEASE, SURFACE_GET_DESC};
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::Mutex;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}

/// IDirect3DDevice9 function numbers (d3d9.h order).
const PRESENT: usize = 17;
const GET_BACK_BUFFER: usize = 18;
/// IDirect3DDevice9Ex::PresentEx
const PRESENT_EX: usize = 121;

type PresentFn = unsafe extern "system" fn(usize, *const c_void, *const c_void, *mut c_void, *const c_void) -> i32;
type PresentExFn = unsafe extern "system" fn(usize, *const c_void, *const c_void, *mut c_void, *const c_void, u32) -> i32;

/// The functions the hook replaced (0 = not hooked).
static ORIG_PRESENT: AtomicUsize = AtomicUsize::new(0);
static ORIG_PRESENT_EX: AtomicUsize = AtomicUsize::new(0);
/// The function table that was patched, and Garry's Mod's device.
static HOOKED_TABLE: AtomicUsize = AtomicUsize::new(0);
static DEVICE: AtomicUsize = AtomicUsize::new(0);
static INSTALL: Mutex<()> = Mutex::new(());

/// Counters, written by the drawing thread, read (and the "since last" ones reset) from Lua.
static PRESENTS: AtomicU64 = AtomicU64::new(0);
static PRESENTS_EX: AtomicU64 = AtomicU64::new(0);
static OTHER_DEVICE: AtomicU64 = AtomicU64::new(0);
static THREAD: AtomicU32 = AtomicU32::new(0);
static THREAD_CHANGES: AtomicU64 = AtomicU64::new(0);
/// Back buffer: format, width, height, multisample type; and how often reading it failed.
static BB_FORMAT: AtomicU32 = AtomicU32::new(0);
static BB_W: AtomicU32 = AtomicU32::new(0);
static BB_H: AtomicU32 = AtomicU32::new(0);
static BB_MS: AtomicU32 = AtomicU32::new(0);
static BB_CHANGES: AtomicU64 = AtomicU64::new(0);
static BB_FAILS: AtomicU64 = AtomicU64::new(0);
/// Time spent inside our part of the hook (not Present itself), in performance-counter ticks.
static HOOK_TICKS: AtomicU64 = AtomicU64::new(0);
static HOOK_TICKS_MAX: AtomicU64 = AtomicU64::new(0);

/// What status() reported last, to give "since last time" numbers.
static LAST: Mutex<(u64, u64, u64, u64)> = Mutex::new((0, 0, 0, 0));

fn qpc() -> i64 {
    let mut c = 0i64;
    unsafe { QueryPerformanceCounter(&mut c) };
    c
}

fn qpc_ms(ticks: u64) -> f64 {
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f) };
    if f <= 0 { 0.0 } else { ticks as f64 * 1000.0 / f as f64 }
}

/// Our part of every present: count, note the thread and the back buffer.
unsafe fn on_present(dev: usize, ex: bool) {
    let t0 = qpc();
    if dev != DEVICE.load(Relaxed) {
        OTHER_DEVICE.fetch_add(1, Relaxed);
        return;
    }
    if ex { PRESENTS_EX.fetch_add(1, Relaxed) } else { PRESENTS.fetch_add(1, Relaxed) };
    let tid = GetCurrentThreadId();
    if THREAD.swap(tid, Relaxed) != tid {
        THREAD_CHANGES.fetch_add(1, Relaxed);
    }
    back_buffer(dev);
    let dt = (qpc() - t0).max(0) as u64;
    HOOK_TICKS.fetch_add(dt, Relaxed);
    HOOK_TICKS_MAX.fetch_max(dt, Relaxed);
}

/// Reads the back buffer's description (D3DSURFACE_DESC: format, type, usage, pool, multisample
/// type, quality, width, height).
unsafe fn back_buffer(dev: usize) {
    type GetBackBuffer = unsafe extern "system" fn(usize, u32, u32, u32, *mut usize) -> i32;
    type Desc = unsafe extern "system" fn(usize, *mut [u32; 8]) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    let vt = *(dev as *const usize);
    let gbb: GetBackBuffer = std::mem::transmute(func(vt, GET_BACK_BUFFER));
    let mut s = 0usize;
    if gbb(dev, 0, 0, 0, &mut s) != 0 || s == 0 {
        BB_FAILS.fetch_add(1, Relaxed);
        return;
    }
    let svt = *(s as *const usize);
    let desc: Desc = std::mem::transmute(func(svt, SURFACE_GET_DESC));
    let mut d = [0u32; 8];
    let ok = desc(s, &mut d) == 0;
    let rel: Rel = std::mem::transmute(func(svt, RELEASE));
    rel(s);
    if !ok {
        BB_FAILS.fetch_add(1, Relaxed);
        return;
    }
    let changed = [(&BB_FORMAT, d[0]), (&BB_MS, d[4]), (&BB_W, d[6]), (&BB_H, d[7])]
        .iter().fold(false, |any, (a, v)| (a.swap(*v, Relaxed) != *v) | any);
    if changed {
        BB_CHANGES.fetch_add(1, Relaxed);
    }
}

unsafe extern "system" fn present_hook(dev: usize, src: *const c_void, dst: *const c_void, wnd: *mut c_void, dirty: *const c_void) -> i32 {
    let _ = catch_unwind(AssertUnwindSafe(|| on_present(dev, false)));
    let orig: PresentFn = std::mem::transmute(ORIG_PRESENT.load(Acquire));
    orig(dev, src, dst, wnd, dirty)
}

unsafe extern "system" fn present_ex_hook(dev: usize, src: *const c_void, dst: *const c_void, wnd: *mut c_void, dirty: *const c_void, flags: u32) -> i32 {
    let _ = catch_unwind(AssertUnwindSafe(|| on_present(dev, true)));
    let orig: PresentExFn = std::mem::transmute(ORIG_PRESENT_EX.load(Acquire));
    orig(dev, src, dst, wnd, dirty, flags)
}

/// Writes one function-table entry (made writable for the moment of the write). Returns the old one.
unsafe fn patch(table: usize, i: usize, new: usize) -> Result<usize, String> {
    let at = (table + i * 8) as *mut usize;
    let mut old_prot = 0u32;
    if VirtualProtect(at as *mut c_void, 8, 0x04 /* PAGE_READWRITE */, &mut old_prot) == 0 {
        return Err(format!("VirtualProtect failed for entry {i}"));
    }
    // One aligned 8-byte write: a thread calling Present at this moment sees the old or the new one.
    let old = std::ptr::replace(at, new);
    let mut ignored = 0u32;
    VirtualProtect(at as *mut c_void, 8, old_prot, &mut ignored);
    Ok(old)
}

/// Installs the hook on `device` (object, function table, Ex or not). Does nothing if already
/// installed on that table. Returns a line for the log.
unsafe fn install_on(device: usize, table: usize, ex: bool) -> Result<String, String> {
    let _guard = INSTALL.lock().unwrap_or_else(|e| e.into_inner());
    DEVICE.store(device, Release);
    let hooked = HOOKED_TABLE.load(Acquire);
    if hooked == table {
        return Ok("Present hook already installed".into());
    }
    if hooked != 0 {
        // A device with a different function table (shouldn't happen): leave the old one hooked
        // too (it passes everything on) and hook this one.
        uninstall_locked();
    }
    let orig = patch(table, PRESENT, present_hook as PresentFn as usize)?;
    ORIG_PRESENT.store(orig, Release);
    let mut line = format!("Present hook installed (table entry {PRESENT})");
    if ex {
        match patch(table, PRESENT_EX, present_ex_hook as PresentExFn as usize) {
            Ok(o) => {
                ORIG_PRESENT_EX.store(o, Release);
                line += &format!(" and PresentEx (entry {PRESENT_EX})");
            }
            Err(e) => line += &format!("; PresentEx not hooked: {e}"),
        }
    }
    HOOKED_TABLE.store(table, Release);
    Ok(line)
}

/// Puts the original functions back (tests; and before hooking a different table).
unsafe fn uninstall_locked() {
    let table = HOOKED_TABLE.swap(0, AcqRel);
    if table == 0 {
        return;
    }
    // Only if the entry is still ours (something else may have hooked over us since).
    for (i, ours, orig) in [(PRESENT, present_hook as PresentFn as usize, &ORIG_PRESENT), (PRESENT_EX, present_ex_hook as PresentExFn as usize, &ORIG_PRESENT_EX)] {
        let o = orig.load(Acquire);
        if o != 0 && *((table + i * 8) as *const usize) == ours {
            let _ = patch(table, i, o);
        }
    }
}

/// gmsky.OverlayHook(): finds Garry's Mod's device and installs the hook. A line for the log.
pub fn install() -> String {
    let lua_thread = unsafe { GetCurrentThreadId() };
    match d3d::device() {
        Ok((f, note)) => match unsafe { install_on(f.device, f.vtable, f.ex) } {
            Ok(line) => format!("{}{line}; Lua thread {lua_thread}", if note.is_empty() { String::new() } else { format!("{note}; ") }),
            Err(e) => format!("{note}; hook NOT installed: {e}"),
        },
        Err(e) => format!("hook NOT installed, device not found: {e}"),
    }
}

/// gmsky.OverlayStatus(): nil if not hooked, otherwise the numbers since the last call.
pub fn status() -> Option<String> {
    if HOOKED_TABLE.load(Acquire) == 0 {
        return None;
    }
    let (p, pe, other, ticks) = (PRESENTS.load(Relaxed), PRESENTS_EX.load(Relaxed), OTHER_DEVICE.load(Relaxed), HOOK_TICKS.load(Relaxed));
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let (dp, dpe, dother, dticks) = (p - last.0, pe - last.1, other - last.2, ticks - last.3);
    *last = (p, pe, other, ticks);
    let frames = dp + dpe;
    let lua_thread = unsafe { GetCurrentThreadId() };
    let draw_thread = THREAD.load(Relaxed);
    Some(format!(
        "present hook: {dp} Present + {dpe} PresentEx since last ({p} + {pe} in all), {dother} from other devices; \
         drawing thread {draw_thread} ({} Lua's {lua_thread}; {} thread changes); back buffer {} {}x{} multisample {} ({} changes, {} read failures); \
         hook cost {:.3} ms per frame on average, {:.3} ms at most",
        if draw_thread == lua_thread { "SAME as" } else { "not" }, THREAD_CHANGES.load(Relaxed),
        d3d::format_name(BB_FORMAT.load(Relaxed)), BB_W.load(Relaxed), BB_H.load(Relaxed), BB_MS.load(Relaxed),
        BB_CHANGES.load(Relaxed), BB_FAILS.load(Relaxed),
        if frames > 0 { qpc_ms(dticks) / frames as f64 } else { 0.0 }, qpc_ms(HOOK_TICKS_MAX.swap(0, Relaxed))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook on a real hidden 16 x 16 device on this PC, plain and Ex: Present goes through the
    /// hook, is counted, the back buffer is read, and the original runs (it returns success).
    #[test]
    fn hooks_present_on_a_real_device() {
        for ex in [false, true] {
            let line = d3d::tests::with_test_device(ex, |dev, vt| unsafe {
                let before = PRESENTS.load(Relaxed) + PRESENTS_EX.load(Relaxed);
                let line = install_on(dev, vt, ex).unwrap();
                let present: PresentFn = std::mem::transmute(func(vt, PRESENT));
                assert_eq!(present as usize, present_hook as PresentFn as usize);
                let null = std::ptr::null();
                let hr = present(dev, null, null, std::ptr::null_mut(), null);
                if ex {
                    let pex: PresentExFn = std::mem::transmute(func(vt, PRESENT_EX));
                    assert_eq!(pex as usize, present_ex_hook as PresentExFn as usize);
                    assert!(pex(dev, null, null, std::ptr::null_mut(), null, 0) >= 0);
                }
                let counted = PRESENTS.load(Relaxed) + PRESENTS_EX.load(Relaxed) - before;
                let st = status().unwrap();
                uninstall_locked();
                assert_eq!(func(vt, PRESENT), ORIG_PRESENT.load(Relaxed), "original Present back after uninstall");
                // Success, or "occluded" (0x08760878, also a success code): the window is hidden.
                assert!(hr >= 0, "the original Present ran: 0x{:08x}", hr as u32);
                assert_eq!(counted, if ex { 2 } else { 1 });
                assert_eq!((BB_W.load(Relaxed), BB_H.load(Relaxed)), (16, 16));
                format!("{line} | {st}")
            });
            println!("ex={ex}: {line}");
        }
    }
}
