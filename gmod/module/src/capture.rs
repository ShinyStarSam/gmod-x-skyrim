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
//!
//! Step W2 (DESIGN.md "Garry's Mod's world objects and effects in Skyrim"): during the frame, Lua
//! asks for two more copies into the same set: the world layer (gmsky.OverlayGrabWorld, whatever
//! picture is being drawn into once the world objects are done) and the depth picture
//! (gmsky.OverlayGrabDepth, Lua's half-size RGBA16F target drawn with Source's DepthWrite shader).
//! All three come to normal memory together; the depth becomes distances (f32, Garry's Mod units).
//! Test 72: copying at the moment Lua asks found the depth picture empty, because Source still held
//! some of its drawing back (it sends batches to the graphics card later). So Lua's calls only
//! remember which picture to copy; the copies are made at the Present, when Source has finished.

use crate::d3d::{func, RELEASE, SURFACE_GET_DESC};
use gmsky_protocol::overlay::{OverlayMapping, SlotHdr, FMT_BGRA8, F_BLUR, F_DEPTH, F_PREMULTIPLIED, F_SRGB, F_WORLD, MASK_OFF, MAX_H, MAX_W};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::Mutex;

// IDirect3DDevice9 function numbers (d3d9.h order).
const GET_BACK_BUFFER: usize = 18;
const CREATE_RENDER_TARGET: usize = 28;
const GET_RENDER_TARGET_DATA: usize = 32;
const STRETCH_RECT: usize = 34;
const CREATE_OFFSCREEN_PLAIN_SURFACE: usize = 36;
const GET_RENDER_TARGET: usize = 38;
const CREATE_QUERY: usize = 118;
// IDirect3DSurface9
const LOCK_RECT: usize = 13;
const UNLOCK_RECT: usize = 14;
// IDirect3DQuery9
const QUERY_ISSUE: usize = 6;
const QUERY_GET_DATA: usize = 7;

const FMT_A8R8G8B8: u32 = 21;
/// D3DFMT_A16B16G16R16F: four half floats per pixel, red first in memory.
const FMT_A16B16G16R16F: u32 = 113;
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
    /// Step W2: the world layer (same size and format as `rt`) and the depth picture (RGBA16F, its
    /// own size), made the first time Lua asks; whether this frame's copies were made.
    world_rt: usize,
    world_sys: usize,
    depth_rt: usize,
    depth_sys: usize,
    depth_w: u32,
    depth_h: u32,
    has_world: bool,
    has_depth: bool,
    /// After test 73: the distances of Lua's two calibration squares (Garry's Mod units).
    depth_refs: [f32; 2],
    /// Step W4: the camera the world layer was drawn with (epoch 0 = none this frame).
    view: View,
}

/// Step W4: a picture's camera (gmsky_protocol.h GmskyOverlaySlotHdr view_*).
#[derive(Clone, Copy, Default)]
pub struct View {
    pub eye: [f32; 3],
    pub angles: [f32; 3],
    pub fov: f32,
    pub aspect: f32,
    pub epoch: u32,
}

const NO_SET: Set = Set {
    rt: 0, sys: 0, query: 0, stage: Stage::Free, frame: 0, qpc: 0, world_rt: 0, world_sys: 0, depth_rt: 0, depth_sys: 0,
    depth_w: 0, depth_h: 0, has_world: false, has_depth: false, depth_refs: [0.0; 2],
    view: View { eye: [0.0; 3], angles: [0.0; 3], fov: 0.0, aspect: 0.0, epoch: 0 },
};

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
    /// Step W2 / test 72: this frame's pictures to copy at the Present (surfaces Lua drew into,
    /// referenced until then), the depth's reference distance, and the camera.
    pending_world: usize,
    pending_depth: usize,
    pending_refs: [f32; 2],
    pending_view: View,
}

