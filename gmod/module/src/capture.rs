//! Step 2b-3 (DESIGN.md "Step 2b in detail"): copies Garry's Mod's finished picture into the
//! overlay picture block, from the Present hook (present.rs), on the thread that draws.
//!
//! Without making the game wait for the graphics card (the idea of SkyCraft's FrameExporter, MIT):
//!   frame N:   the back buffer is copied, on the graphics card, into one of three pictures of our
//!              own (StretchRect), and an "event query" asks the card to say when that is done;
//!   later:     once the card says so, that picture comes to normal memory (GetRenderTargetData
//!              into a system-memory surface), and is copied into Garry's Mod's slot in the block,
//!              which is then published (the newest finished copy only; older ones are dropped).
//! Only frames Lua prepared (gmsky.OverlayPrepared) are copied. Our pictures are released before
//! the device is reset (resolution change) and made again at the new size.

use crate::d3d::{func, RELEASE, SURFACE_GET_DESC};
use gmsky_protocol::overlay::{OverlayMapping, SlotHdr, FMT_BGRA8, F_BLUR, F_PREMULTIPLIED, F_SRGB, MAX_H, MAX_W};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::Mutex;

// IDirect3DDevice9 function numbers (d3d9.h order).
const GET_BACK_BUFFER: usize = 18;
const CREATE_RENDER_TARGET: usize = 28;
const GET_RENDER_TARGET_DATA: usize = 32;
const STRETCH_RECT: usize = 34;
const CREATE_OFFSCREEN_PLAIN_SURFACE: usize = 36;
const CREATE_QUERY: usize = 118;
// IDirect3DSurface9
const LOCK_RECT: usize = 13;
const UNLOCK_RECT: usize = 14;
// IDirect3DQuery9
const QUERY_ISSUE: usize = 6;
const QUERY_GET_DATA: usize = 7;

const FMT_A8R8G8B8: u32 = 21;
const POOL_SYSTEMMEM: u32 = 2;
const QUERY_EVENT: u32 = 8;
const ISSUE_END: u32 = 1;
const LOCK_READONLY: u32 = 0x10;
const S_OK: i32 = 0;

/// Copies in flight at once (SkyCraft: 3).
const SETS: usize = 3;

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Free,
    /// Copied on the graphics card; waiting for the event query.
    Pending,
}

#[derive(Default)]
struct Set {
    /// Our picture on the graphics card, the system-memory surface it comes to, the query.
    rt: usize,
    sys: usize,
    query: usize,
    stage: Stage,
    frame: u64,
    qpc: i64,
}

const NO_SET: Set = Set { rt: 0, sys: 0, query: 0, stage: Stage::Free, frame: 0, qpc: 0 };

struct Capture {
    /// The device the sets were made on.
    dev: usize,
    /// Size and format the sets were made for (0 = not made).
    w: u32,
    h: u32,
    format: u32,
    sets: [Set; SETS],
    block: Option<OverlayMapping>,
    frame: u64,
}

static CAP: Mutex<Capture> = Mutex::new(Capture {
    dev: 0, w: 0, h: 0, format: 0, sets: [NO_SET; SETS], block: None, frame: 0,
});
static ENABLED: AtomicBool = AtomicBool::new(false);
/// gmsky_overlay_blur: Skyrim blurs behind see-through parts (flag in each picture's header).
static BLUR: AtomicBool = AtomicBool::new(true);
/// Set by Lua each frame it prepared; taken by the next Present.
static PREPARED: AtomicBool = AtomicBool::new(false);
/// Block name (tests use their own).
static BLOCK_NAME: Mutex<Option<String>> = Mutex::new(None);

// Counters, read and reset by status().
static PUBLISHED: AtomicU64 = AtomicU64::new(0);
static STARTED: AtomicU64 = AtomicU64::new(0);
static UNPREPARED: AtomicU64 = AtomicU64::new(0);
static BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static TOO_BIG: AtomicU64 = AtomicU64::new(0);
static FAILURES: AtomicU64 = AtomicU64::new(0);
static REMADE: AtomicU64 = AtomicU64::new(0);
static RESETS: AtomicU64 = AtomicU64::new(0);
/// Time on the drawing thread: starting a copy (StretchRect + query), and finishing one
/// (GetRenderTargetData + lock + copy into the block); performance-counter ticks, sum and max.
static START_TICKS: AtomicU64 = AtomicU64::new(0);
static FINISH_TICKS: AtomicU64 = AtomicU64::new(0);
static FINISH_MAX: AtomicU64 = AtomicU64::new(0);
/// Frames between starting a copy and publishing it (sum, for the average).
static LAG_FRAMES: AtomicU64 = AtomicU64::new(0);
/// How much of each published picture is fully opaque, in thousandths, from every 8th pixel in
/// each direction (after test 44: a halo made whole pictures opaque). Sum (for the average) and max.
static OPAQUE_SUM: AtomicU64 = AtomicU64::new(0);
static OPAQUE_MAX: AtomicU64 = AtomicU64::new(0);
/// The last error, for the log.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn fail(msg: String) {
    FAILURES.fetch_add(1, Relaxed);
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = msg;
}

unsafe fn release(obj: &mut usize) {
    if *obj != 0 {
        type Rel = unsafe extern "system" fn(usize) -> u32;
        let rel: Rel = std::mem::transmute(func(*(*obj as *const usize), RELEASE));
        rel(*obj);
        *obj = 0;
    }
}

impl Capture {
    unsafe fn release_all(&mut self) {
        for s in self.sets.iter_mut() {
            release(&mut s.rt);
            release(&mut s.sys);
            release(&mut s.query);
            *s = NO_SET;
        }
        self.w = 0;
        self.h = 0;
    }