static CAP: Mutex<Capture> = Mutex::new(Capture {
    dev: 0, w: 0, h: 0, format: 0, sets: [NO_SET; SETS], block: None, frame: 0, pending_world: 0, pending_depth: 0, pending_refs: [0.0; 2],
    pending_view: View { eye: [0.0; 3], angles: [0.0; 3], fov: 0.0, aspect: 0.0, epoch: 0 },
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
// Step W2 counters (status line "world layer: ...").
static WORLD_GRABS: AtomicU64 = AtomicU64::new(0);
static DEPTH_GRABS: AtomicU64 = AtomicU64::new(0);
/// Grabs refused: copies not made yet (first frames), all sets busy, or the target didn't fit.
static GRABS_REFUSED: AtomicU64 = AtomicU64::new(0);
static WORLD_SENT: AtomicU64 = AtomicU64::new(0);
static DEPTH_SENT: AtomicU64 = AtomicU64::new(0);
/// Pictures whose layers didn't fit the slot (sent as the HUD picture only).
static LAYERS_TOO_BIG: AtomicU64 = AtomicU64::new(0);
/// Depth pixels in all published depth pictures: with a depth, drawn without the override
/// ("foreign": not red-only with alpha 1), and all.
static DEPTH_PX_VALID: AtomicU64 = AtomicU64::new(0);
static DEPTH_PX_FOREIGN: AtomicU64 = AtomicU64::new(0);
static DEPTH_PX_ALL: AtomicU64 = AtomicU64::new(0);
/// Agreement: sampled solid world-layer pixels (alpha 255), and those with a depth next to them.
static AGREE_SOLID: AtomicU64 = AtomicU64::new(0);
static AGREE_DEPTH: AtomicU64 = AtomicU64::new(0);
/// The nearest depth around the screen centre in the newest published depth picture (f64 bits;
/// 0 = none), for Lua's calibration line.
static CENTRE_DEPTH: AtomicU64 = AtomicU64::new(0);
/// Step W4: time spent bringing the world layer and the depth to normal memory (ticks).
static WORLD_TICKS: AtomicU64 = AtomicU64::new(0);
static DEPTH_TICKS: AtomicU64 = AtomicU64::new(0);

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
            release(&mut s.world_rt);
            release(&mut s.world_sys);
            release(&mut s.depth_rt);
            release(&mut s.depth_sys);
            *s = NO_SET;
        }
        self.w = 0;
        self.h = 0;
        self.drop_pending();
    }

    /// Test 72: forgets this frame's pictures to copy (and lets go of them).
    unsafe fn drop_pending(&mut self) {
        release(&mut self.pending_world);
        release(&mut self.pending_depth);
        self.pending_view = View::default();
    }

    /// Step W2 / test 72: remembers the picture Source is drawing into right now (render target 0)
    /// as this frame's world layer (`depth_ref` None) or depth picture; copied at the Present.
    unsafe fn grab(&mut self, depth_ref: Option<[f32; 2]>) -> Result<(), String> {
        type GetRt = unsafe extern "system" fn(usize, u32, *mut usize) -> i32;
        type Desc = unsafe extern "system" fn(usize, *mut [u32; 8]) -> i32;
        let dev = self.dev;
        if !ENABLED.load(Acquire) || dev == 0 || self.w == 0 {
            GRABS_REFUSED.fetch_add(1, Relaxed);
            return Ok(()); // the copies are made on the first prepared Present
        }
        let vt = *(dev as *const usize);
        let get_rt: GetRt = std::mem::transmute(func(vt, GET_RENDER_TARGET));
        let mut surf = 0usize;
        let hr = get_rt(dev, 0, &mut surf); // adds a reference: ours until the Present
        if hr != S_OK || surf == 0 {
            return Err(format!("GetRenderTarget: 0x{:08x}", hr as u32));
        }
        let desc: Desc = std::mem::transmute(func(*(surf as *const usize), SURFACE_GET_DESC));
        let mut d = [0u32; 8];
        if desc(surf, &mut d) != S_OK {
            release(&mut surf);
            return Err("render target GetDesc failed".to_string());
        }
        let (fmt, w, h) = (d[0], d[6], d[7]);
        match depth_ref {
            None => {
                if (w, h) != (self.w, self.h) {
                    release(&mut surf);
                    GRABS_REFUSED.fetch_add(1, Relaxed);
                    return Err(format!("world layer: render target {w}x{h}, not the picture's {}x{}", self.w, self.h));
                }
                release(&mut self.pending_world);
                self.pending_world = surf;
                WORLD_GRABS.fetch_add(1, Relaxed);
            }
            Some(r) => {
                if fmt != FMT_A16B16G16R16F || w == 0 || h == 0 || w > 0xFFFF || h > 0xFFFF {
                    release(&mut surf);
                    GRABS_REFUSED.fetch_add(1, Relaxed);
                    return Err(format!("depth: render target {w}x{h} format {fmt}, not RGBA16F ({FMT_A16B16G16R16F})"));
                }
                release(&mut self.pending_depth);
                self.pending_depth = surf;
                self.pending_refs = r;
                DEPTH_GRABS.fetch_add(1, Relaxed);
            }
        }
        Ok(())
    }

    /// Test 72: at the Present, this frame's remembered pictures are copied into set `i` (on the
    /// graphics card; the set's event query, issued after, covers these copies too).
    unsafe fn apply_pending(&mut self, dev: usize, i: usize) -> Result<(), String> {
        type Desc = unsafe extern "system" fn(usize, *mut [u32; 8]) -> i32;
        type CreateRt = unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, i32, *mut usize, *mut c_void) -> i32;
        type CreateOff = unsafe extern "system" fn(usize, u32, u32, u32, u32, *mut usize, *mut c_void) -> i32;
        type Stretch = unsafe extern "system" fn(usize, usize, *const c_void, usize, *const c_void, u32) -> i32;
        let vt = *(dev as *const usize);
        let create_rt: CreateRt = std::mem::transmute(func(vt, CREATE_RENDER_TARGET));
        let create_off: CreateOff = std::mem::transmute(func(vt, CREATE_OFFSCREEN_PLAIN_SURFACE));
        let stretch: Stretch = std::mem::transmute(func(vt, STRETCH_RECT));
        let (w, h, fmt) = (self.w, self.h, self.format);
        let (world, depth, r, view) = (self.pending_world, self.pending_depth, self.pending_refs, self.pending_view);
        let s = &mut self.sets[i];
        s.has_world = false;
        s.has_depth = false;
        s.view = View::default();
        if world != 0 {
            if s.world_rt == 0 {
                let hr = create_rt(dev, w, h, fmt, 0, 0, 0, &mut s.world_rt, std::ptr::null_mut());
                if hr != S_OK {
                    return Err(format!("CreateRenderTarget world {w}x{h}: 0x{:08x}", hr as u32));
                }
                let hr = create_off(dev, w, h, fmt, POOL_SYSTEMMEM, &mut s.world_sys, std::ptr::null_mut());
                if hr != S_OK {
                    release(&mut s.world_rt);
                    return Err(format!("CreateOffscreenPlainSurface world: 0x{:08x}", hr as u32));
                }
            }
            // (Lua's copy may have another 8-bit format; StretchRect converts.)
            let hr = stretch(dev, world, std::ptr::null(), s.world_rt, std::ptr::null(), 0 /* D3DTEXF_NONE */);
            if hr != S_OK {
                return Err(format!("StretchRect (world): 0x{:08x}", hr as u32));
            }
            s.has_world = true;
            s.view = view;
        }
        if depth != 0 && s.has_world {
            let desc: Desc = std::mem::transmute(func(*(depth as *const usize), SURFACE_GET_DESC));
            let mut d = [0u32; 8];
            desc(depth, &mut d);
            let (dw, dh) = (d[6], d[7]);
            if s.depth_rt == 0 || (s.depth_w, s.depth_h) != (dw, dh) {
                release(&mut s.depth_rt);
                release(&mut s.depth_sys);
                let hr = create_rt(dev, dw, dh, FMT_A16B16G16R16F, 0, 0, 0, &mut s.depth_rt, std::ptr::null_mut());
                if hr != S_OK {
                    return Err(format!("CreateRenderTarget depth {dw}x{dh}: 0x{:08x}", hr as u32));
                }
                let hr = create_off(dev, dw, dh, FMT_A16B16G16R16F, POOL_SYSTEMMEM, &mut s.depth_sys, std::ptr::null_mut());
                if hr != S_OK {
                    release(&mut s.depth_rt);
                    return Err(format!("CreateOffscreenPlainSurface depth: 0x{:08x}", hr as u32));
                }
                s.depth_w = dw;
                s.depth_h = dh;
            }
            let hr = stretch(dev, depth, std::ptr::null(), s.depth_rt, std::ptr::null(), 0);
            if hr != S_OK {
                return Err(format!("StretchRect (depth): 0x{:08x}", hr as u32));
            }
            s.has_depth = true;
            s.depth_refs = r;
        }
        Ok(())
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
        // Step W2: the world layer after the HUD picture, its depth after that (if they fit).
        let mut flags = F_PREMULTIPLIED | F_SRGB | if BLUR.load(Relaxed) { F_BLUR } else { 0 };
        let (mut dw, mut dh) = (0u32, 0u32);
        if s.has_world {
            let layer = row * h as usize;
            let depth_bytes = if s.has_depth { (s.depth_w * s.depth_h * 4) as usize } else { 0 };
            if 2 * layer + depth_bytes > MASK_OFF {
                LAYERS_TOO_BIG.fetch_add(1, Relaxed);
            } else {
                let world = dst.add(layer);
                let tw = qpc();
                copy_surface(dev, s.world_rt, s.world_sys, |bits, pitch| {
                    for y in 0..h as usize {
                        std::ptr::copy_nonoverlapping(bits.add(y * pitch), world.add(y * row), row);
                    }
                })?;
                flags |= F_WORLD;
                WORLD_SENT.fetch_add(1, Relaxed);
                WORLD_TICKS.fetch_add(qpc() - tw, Relaxed);
                if s.has_depth {
                    let td = qpc();
                    let (sw, sh, refs) = (s.depth_w, s.depth_h, s.depth_refs);
                    let tx = (s.view.fov.max(1.0).min(179.0) as f64 * 0.5).to_radians().tan();
                    let ty = tx / (s.view.aspect.max(0.1) as f64);
                    let depth = world.add(layer) as *mut f32;
                    let stats = copy_surface(dev, s.depth_rt, s.depth_sys, |bits, pitch| depth_to_distance(bits, pitch, sw, sh, refs, (tx, ty), depth))?;
                    let Some(stats) = stats else {
                        // No calibration yet: the world layer goes without depth.
                        block.publish(&SlotHdr {
                            width: w, height: h, format: FMT_BGRA8, flags, frame: s.frame, capture_qpc: s.qpc, row_bytes: row as u32,
                            view_eye: s.view.eye, view_angles: s.view.angles, view_fov: s.view.fov, view_aspect: s.view.aspect,
                            view_epoch: s.view.epoch, ..Default::default()
                        });
                        return Ok(());
                    };
                    DEPTH_PX_VALID.fetch_add(stats.0, Relaxed);
                    DEPTH_PX_FOREIGN.fetch_add(stats.1, Relaxed);
                    DEPTH_PX_ALL.fetch_add((sw * sh) as u64, Relaxed);
                    let (solid, with_depth) = agreement(world, row, w, h, depth, sw, sh);
                    AGREE_SOLID.fetch_add(solid, Relaxed);
                    AGREE_DEPTH.fetch_add(with_depth, Relaxed);
                    CENTRE_DEPTH.store(centre_depth(depth, sw, sh).to_bits(), Relaxed);
                    flags |= F_DEPTH;
                    (dw, dh) = (sw, sh);
                    DEPTH_SENT.fetch_add(1, Relaxed);
                    DEPTH_TICKS.fetch_add(qpc() - td, Relaxed);
                }
            }
        }
        let v = if flags & F_WORLD != 0 { s.view } else { View::default() };
        block.publish(&SlotHdr {
            width: w, height: h, format: FMT_BGRA8, flags, frame: s.frame,
            capture_qpc: s.qpc, row_bytes: row as u32, depth_width: dw as u16, depth_height: dh as u16,
            view_eye: v.eye, view_angles: v.angles, view_fov: v.fov, view_aspect: v.aspect, view_epoch: v.epoch,
            ..Default::default()
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

/// Step W2: GetRenderTargetData into the system-memory surface, then `read(bits, pitch)` while locked.
unsafe fn copy_surface<R>(dev: usize, rt: usize, sys: usize, read: impl FnOnce(*const u8, usize) -> R) -> Result<R, String> {
    type Grtd = unsafe extern "system" fn(usize, usize, usize) -> i32;
    #[repr(C)]
    struct Locked { pitch: i32, bits: *mut u8 }
    type Lock = unsafe extern "system" fn(usize, *mut Locked, *const c_void, u32) -> i32;
    type Unlock = unsafe extern "system" fn(usize) -> i32;
    let grtd: Grtd = std::mem::transmute(func(*(dev as *const usize), GET_RENDER_TARGET_DATA));
    let hr = grtd(dev, rt, sys);
    if hr != S_OK {
        return Err(format!("GetRenderTargetData (layer): 0x{:08x}", hr as u32));
    }
    let svt = *(sys as *const usize);
    let lock: Lock = std::mem::transmute(func(svt, LOCK_RECT));
    let unlock: Unlock = std::mem::transmute(func(svt, UNLOCK_RECT));
    let mut l = Locked { pitch: 0, bits: std::ptr::null_mut() };
    let hr = lock(sys, &mut l, std::ptr::null(), LOCK_READONLY);
    if hr != S_OK || l.bits.is_null() {
        return Err(format!("LockRect (layer): 0x{:08x}", hr as u32));
    }
    let r = read(l.bits, l.pitch as usize);
    unlock(sys);
    Ok(r)
}

/// IEEE half float to f32.
pub(crate) fn half(h: u16) -> f32 {
    let (sign, exp, man) = ((h >> 15) as u32, ((h >> 10) & 0x1F) as u32, (h & 0x3FF) as u32);
    let bits = match (exp, man) {
        (0, 0) => sign << 31,
        (0, m) => {
            // Subnormal: normalize.
            let mut e = 127 - 15 + 1;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            (sign << 31) | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
        (0x1F, m) => (sign << 31) | (0xFF << 23) | (m << 13),
        (e, m) => (sign << 31) | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

/// After test 73 (Source's DepthWrite drew nothing in the main view): the depth picture is drawn with
/// fog. Every model is black under white linear fog from 0 to Lua's fog end, so a pixel's grey level
/// grows with its distance. Whether the fog measures distance along the view (planar) or along the
/// ray to the pixel (radial), and its exact scale, come from Lua's squares, 8 x 8 depth pixels each:
/// A at (4, 4) and C at (4, h/2), both `refs[0]` away along the view, B at (12, 4) `refs[1]` away,
/// and D at (20, 4) plain white without fog (shows the picture receives drawing at all). Their
/// areas are erased afterwards.
const SQ: usize = 8;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Cal {
    /// distance = (a x grey + b), divided by the pixel's ray length if radial.
    a: f64,
    b: f64,
    radial: bool,
}
/// The newest good calibration, and the newest squares' raw values (grey of A, B, C, and D's red),
/// for the log.
static CAL: Mutex<Option<Cal>> = Mutex::new(None);
static SQUARES: Mutex<[f32; 4]> = Mutex::new([0.0; 4]);
static CAL_MISSING: AtomicU64 = AtomicU64::new(0);

/// The ray length (1 along the view) through depth pixel (x, y), for half-view tangents (tx, ty).
fn ray_len(x: f64, y: f64, w: u32, h: u32, (tx, ty): (f64, f64)) -> f64 {
    let nx = (x + 0.5) / w as f64 * 2.0 - 1.0;
    let ny = 1.0 - (y + 0.5) / h as f64 * 2.0;
    (1.0 + (nx * tx).powi(2) + (ny * ty).powi(2)).sqrt()
}

/// After test 73: the calibration from the squares' grey levels (A, B, C at refs 0, 1, 0), or None
/// if they don't make sense (missing, or a model in front of them).
pub(crate) fn calibrate(g: [f64; 3], refs: [f32; 2], w: u32, h: u32, tan: (f64, f64)) -> Option<Cal> {
    let (ga, gb, gc) = (g[0], g[1], g[2]);
    let (d1, d2) = (refs[0] as f64, refs[1] as f64);
    if !(ga > 0.0 && gb > ga + 1e-4 && gc > 0.0 && d2 > d1 && d1 > 0.0) {
        return None;
    }
    let (ka, kb, kc) = (
        ray_len(4.0, 4.0, w, h, tan),
        ray_len(12.0, 4.0, w, h, tan),
        ray_len(4.0, (h / 2) as f64, w, h, tan),
    );
    // Planar: grey is linear in the distance along the view (C reads like A).
    let ap = (d2 - d1) / (gb - ga);
    let bp = d1 - ap * ga;
    // Radial: grey is linear in the distance along the ray (C reads nearer than A).
    let ar = (d2 * kb - d1 * ka) / (gb - ga);
    let br = d1 * ka - ar * ga;
    let gc_radial = (d1 * kc - br) / ar;
    let radial = (gc - gc_radial).abs() < (gc - ga).abs();
    Some(if radial { Cal { a: ar, b: br, radial } } else { Cal { a: ap, b: bp, radial } })
}

/// Step W2 / after test 73: the fog depth picture (RGBA16F: grey, alpha 1) into distances along
/// the view (Garry's Mod units) at `out`, w x h floats; anything else is 0 (no depth). Returns
/// (pixels with a depth, "foreign" pixels: drawn, but not grey with alpha 1), or None without any
/// calibration yet.
unsafe fn depth_to_distance(bits: *const u8, pitch: usize, w: u32, h: u32, refs: [f32; 2], tan: (f64, f64), out: *mut f32) -> Option<(u64, u64)> {
    const ONE: u16 = 0x3C00;
    let px = |x: usize, y: usize| std::ptr::read_unaligned((bits.add(y * pitch) as *const u16).add(x * 4) as *const [u16; 4]);
    let grey = |p: [u16; 4]| -> Option<f64> {
        let (r, g, b) = (half(p[0]) as f64, half(p[1]) as f64, half(p[2]) as f64);
        (p[3] == ONE && r > 0.0 && (r - g).abs() < 2e-3 && (r - b).abs() < 2e-3).then_some(r)
    };
    let big_enough = (w as usize) > 3 * SQ + 2 && (h as usize) > 2 * SQ + 2;
    let mut cal = *CAL.lock().unwrap_or_else(|e| e.into_inner());
    if big_enough {
        let (pa, pb, pc, pd) = (px(4, 4), px(12, 4), px(4, h as usize / 2), px(20, 4));
        *SQUARES.lock().unwrap_or_else(|e| e.into_inner()) = [half(pa[0]), half(pb[0]), half(pc[0]), half(pd[0])];
        match (grey(pa), grey(pb), grey(pc)) {
            (Some(ga), Some(gb), Some(gc)) => match calibrate([ga, gb, gc], refs, w, h, tan) {
                Some(c) => {
                    cal = Some(c);
                    *CAL.lock().unwrap_or_else(|e| e.into_inner()) = cal;
                }
                None => {
                    CAL_MISSING.fetch_add(1, Relaxed);
                }
            },
            _ => {
                CAL_MISSING.fetch_add(1, Relaxed);
            }
        }
    }
    let cal = cal?;
    let (mut valid, mut foreign) = (0u64, 0u64);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = px(x, y);
            let d = if p == [0; 4] {
                0.0
            } else if let Some(g) = grey(p) {
                if g >= 0.999 {
                    0.0 // beyond the fog's end: too far to know (always shown)
                } else {
                    valid += 1;
                    let along = cal.a * g + cal.b;
                    (if cal.radial { along / ray_len(x as f64, y as f64, w, h, tan) } else { along }).max(0.0) as f32
                }
            } else {
                foreign += 1;
                0.0
            };
            *out.add(y * w as usize + x) = d;
        }
    }
    // The squares aren't Garry's Mod objects.
    let erase = |x0: usize, x1: usize, y0: usize, y1: usize| {
        for y in y0..y1.min(h as usize) {
            for x in x0..x1.min(w as usize) {
                *out.add(y * w as usize + x) = 0.0;
            }
        }
    };
    erase(0, 3 * SQ + 2, 0, SQ + 2);
    let hm = h as usize / 2;
    erase(0, SQ + 2, hm.saturating_sub(SQ / 2 + 1), hm + SQ / 2 + 1);
    Some((valid, foreign))
}

/// Step W2: of the world layer's solid pixels (alpha 255; every 4th in each direction), how many
/// have a depth in the 2 x 2 depth pixels nearest them. Low agreement = the depth view doesn't line
/// up with the picture. Returns (solid, solid with a depth).
unsafe fn agreement(world: *const u8, row: usize, w: u32, h: u32, depth: *const f32, dw: u32, dh: u32) -> (u64, u64) {
    let (mut solid, mut with) = (0u64, 0u64);
    if dw == 0 || dh == 0 {
        return (0, 0);
    }
    for y in (0..h as usize).step_by(4) {
        for x in (0..w as usize).step_by(4) {
            if *world.add(y * row + x * 4 + 3) != 255 {
                continue;
            }
            solid += 1;
            // Depth pixel centres around this pixel (the depth picture covers the same view).
            let fx = (x as f32 + 0.5) * dw as f32 / w as f32 - 0.5;
            let fy = (y as f32 + 0.5) * dh as f32 / h as f32 - 0.5;
            let (x0, y0) = (fx.floor().max(0.0) as u32, fy.floor().max(0.0) as u32);
            let mut found = false;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (xx, yy) = ((x0 + dx).min(dw - 1), (y0 + dy).min(dh - 1));
                found |= *depth.add((yy * dw + xx) as usize) > 0.0;
            }
            with += found as u64;
        }
    }
    (solid, with)
}

/// Step W2: the nearest depth within 4 depth pixels of the centre; 0 if none.
unsafe fn centre_depth(depth: *const f32, dw: u32, dh: u32) -> f64 {
    let (cx, cy) = (dw as i64 / 2, dh as i64 / 2);
    let mut best = 0.0f32;
    for y in (cy - 4).max(0)..(cy + 5).min(dh as i64) {
        for x in (cx - 4).max(0)..(cx + 5).min(dw as i64) {
            let d = *depth.add((y * dw as i64 + x) as usize);
            if d > 0.0 && (best == 0.0 || d < best) {
                best = d;
            }
        }
    }
    best as f64
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
        cap.pending_world = 0; // (belonged to the old device too)
        cap.pending_depth = 0;
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

    // 2. This frame, if Lua prepared it (with its world layer / depth, if Lua grabbed them).
    if !PREPARED.swap(false, AcqRel) {
        UNPREPARED.fetch_add(1, Relaxed);
        cap.drop_pending();
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
                Some(i) => {
                    // This frame's world layer and depth first, then the picture (and the query).
                    let layers = cap.apply_pending(dev, i);
                    if let Err(e) = layers {
                        fail(e);
                        cap.sets[i].has_world = false;
                        cap.sets[i].has_depth = false;
                    }
                    cap.start(dev, bb, i).map(|()| { STARTED.fetch_add(1, Relaxed); })
                }
            },
        }
    };
    release(&mut bb);
    cap.drop_pending();
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

/// gmsky.OverlayGrabWorld(): step W2, the picture being drawn now is this frame's world layer.
pub fn grab_world() {
    let mut cap = CAP.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = unsafe { cap.grab(None) } {
        fail(e);
    }
}

/// gmsky.OverlayGrabDepth(d1, d2): step W2, the render target being drawn now (Lua's RGBA16F depth
/// picture) is this frame's depth picture; after test 73: its calibration squares are d1 and d2 away.
pub fn grab_depth(d1: f32, d2: f32) {
    let mut cap = CAP.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = unsafe { cap.grab(Some([d1, d2])) } {
        fail(e);
    }
}

/// gmsky.OverlayView(...): step W4, the camera this frame's world layer is drawn with (eye already
/// made relative to the anchor of origin epoch `v.epoch` by the caller).
pub fn set_view(v: View) {
    let mut cap = CAP.lock().unwrap_or_else(|e| e.into_inner());
    if !ENABLED.load(Acquire) || cap.w == 0 {
        return;
    }
    cap.pending_view = v;
}

/// gmsky.OverlayDepthCentre(): the nearest Garry's Mod depth at the screen centre in the newest
/// depth picture (Garry's Mod units), None if there was none.
pub fn depth_centre() -> Option<f64> {
    let d = f64::from_bits(CENTRE_DEPTH.load(Relaxed));
    (d > 0.0).then_some(d)
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
    let (wg, dg, refused, ws, ds, big) = (take(&WORLD_GRABS), take(&DEPTH_GRABS), take(&GRABS_REFUSED), take(&WORLD_SENT), take(&DEPTH_SENT), take(&LAYERS_TOO_BIG));
    let (pv, pf, pa, solid, agree) = (take(&DEPTH_PX_VALID), take(&DEPTH_PX_FOREIGN), take(&DEPTH_PX_ALL), take(&AGREE_SOLID), take(&AGREE_DEPTH));
    let pct = |a: u64, b: u64| if b > 0 { a as f64 * 100.0 / b as f64 } else { 0.0 };
    let (wt, dt) = (take(&WORLD_TICKS), take(&DEPTH_TICKS));
    let each = |t: u64, n: u64| if n > 0 { ms(t) / n as f64 } else { 0.0 };
    let cal = *CAL.lock().unwrap_or_else(|e| e.into_inner());
    let sq = *SQUARES.lock().unwrap_or_else(|e| e.into_inner());
    let cal_line = match cal {
        Some(c) => format!("fog depth {} (distance = {:.1} x grey {:+.1})", if c.radial { "along the ray" } else { "along the view" }, c.a, c.b),
        None => "no calibration yet".to_string(),
    };
    let layers = format!(
        "world layer: {wg} grabbed, {ws} sent ({:.3} ms each to normal memory); depth: {dg} grabbed, {ds} sent ({:.3} ms each), \
         {cal_line}; squares A {:.4} B {:.4} C {:.4} white {:.4}, {} pictures without a good calibration; \
         pixels {:.2}% with depth, {:.2}% foreign; agreement {:.1}% of {solid} solid world pixels have a depth; {refused} grabs refused, {big} too big for the slot",
        each(wt, ws), each(dt, ds), sq[0], sq[1], sq[2], sq[3], take(&CAL_MISSING), pct(pv, pa), pct(pf, pa), pct(agree, solid));
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
         start {:.3} ms/frame, finish {:.3} ms/frame (max {:.3}), {:.2} frames from copy to publish; opaque {:.1}% avg, {:.1}% max; {layers}; {block}",
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

    /// After test 73: the fog's calibration tells "along the view" from "along the ray" fog by square
    /// C (same distance along the view as A, nearer the middle of the screen), and decodes both.
    #[test]
    fn fog_calibration() {
        let (w, h, tan) = (640u32, 360u32, (1.589f64, 0.894f64));
        let (d1, d2, end) = (500.0f64, 4000.0f64, 20000.0f64);
        let ka = ray_len(4.0, 4.0, w, h, tan);
        let kb = ray_len(12.0, 4.0, w, h, tan);
        let kc = ray_len(4.0, 180.0, w, h, tan);
        // Along the view: grey = distance / end.
        let c = calibrate([d1 / end, d2 / end, d1 / end], [d1 as f32, d2 as f32], w, h, tan).unwrap();
        assert!(!c.radial && (c.a * (1234.0 / end) + c.b - 1234.0).abs() < 1e-6, "{c:?}");
        // Along the ray: grey = ray distance / end; C (nearer the middle) reads nearer than A.
        let c = calibrate([d1 * ka / end, d2 * kb / end, d1 * kc / end], [d1 as f32, d2 as f32], w, h, tan).unwrap();
        assert!(c.radial, "{c:?}");
        let k = ray_len(300.0, 100.0, w, h, tan);
        assert!(((c.a * (1234.0 * k / end) + c.b) / k - 1234.0).abs() < 1e-6, "{c:?}");
        // Nonsense (B not further than A, or missing): no calibration.
        assert!(calibrate([0.1, 0.05, 0.1], [500.0, 4000.0], w, h, tan).is_none());
        assert!(calibrate([0.0, 0.2, 0.0], [500.0, 4000.0], w, h, tan).is_none());
    }

    #[test]
    fn half_floats() {
        assert_eq!(half(0x3C00), 1.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0x4400), 4.0);
        assert_eq!(half(0xC000), -2.0);
        assert_eq!(half(0x0000), 0.0);
        assert_eq!(half(0x0001), 2f32.powi(-24)); // smallest subnormal
        assert_eq!(half(0x7BFF), 65504.0);
    }

    /// Step W2 on a real hidden device: mid-frame, a world layer (an A8R8G8B8 target Lua would be
    /// drawing into) and a half-size RGBA16F depth picture are grabbed; the Present then publishes
    /// the HUD picture, the world layer after it and the depth (as distances) after that, with the
    /// flags and depth size set. Depth: red-only with alpha 1 = red x far; another colour =
    /// "foreign" (0); nothing drawn = 0. A frame without grabs has neither layer.
    #[test]
    fn world_layer_and_depth_arrive() {
        let _one_at_a_time = GLOBAL_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let name = format!(r"Local\GmodSkyrim_overlay_test_{}_layers", std::process::id());
        *BLOCK_NAME.lock().unwrap() = Some(name.clone());
        let mut reader = OverlayMapping::open_named(&name).unwrap();
        reader.attach(false);
        let ms = |t: u64| t as f64 / 10_000.0;
        let (hud, world) = (0x40_10_20_30u32, 0xFF_80_40_20u32);
        with_test_device_sized(true, 64, 48, FMT_A8R8G8B8, |dev, vt| unsafe {
            type Clear = unsafe extern "system" fn(usize, u32, *const [i32; 4], u32, u32, f32, u32) -> i32;
            type Present = unsafe extern "system" fn(usize, *const c_void, *const c_void, *mut c_void, *const c_void) -> i32;
            type CreateRt = unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, i32, *mut usize, *mut c_void) -> i32;
            type SetRt = unsafe extern "system" fn(usize, u32, usize) -> i32;
            type GetBb = unsafe extern "system" fn(usize, u32, u32, u32, *mut usize) -> i32;
            let clear: Clear = std::mem::transmute(func(vt, 43));
            let present: Present = std::mem::transmute(func(vt, 17));
            let create_rt: CreateRt = std::mem::transmute(func(vt, CREATE_RENDER_TARGET));
            let set_rt: SetRt = std::mem::transmute(func(vt, 37));
            let get_bb: GetBb = std::mem::transmute(func(vt, GET_BACK_BUFFER));
            let null = std::ptr::null();
            let (mut bb, mut wrt, mut drt) = (0usize, 0usize, 0usize);
            assert_eq!(get_bb(dev, 0, 0, 0, &mut bb), 0);
            assert_eq!(create_rt(dev, 64, 48, FMT_A8R8G8B8, 0, 0, 0, &mut wrt, std::ptr::null_mut()), 0);
            assert_eq!(create_rt(dev, 32, 24, FMT_A16B16G16R16F, 0, 0, 0, &mut drt, std::ptr::null_mut()), 0, "RGBA16F render target");
            enable(true);
            status(ms);
            // A first prepared frame makes the copies (grabs before that are refused).
            grab_world();
            mark_prepared();
            on_present(dev);
            present(dev, null, null, std::ptr::null_mut(), null);
            // Each frame grabs again (as the game does): a frame whose copies find all three sets
            // busy is skipped, and the next frame sends its own.
            let frame = |grab: bool| -> u32 {
              for _ in 0..500 {
                if grab {
                    set_view(View { eye: [1.0, 2.0, 3.0], angles: [10.0, 20.0, 0.5], fov: 106.0, aspect: 1.5, epoch: 7 });
                    // Test 72: drawn into after the grab, as Source finishes its held-back drawing
                    // later; the copy made at the Present must have it.
                    set_rt(dev, 0, wrt);
                    assert_eq!(clear(dev, 0, std::ptr::null(), 1, 0xFF_00_00_00, 1.0, 0), 0);
                    grab_world();
                    assert_eq!(clear(dev, 0, std::ptr::null(), 1, world, 1.0, 0), 0);
                    set_rt(dev, 0, drt);
                    // After test 73, fog depth: grey 0x40 everywhere; squares A and C (500 away) grey
                    // 0x10, B (4,000 away) grey 0x80, D white; an off-grey patch and an empty patch.
                    assert_eq!(clear(dev, 0, std::ptr::null(), 1, 0xFF_40_40_40, 1.0, 0), 0);
                    for (r, c) in [([0i32, 0, 8, 8], 0xFF_10_10_10u32), ([8, 0, 16, 8], 0xFF_80_80_80), ([0, 8, 8, 16], 0xFF_10_10_10),
                                   ([16, 0, 24, 8], 0xFF_FF_FF_FF), ([24, 0, 32, 12], 0xFF_00_FF_00), ([16, 12, 32, 24], 0)] {
                        assert_eq!(clear(dev, 1, &r, 1, c, 1.0, 0), 0);
                    }
                    grab_depth(500.0, 4000.0);
                    set_rt(dev, 0, bb);
                }
                assert_eq!(clear(dev, 0, std::ptr::null(), 1, hud, 1.0, 0), 0);
                mark_prepared();
                on_present(dev);
                present(dev, null, null, std::ptr::null_mut(), null);
                // (Pictures started before this frame can still arrive first: skip them.)
                if let Some(slot) = reader.take() {
                    if (reader.slot_hdr(slot).flags & F_WORLD != 0) == grab {
                        return slot;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
              }
              panic!("no picture arrived");
            };
            // Drain what the first frames published, then one frame with both grabs.
            while reader.take().is_some() {}
            let slot = frame(true);
            let line = status(ms);
            println!("{line}");
            let h = reader.slot_hdr(slot);
            assert_eq!(h.flags & (F_WORLD | F_DEPTH), F_WORLD | F_DEPTH, "{line}");
            assert_eq!((h.depth_width, h.depth_height), (32, 24));
            let base = reader.slot_ptr(slot);
            let px = |off: usize, i: usize| std::ptr::read_unaligned((base.add(off) as *const u32).add(i));
            assert_eq!(px(0, 0), hud);
            assert_eq!(px(64 * 48 * 4, 0), world);
            assert_eq!(px(64 * 48 * 4, 64 * 48 - 1), world);
            let depth = std::slice::from_raw_parts(base.add(2 * 64 * 48 * 4) as *const f32, 32 * 24);
            assert_eq!((depth[0], depth[4 * 32 + 20], depth[12 * 32 + 4]), (0.0, 0.0, 0.0), "the squares A, D, C are erased");
            assert_eq!(depth[4 * 32 + 28], 0.0, "off-grey: no depth");
            assert_eq!(depth[20 * 32 + 20], 0.0, "nothing drawn");
            // Grey is linear in distance along the view: 500 at 0x10, 4,000 at 0x80.
            let (g1, g2, g) = (16.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0);
            let want = (500.0 + (4000.0 - 500.0) * (g - g1) / (g2 - g1)) as f32;
            assert!((depth[20 * 32 + 12] - want).abs() < 3.0, "grey 0x40: {} vs {want}", depth[20 * 32 + 12]);
            assert!(line.contains("world layer: ") && line.contains(" 0 failures"), "{line}");
            assert_eq!((h.view_eye, h.view_angles, h.view_fov, h.view_aspect, h.view_epoch),
                ([1.0, 2.0, 3.0], [10.0, 20.0, 0.5], 106.0, 1.5, 7), "step W4: the picture's camera");
            assert!(line.contains("pixels 54.17% with depth, 12.50% foreign"), "{line}"); // (every depth picture is the same)
            assert!(line.contains(" 0 failures"), "{line}");
            assert!(line.contains("fog depth along the view"), "{line}");
            assert!((depth_centre().unwrap() - want as f64).abs() < 3.0, "{:?}", depth_centre());
            // The next frame without grabs: the HUD picture only.
            let slot = frame(false);
            assert_eq!(reader.slot_hdr(slot).flags & (F_WORLD | F_DEPTH), 0);
            assert_eq!(reader.slot_hdr(slot).view_epoch, 0, "no world layer: no camera");
            release(&mut wrt);
            release(&mut drt);
            release(&mut bb);
            on_reset();
            enable(false);
        });
        *BLOCK_NAME.lock().unwrap() = None;
        CAP.lock().unwrap().block = None;
    }
}