    /// Makes the three sets for a w x h picture of `format`.
    unsafe fn make(&mut self, dev: usize, w: u32, h: u32, format: u32) -> Result<(), String> {
        type CreateRt = unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, i32, *mut usize, *mut c_void) -> i32;
        type CreateOff = unsafe extern "system" fn(usize, u32, u32, u32, u32, *mut usize, *mut c_void) -> i32;
        type CreateQuery = unsafe extern "system" fn(usize, u32, *mut usize) -> i32;
        self.release_all();
        let vt = *(dev as *const usize);
        let create_rt: CreateRt = std::mem::transmute(func(vt, CREATE_RENDER_TARGET));
        let create_off: CreateOff = std::mem::transmute(func(vt, CREATE_OFFSCREEN_PLAIN_SURFACE));
        let create_query: CreateQuery = std::mem::transmute(func(vt, CREATE_QUERY));
        for (i, s) in self.sets.iter_mut().enumerate() {
            let hr = create_rt(dev, w, h, format, 0, 0, 0, &mut s.rt, std::ptr::null_mut());
            if hr != S_OK {
                return Err(format!("CreateRenderTarget {w}x{h} format {format} (set {i}): 0x{:08x}", hr as u32));
            }
            let hr = create_off(dev, w, h, format, POOL_SYSTEMMEM, &mut s.sys, std::ptr::null_mut());
            if hr != S_OK {
                return Err(format!("CreateOffscreenPlainSurface (set {i}): 0x{:08x}", hr as u32));
            }
            let hr = create_query(dev, QUERY_EVENT, &mut s.query);
            if hr != S_OK {
                return Err(format!("CreateQuery event (set {i}): 0x{:08x}", hr as u32));
            }
        }
        self.w = w;
        self.h = h;
        self.format = format;
        REMADE.fetch_add(1, Relaxed);
        Ok(())
    }

    /// Has the graphics card finished the copy of this set? (Never waits.)
    unsafe fn done(s: &Set) -> bool {
        type GetData = unsafe extern "system" fn(usize, *mut c_void, u32, u32) -> i32;
        let get: GetData = std::mem::transmute(func(*(s.query as *const usize), QUERY_GET_DATA));
        let mut v = 0u32;
        get(s.query, &mut v as *mut u32 as *mut c_void, 4, 0) == S_OK
    }

    /// Brings a finished set to normal memory and publishes it in the block.
    unsafe fn finish(&mut self, dev: usize, i: usize) -> Result<(), String> {
        type Grtd = unsafe extern "system" fn(usize, usize, usize) -> i32;
        #[repr(C)]
        struct Locked { pitch: i32, bits: *mut u8 }
        type Lock = unsafe extern "system" fn(usize, *mut Locked, *const c_void, u32) -> i32;
        type Unlock = unsafe extern "system" fn(usize) -> i32;
        let (w, h) = (self.w, self.h);
        let s = &self.sets[i];
        let vt = *(dev as *const usize);
        let grtd: Grtd = std::mem::transmute(func(vt, GET_RENDER_TARGET_DATA));
        let hr = grtd(dev, s.rt, s.sys);
        if hr != S_OK {
            return Err(format!("GetRenderTargetData: 0x{:08x}", hr as u32));
        }
        let svt = *(s.sys as *const usize);
        let lock: Lock = std::mem::transmute(func(svt, LOCK_RECT));
        let unlock: Unlock = std::mem::transmute(func(svt, UNLOCK_RECT));
        let mut l = Locked { pitch: 0, bits: std::ptr::null_mut() };
        let hr = lock(s.sys, &mut l, std::ptr::null(), LOCK_READONLY);
        if hr != S_OK || l.bits.is_null() {
            return Err(format!("LockRect: 0x{:08x}", hr as u32));
        }
        let Some(block) = self.block.as_ref() else {
            unlock(s.sys);
            return Err("no overlay block".into());
        };
        let row = (w * 4) as usize;
        let dst = block.slot_ptr(block.own_slot());
        if l.pitch as usize == row {
            std::ptr::copy_nonoverlapping(l.bits, dst, row * h as usize);
        } else {
            for y in 0..h as usize {
                std::ptr::copy_nonoverlapping(l.bits.add(y * l.pitch as usize), dst.add(y * row), row);
            }
        }
        let opaque = opaque_permille(l.bits, l.pitch as usize, w, h);
        OPAQUE_SUM.fetch_add(opaque, Relaxed);
        OPAQUE_MAX.fetch_max(opaque, Relaxed);
        unlock(s.sys);
        block.publish(&SlotHdr {
            width: w, height: h, format: FMT_BGRA8, flags: F_PREMULTIPLIED | F_SRGB | if BLUR.load(Relaxed) { F_BLUR } else { 0 },
            frame: s.frame, capture_qpc: s.qpc, row_bytes: row as u32, ..Default::default()
        });
        Ok(())
    }

    /// Copies the back buffer into a free set on the graphics card and asks to be told when done.
    unsafe fn start(&mut self, dev: usize, bb: usize, i: usize) -> Result<(), String> {
        type Stretch = unsafe extern "system" fn(usize, usize, *const c_void, usize, *const c_void, u32) -> i32;
        type Issue = unsafe extern "system" fn(usize, u32) -> i32;
        let vt = *(dev as *const usize);
        let stretch: Stretch = std::mem::transmute(func(vt, STRETCH_RECT));
        let s = &mut self.sets[i];
        let hr = stretch(dev, bb, std::ptr::null(), s.rt, std::ptr::null(), 0 /* D3DTEXF_NONE */);
        if hr != S_OK {
            return Err(format!("StretchRect: 0x{:08x}", hr as u32));
        }
        let issue: Issue = std::mem::transmute(func(*(s.query as *const usize), QUERY_ISSUE));
        issue(s.query, ISSUE_END);
        s.stage = Stage::Pending;
        s.frame = self.frame;
        s.qpc = gmsky_protocol::shm::now_qpc();
        Ok(())
    }
}

/// Thousandths of the sampled pixels (every 8th in each direction) with alpha 255. BGRA bytes.
unsafe fn opaque_permille(bits: *const u8, pitch: usize, w: u32, h: u32) -> u64 {
    let (mut all, mut opaque) = (0u64, 0u64);
    for y in (0..h as usize).step_by(8) {
        let row = bits.add(y * pitch);
        for x in (0..w as usize).step_by(8) {
            all += 1;
            opaque += (*row.add(x * 4 + 3) == 255) as u64;
        }
    }
    if all == 0 { 0 } else { opaque * 1000 / all }
}

fn qpc() -> u64 {
    gmsky_protocol::shm::now_qpc().max(0) as u64
}

/// Called from the Present hook (drawing thread) for Garry's Mod's device, before Present itself.
pub unsafe fn on_present(dev: usize) {
    let mut cap = CAP.lock().unwrap_or_else(|e| e.into_inner());
    cap.frame += 1;
    if cap.dev != dev {
        // A different device (the game made a new one): the old sets belong to the old device,
        // which may be gone, so they are forgotten, not released.
        for s in cap.sets.iter_mut() {
            *s = NO_SET;
        }
        cap.w = 0;
        cap.h = 0;
        cap.dev = dev;
    }
    if !ENABLED.load(Acquire) {
        if cap.w != 0 {
            cap.release_all();
        }
        PREPARED.store(false, Relaxed);
        return;
    }
    if cap.block.is_none() {
        let name = BLOCK_NAME.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match name.as_deref().map(OverlayMapping::open_named).unwrap_or_else(OverlayMapping::open) {
            Ok(mut b) => {
                b.attach(true);
                cap.block = Some(b);
            }
            Err(e) => {
                fail(format!("overlay block: {e}"));
                ENABLED.store(false, Release);
                return;
            }
        }
    }

    // 1. The newest set the graphics card has finished goes to the block; older finished ones
    //    are dropped (a newer picture is already there).
    let t0 = qpc();
    let finished: Vec<usize> = (0..SETS)
        .filter(|&i| cap.sets[i].stage == Stage::Pending && Capture::done(&cap.sets[i]))
        .collect();
    let newest = finished.iter().copied().max_by_key(|&i| cap.sets[i].frame);
    for &i in &finished {
        if Some(i) != newest {
            cap.sets[i].stage = Stage::Free;
            DROPPED.fetch_add(1, Relaxed);
        }
    }
    if let Some(i) = newest {
        match cap.finish(dev, i) {
            Ok(()) => {
                PUBLISHED.fetch_add(1, Relaxed);
                LAG_FRAMES.fetch_add(cap.frame - cap.sets[i].frame, Relaxed);
            }
            Err(e) => fail(e),
        }
        cap.sets[i].stage = Stage::Free;
        let dt = qpc() - t0;
        FINISH_TICKS.fetch_add(dt, Relaxed);
        FINISH_MAX.fetch_max(dt, Relaxed);
    }

    // 2. This frame, if Lua prepared it.
    if !PREPARED.swap(false, AcqRel) {
        UNPREPARED.fetch_add(1, Relaxed);
        return;
    }
    let t1 = qpc();
    type GetBackBuffer = unsafe extern "system" fn(usize, u32, u32, u32, *mut usize) -> i32;
    type Desc = unsafe extern "system" fn(usize, *mut [u32; 8]) -> i32;
    let vt = *(dev as *const usize);
    let gbb: GetBackBuffer = std::mem::transmute(func(vt, GET_BACK_BUFFER));
    let mut bb = 0usize;
    if gbb(dev, 0, 0, 0, &mut bb) != S_OK || bb == 0 {
        fail("GetBackBuffer failed".into());
        return;
    }
    let desc: Desc = std::mem::transmute(func(*(bb as *const usize), SURFACE_GET_DESC));
    let mut d = [0u32; 8];
    let result = if desc(bb, &mut d) != S_OK {
        Err("back buffer GetDesc failed".to_string())
    } else if d[0] != FMT_A8R8G8B8 {
        Err(format!("back buffer format {} is not A8R8G8B8: no alpha to send", d[0]))
    } else if d[6] > MAX_W || d[7] > MAX_H {
        TOO_BIG.fetch_add(1, Relaxed);
        Ok(())
    } else {
        let (w, h) = (d[6], d[7]);
        let made = if (cap.w, cap.h, cap.format) != (w, h, d[0]) { cap.make(dev, w, h, d[0]) } else { Ok(()) };
        match made {
            Err(e) => {
                cap.release_all();
                Err(e)
            }
            Ok(()) => match cap.sets.iter().position(|s| s.stage == Stage::Free) {
                None => {
                    BUSY.fetch_add(1, Relaxed);
                    Ok(())
                }
                Some(i) => cap.start(dev, bb, i).map(|()| { STARTED.fetch_add(1, Relaxed); }),
            },
        }
    };
    release(&mut bb);
    if let Err(e) = result {
        fail(e);
    }
    START_TICKS.fetch_add(qpc() - t1, Relaxed);
}

/// Called from the Reset / ResetEx hooks, before the device is reset: Direct3D 9 requires our
/// pictures on the graphics card to be released first. They are made again on the next frame.
pub unsafe fn on_reset() {
    RESETS.fetch_add(1, Relaxed);
    CAP.lock().unwrap_or_else(|e| e.into_inner()).release_all();
}

/// gmsky.OverlayEnable(on): start / stop copying (the block stays open once opened).
pub fn enable(on: bool) {
    ENABLED.store(on, Release);
}

/// gmsky.OverlayPrepared(): Lua prepared this frame (cleared behind the view model, alpha forced).
pub fn mark_prepared() {
    PREPARED.store(true, Release);
}

/// gmsky.OverlayBlur(on): the blur flag for the next pictures.
pub fn set_blur(on: bool) {
    BLUR.store(on, Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Acquire)
}

/// gmsky.OverlayShot(): asks Skyrim to save its next drawn frame (before and after our picture is
/// drawn, and the picture; step 2c-1). A line for the log.
pub fn request_shot() -> String {
    let cap = CAP.lock().unwrap_or_else(|e| e.into_inner());
    match cap.block.as_ref() {
        Some(b) => {
            let n = b.ctl().shot_request.fetch_add(1, AcqRel) + 1;
            format!("Skyrim asked for snapshot {n}: files logs/overlay/skyrim_shot_{n}_*.bin once Skyrim draws a frame")
        }
        None => "no overlay block open: switch gmsky_overlay 1 on first".into(),
    }
}

/// Step 2b-4: Skyrim's back buffer size (written by Skyrim every frame) against ours.
fn size_line(b: &OverlayMapping, w: u32, h: u32) -> String {
    let (sw, sh) = b.skyrim_size();
    if sw == 0 {
        return "Skyrim's size not known yet".into();
    }
    let at = b.ctl().skyrim_size_qpc.load(Acquire) as i64;
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f) };
    let age_s = (gmsky_protocol::shm::now_qpc() - at) as f64 / f.max(1) as f64;
    let fresh = if age_s > 2.0 { format!(" (written {age_s:.0} s ago: Skyrim not running?)") } else { String::new() };
    if w == 0 {
        format!("Skyrim's size {sw}x{sh}{fresh}")
    } else if (sw, sh) == (w, h) {
        format!("Skyrim's size {sw}x{sh} = ours{fresh}")
    } else {
        format!("Skyrim's size {sw}x{sh} DIFFERS from ours {w}x{h} (set Garry's Mod's resolution to {sw}x{sh}){fresh}")
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}

/// Numbers since the last call, one line.
pub fn status(ms: impl Fn(u64) -> f64) -> String {
    let take = |a: &AtomicU64| a.swap(0, Relaxed);
    let (published, started) = (take(&PUBLISHED), take(&STARTED));
    let (unprepared, busy, dropped, too_big) = (take(&UNPREPARED), take(&BUSY), take(&DROPPED), take(&TOO_BIG));
    let (failures, remade, resets) = (take(&FAILURES), take(&REMADE), take(&RESETS));
    let (start_t, finish_t, finish_max, lag) = (take(&START_TICKS), take(&FINISH_TICKS), take(&FINISH_MAX), take(&LAG_FRAMES));
    let (opaque_sum, opaque_max) = (take(&OPAQUE_SUM), take(&OPAQUE_MAX));
    let cap = CAP.lock().unwrap_or_else(|e| e.into_inner());
    let block = cap.block.as_ref().map(|b| {
        let c = b.ctl();
        format!("block: {} published in all, {} taken by Skyrim, slots {:#x}; {}",
            c.frames_published.load(Relaxed), c.frames_taken.load(Relaxed), b.state(), size_line(b, cap.w, cap.h))
    }).unwrap_or_else(|| "block not open".into());
    let err = std::mem::take(&mut *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()));
    format!(
        "overlay copy {}: {published} published, {started} started, {unprepared} frames not prepared by Lua, {busy} skipped (all copies busy), \
         {dropped} dropped (newer one ready), {too_big} too big, {failures} failures{}; {}x{}, made {remade}x, {resets} device resets; \
         start {:.3} ms/frame, finish {:.3} ms/frame (max {:.3}), {:.2} frames from copy to publish; opaque {:.1}% avg, {:.1}% max; {block}",
        if enabled() { "ON" } else { "off" },
        if err.is_empty() { String::new() } else { format!(" (last: {err})") },
        cap.w, cap.h,
        if started > 0 { ms(start_t) / started as f64 } else { 0.0 },
        if published > 0 { ms(finish_t) / published as f64 } else { 0.0 }, ms(finish_max),
        if published > 0 { lag as f64 / published as f64 } else { 0.0 },
        if published > 0 { opaque_sum as f64 / published as f64 / 10.0 } else { 0.0 }, opaque_max as f64 / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d3d::tests::{with_test_device_sized, GLOBAL_STATE};

    /// After test 44: the opaque share is counted from every 8th pixel in each direction.
    #[test]
    fn counts_opaque_pixels() {
        let (w, h) = (64u32, 16u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            for x in 0..(w / 2) as usize {
                px[(y * w as usize + x) * 4 + 3] = 255; // left half opaque
            }
        }
        assert_eq!(unsafe { opaque_permille(px.as_ptr(), (w * 4) as usize, w, h) }, 500);
        assert_eq!(unsafe { opaque_permille(px.as_ptr(), (w * 4) as usize, 0, 0) }, 0);
    }

    /// Step 2b-4: Skyrim's size (as Skyrim writes it) against ours.
    #[test]
    fn compares_skyrim_size() {
        let name = format!(r"Local\GmodSkyrim_overlay_test_{}_size", std::process::id());
        let b = OverlayMapping::open_named(&name).unwrap();
        assert_eq!(size_line(&b, 1280, 720), "Skyrim's size not known yet");
        b.set_skyrim_size(1920, 1080);
        assert_eq!(size_line(&b, 1920, 1080), "Skyrim's size 1920x1080 = ours");
        assert_eq!(size_line(&b, 1280, 720),
            "Skyrim's size 1920x1080 DIFFERS from ours 1280x720 (set Garry's Mod's resolution to 1920x1080)");
        // Skyrim stopped writing 5 s ago.
        let mut f = 0i64;
        unsafe { QueryPerformanceFrequency(&mut f) };
        b.ctl().skyrim_size_qpc.store((gmsky_protocol::shm::now_qpc() - 5 * f) as u64, Release);
        assert!(size_line(&b, 1920, 1080).ends_with("(written 5 s ago: Skyrim not running?)"), "{}", size_line(&b, 1920, 1080));
    }

    /// On a real hidden device with an A8R8G8B8 back buffer (like Garry's Mod's), plain and Ex:
    /// the back buffer, filled with a known half-transparent colour, arrives whole in the block
    /// (read as Skyrim would), a frame Lua didn't prepare is not copied, and a reset releases
    /// our pictures; they are made again afterwards.
    #[test]
    fn back_buffer_arrives_in_the_block() {
        let _one_at_a_time = GLOBAL_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let name = format!(r"Local\GmodSkyrim_overlay_test_{}_capture", std::process::id());
        *BLOCK_NAME.lock().unwrap() = Some(name.clone());
        let mut reader = OverlayMapping::open_named(&name).unwrap();
        reader.attach(false);
        let ms = |t: u64| t as f64 / 10_000.0; // QPC is 10 MHz on Windows 10+ (only for the printout)
        for (ex, colour) in [(false, 0x80FF_2010u32), (true, 0x40_10_20_30u32)] {
            with_test_device_sized(ex, 64, 48, FMT_A8R8G8B8, |dev, vt| unsafe {
                type Clear = unsafe extern "system" fn(usize, u32, *const c_void, u32, u32, f32, u32) -> i32;
                type Present = unsafe extern "system" fn(usize, *const c_void, *const c_void, *mut c_void, *const c_void) -> i32;
                let clear: Clear = std::mem::transmute(func(vt, 43));
                let present: Present = std::mem::transmute(func(vt, 17));
                let null = std::ptr::null();
                assert_eq!(clear(dev, 0, null, 1 /* target */, colour, 1.0, 0), 0);
                enable(true);
                status(ms); // reset the counters
                // Not prepared: nothing starts.
                on_present(dev);
                present(dev, null, null, std::ptr::null_mut(), null);
                let mut got = None;
                for _ in 0..500 {
                    mark_prepared();
                    on_present(dev);
                    present(dev, null, null, std::ptr::null_mut(), null);
                    if let Some(slot) = reader.take() {
                        got = Some(slot);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let line = status(ms);
                println!("ex={ex}: {line}");
                let slot = got.expect("a picture arrived");
                let h = reader.slot_hdr(slot);
                assert_eq!((h.width, h.height, h.row_bytes, h.format), (64, 48, 256, FMT_BGRA8));
                assert_eq!(h.flags, F_PREMULTIPLIED | F_SRGB | F_BLUR, "blur on by default (gmsky_overlay_blur 1)");
                assert!(h.frame > 0 && h.capture_qpc > 0);
                let px = std::slice::from_raw_parts(reader.slot_ptr(slot) as *const u32, 64 * 48);
                // A8R8G8B8 in memory is B, G, R, A: as a little-endian u32, exactly the ARGB colour.
                if let Some(bad) = px.iter().position(|&p| p != colour) {
                    panic!("pixel {bad} is {:#010x}, not {colour:#010x}", px[bad]);
                }
                assert!(line.contains("1 frames not prepared by Lua"), "{line}");
                assert!(line.contains(" 0 failures"), "{line}");
                assert!(line.contains("opaque 0.0% avg, 0.0% max"), "half-transparent colour: nothing opaque: {line}");
                // A reset releases our pictures; the next prepared frame makes them again.
                on_reset();
                assert_eq!(CAP.lock().unwrap().w, 0);
                mark_prepared();
                on_present(dev);
                assert_eq!(CAP.lock().unwrap().w, 64);
                on_reset(); // leave nothing behind for the device's release
                enable(false);
            });
        }
        *BLOCK_NAME.lock().unwrap() = None;
        CAP.lock().unwrap().block = None;
    }
}
