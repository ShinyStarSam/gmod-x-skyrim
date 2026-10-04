//! gmsky module: the Garry's Mod half of the link.
//!
//! Loaded by the stubs (gmod/stub) through gmsky_open(). Opens the shared memory, gives Lua a
//! `gmsky` table of functions, then runs gmod/lua/gmsky/<server|client>.lua from this project.
//! Logs: logs/gmod_server.log, logs/gmod_client.log

mod lua;

use gmsky_protocol::shm::{now_ms, now_qpc, Mapping, Side};
use gmsky_protocol::{PlayerState, PF_INPUT_APPLIED};
use lua::{Api, State};
use std::ffi::c_int;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Must match STUB_ABI_VERSION in gmod/stub. Changing it means re-installing the stubs.
const ABI_VERSION: u32 = 1;

const SIDE_SERVER: u32 = 0;
const SIDE_CLIENT: u32 = 1;

fn side_name(side: u32) -> &'static str {
    if side == SIDE_CLIENT { "client" } else { "server" }
}

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static MAPPING: OnceLock<Mapping> = OnceLock::new();
/// Last failed open: (time in ms, error). A failed open is retried at most every RETRY_MS.
static OPEN_ERROR: Mutex<Option<(u64, String)>> = Mutex::new(None);
const RETRY_MS: u64 = 2000;
/// Set once we've logged that another process took over the Garry's Mod side.
static LOST_OWNERSHIP_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LOGS: Mutex<[Option<File>; 2]> = Mutex::new([None, None]);

#[derive(Default)]
struct LinkState {
    epoch: u32,
    anchor: [f64; 3],
    writes: u64,
}
static LINK: Mutex<LinkState> = Mutex::new(LinkState { epoch: 0, anchor: [0.0; 3], writes: 0 });

/// Input from Skyrim is used only while Skyrim keeps writing it: Skyrim writes every frame, so a
/// slot that hasn't changed for this long means Skyrim paused or stopped.
const INPUT_STALE_MS: u64 = 250;
struct InputTrack {
    last_seq: u32,
    last_change_ms: u64,
    /// Mouse/wheel/noclip-press totals already applied; None = start fresh on the next active read.
    base: Option<(i64, i64, i32, u32)>,
    /// GmskyInput.frame of the last active read (reported back to Skyrim with the player state).
    applied_frame: u64,
}
static INPUT: Mutex<InputTrack> = Mutex::new(InputTrack { last_seq: 0, last_change_ms: 0, base: None, applied_frame: 0 });

fn root() -> PathBuf {
    ROOT.get().cloned().unwrap_or_else(|| PathBuf::from("."))
}

/// logs/ in the project. GMSKY_LOG_DIR lets the test harness log elsewhere, so it never
/// overwrites in-game logs.
fn log_dir() -> PathBuf {
    std::env::var_os("GMSKY_LOG_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("logs"))
}

fn log(side: u32, msg: &str) {
    let mut logs = LOGS.lock().unwrap_or_else(|e| e.into_inner());
    let slot = &mut logs[(side == SIDE_CLIENT) as usize];
    if slot.is_none() {
        let dir = log_dir();
        let _ = std::fs::create_dir_all(&dir);
        *slot = OpenOptions::new().create(true).write(true).truncate(true)
            .open(dir.join(format!("gmod_{}.log", side_name(side)))).ok();
    }
    if let Some(f) = slot {
        let _ = writeln!(f, "[{}] {}", now_ms(), msg);
        let _ = f.flush();
    }
}

/// The shared memory, opening it on first use. A failure (e.g. a protocol-version mismatch with a
/// Skyrim that is still running) is retried every RETRY_MS, so it recovers without a restart.
fn mapping() -> Result<&'static Mapping, String> {
    if let Some(m) = MAPPING.get() {
        return Ok(m);
    }
    let mut err = OPEN_ERROR.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, e)) = err.as_ref() {
        if now_ms().saturating_sub(*at) < RETRY_MS {
            return Err(e.clone());
        }
    }
    match Mapping::open(Side::Gmod) {
        Ok(m) => {
            *err = None;
            Ok(MAPPING.get_or_init(|| m))
        }
        Err(e) => {
            if err.as_ref().map_or(true, |(_, prev)| *prev != e) {
                log(SIDE_SERVER, &format!("ERROR opening shared memory (retrying every {RETRY_MS} ms): {e}"));
            }
            *err = Some((now_ms(), e.clone()));
            Err(e)
        }
    }
}

/// The mapping, if this process may write the Garry's Mod side. Logs once if another
/// Garry's Mod-side process (e.g. a forgotten fake_gmod) has taken over.
fn writable_mapping() -> Option<&'static Mapping> {
    let m = mapping().ok()?;
    if m.owns_side() {
        LOST_OWNERSHIP_LOGGED.store(false, std::sync::atomic::Ordering::Relaxed);
        return Some(m);
    }
    if !LOST_OWNERSHIP_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log(SIDE_SERVER, &format!("ERROR: another process (pid {}) is writing the Garry's Mod side; not writing until it exits",
            m.shared().header.gmod_pid.load(std::sync::atomic::Ordering::Acquire)));
    }
    None
}

#[no_mangle]
pub extern "C" fn gmsky_abi_version() -> u32 {
    ABI_VERSION
}

/// Called by the stub from gmod13_open. `root` is the project folder (UTF-16, null-terminated).
#[no_mangle]
pub unsafe extern "C" fn gmsky_open(l: *mut State, side: u32, root_w: *const u16) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if !root_w.is_null() {
            let mut n = 0;
            while *root_w.add(n) != 0 { n += 1; }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(root_w, n));
            let _ = ROOT.set(PathBuf::from(s));
        }
        log(side, &format!("gmsky_open: side={} root={}", side_name(side), root().display()));
        let a = match lua::api() {
            Ok(a) => a,
            Err(e) => { log(side, &format!("ERROR: {e}")); return; }
        };
        match mapping() {
            Ok(m) => log(side, &format!("shared memory open, skyrim alive={}", m.peer_alive())),
            Err(e) => log(side, &format!("ERROR opening shared memory: {e}")),
        }
        register_functions(a, l, side);
        run_side_script(a, l, side);
    }));
    if r.is_err() {
        log(side, "PANIC in gmsky_open");
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn gmsky_close(_l: *mut State, side: u32) -> c_int {
    log(side, "gmsky_close (Lua state shutting down)");
    0
}

unsafe fn register_functions(a: &Api, l: *mut State, side: u32) {
    (a.createtable)(l, 0, 8);
    let funcs: [(&str, lua::CFn); 11] = [
        ("Log", l_log),
        ("Heartbeat", l_heartbeat),
        ("ReadInput", l_read_input),
        ("WritePlayerState", l_write_player_state),
        ("Status", l_status),
        ("Root", l_root),
        ("Reload", l_reload),
        ("Include", l_include),
        ("ReadTerrain", l_read_terrain),
        ("OriginEpoch", l_origin_epoch),
        ("ReadGridFile", l_read_grid_file),
    ];
    for (name, f) in funcs {
        (a.pushnumber)(l, side as f64); // upvalue 1 = side
        (a.pushcclosure)(l, f, 1);
        lua::set_field(a, l, -2, name);
    }
    lua::push_str(a, l, side_name(side));
    lua::set_field(a, l, -2, "side");
    lua::set_field(a, l, lua::GLOBALSINDEX, "gmsky");
}

unsafe fn run_side_script(a: &Api, l: *mut State, side: u32) {
    run_file(a, l, side, &format!("gmod/lua/gmsky/{}.lua", side_name(side)));
}

/// Runs a Lua file from the project folder; logs the result.
unsafe fn run_file(a: &Api, l: *mut State, side: u32, rel: &str) -> bool {
    let path = root().join(rel);
    match std::fs::read(&path) {
        Ok(code) => match lua::run_chunk(a, l, &code, rel) {
            Ok(()) => { log(side, &format!("ran {rel}")); true }
            Err(e) => { log(side, &format!("ERROR in {rel}: {e}")); false }
        },
        Err(e) => { log(side, &format!("ERROR reading {}: {e}", path.display())); false }
    }
}

unsafe fn side_of(a: &Api, l: *mut State) -> u32 {
    (a.tonumber)(l, lua::upvalueindex(1)) as u32
}

/// Wraps a Lua-callable function so a Rust panic is logged instead of crashing the game.
unsafe fn guarded(l: *mut State, f: impl FnOnce(&Api, u32) -> c_int) -> c_int {
    let Ok(a) = lua::api() else { return 0 };
    let side = side_of(a, l);
    match catch_unwind(AssertUnwindSafe(|| f(a, side))) {
        Ok(n) => n,
        Err(_) => { log(side, "PANIC in a gmsky Lua function"); 0 }
    }
}

/// gmsky.Log(text)
unsafe extern "C" fn l_log(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        log(side, &lua::to_string(a, l, 1).unwrap_or_default());
        0
    })
}

/// gmsky.Heartbeat(): tells Skyrim this Garry's Mod is alive. Called every client frame, which
/// keeps running while single-player is paused (server ticks stop then).
unsafe extern "C" fn l_heartbeat(l: *mut State) -> c_int {
    guarded(l, |_, _| {
        if let Some(m) = writable_mapping() {
            m.heartbeat();
        }
        0
    })
}

/// gmsky.ReadInput() -> active, buttons, mouse_dx, mouse_dy, wheel_delta, noclip_presses
/// active is false unless Skyrim is alive, routing input (GmskyInput.active) and wrote it within
/// INPUT_STALE_MS. The deltas are since the previous call that returned active (so presses made
/// while input went to Skyrim, e.g. in a menu, are not counted).
unsafe extern "C" fn l_read_input(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let mut t = INPUT.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        let read = mapping().ok().and_then(|m| m.shared().input.read().map(|(i, seq)| (m, i, seq)));
        let mut result = None;
        if let Some((m, inp, seq)) = read {
            if seq != t.last_seq {
                t.last_seq = seq;
                t.last_change_ms = now;
            }
            if inp.active == 1 && m.peer_alive() && now.saturating_sub(t.last_change_ms) < INPUT_STALE_MS {
                let (bx, by, bw, bn) = t.base.unwrap_or((inp.mouse_x_total, inp.mouse_y_total, inp.wheel_total, inp.noclip_presses));
                let (mut dx, mut dy) = (inp.mouse_x_total - bx, inp.mouse_y_total - by);
                if dx.abs() > 100_000 || dy.abs() > 100_000 {
                    dx = 0; // totals restarted (Skyrim restarted): start again from here
                    dy = 0;
                }
                // More than a few presses since the last tick means the total restarted: ignore it.
                let presses = match inp.noclip_presses.wrapping_sub(bn) { n if n <= 8 => n, _ => 0 };
                result = Some((inp.buttons, dx, dy, inp.wheel_total.wrapping_sub(bw), presses));
                t.base = Some((inp.mouse_x_total, inp.mouse_y_total, inp.wheel_total, inp.noclip_presses));
                t.applied_frame = inp.frame;
            }
        }
        match result {
            Some((buttons, dx, dy, dw, presses)) => {
                (a.pushboolean)(l, 1);
                (a.pushnumber)(l, buttons as f64);
                (a.pushnumber)(l, dx as f64);
                (a.pushnumber)(l, dy as f64);
                (a.pushnumber)(l, dw as f64);
                (a.pushnumber)(l, presses as f64);
            }
            None => {
                t.base = None;
                (a.pushboolean)(l, 0);
                for _ in 0..5 { (a.pushnumber)(l, 0.0); }
            }
        }
        6
    })
}

/// gmsky.Root() -> project folder
unsafe extern "C" fn l_root(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &root().display().to_string());
        1
    })
}

/// gmsky.Include(name) -> ok: runs gmod/lua/gmsky/<name>.lua from the project folder on this side
/// (name: lowercase letters, digits and _ only). Errors are logged and return false.
unsafe extern "C" fn l_include(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let name = lua::to_string(a, l, 1).unwrap_or_default();
        let ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        let ok = if ok {
            run_file(a, l, side, &format!("gmod/lua/gmsky/{name}.lua"))
        } else {
            log(side, &format!("ERROR gmsky.Include: bad name {name:?}"));
            false
        };
        (a.pushboolean)(l, ok as c_int);
        1
    })
}

/// gmsky.ReadTerrain(last_id) -> id [, origin_epoch, n, center_x, center_y, spacing, misses, heights]
/// Skyrim's newest ground grid (Phase 1 step 3), in Garry's Mod units. Returns only the id (0 = none
/// yet) when it is still `last_id`; otherwise everything, with `heights` a list of n*n numbers,
/// index i*n + j + 1 (NaN where Skyrim's ray hit nothing).
unsafe extern "C" fn l_read_terrain(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let last = (a.tonumber)(l, 1) as u32;
        let t = mapping().ok().and_then(|m| m.shared().terrain.read().map(|(t, _)| t));
        let Some(t) = t else { (a.pushnumber)(l, 0.0); return 1 };
        (a.pushnumber)(l, t.id as f64);
        if t.id == last || t.id == 0 {
            return 1;
        }
        if side != SIDE_CLIENT {
            record_grid(&t);
        }
        push_grid_rest(a, l, &t);
        8
    })
}

/// Pushes the 7 values after `id` that ReadTerrain and ReadGridFile return.
unsafe fn push_grid_rest(a: &Api, l: *mut State, t: &gmsky_protocol::Terrain) {
    (a.pushnumber)(l, t.origin_epoch as f64);
    (a.pushnumber)(l, t.n as f64);
    (a.pushnumber)(l, t.center[0]);
    (a.pushnumber)(l, t.center[1]);
    (a.pushnumber)(l, t.spacing as f64);
    (a.pushnumber)(l, t.misses as f64);
    let count = (t.n as usize * t.n as usize).min(t.height.len());
    (a.createtable)(l, count as c_int, 0);
    for (k, h) in t.height[..count].iter().enumerate() {
        (a.pushnumber)(l, *h as f64);
        (a.rawseti)(l, -2, k as c_int + 1);
    }
}

/// gmsky.ReadGridFile(path) -> the same values as ReadTerrain, from a grid recorded by
/// record_grid; or false, error. `path` is relative to the project folder and may not contain "..".
/// For replaying a recorded Skyrim grid in Garry's Mod alone (the step 3c crash investigation).
unsafe extern "C" fn l_read_grid_file(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let rel = lua::to_string(a, l, 1).unwrap_or_default();
        let fail = |msg: String| {
            (a.pushboolean)(l, 0);
            lua::push_str(a, l, &msg);
            2
        };
        if rel.is_empty() || rel.contains("..") {
            return fail(format!("bad path {rel:?}"));
        }
        let bytes = match std::fs::read(root().join(&rel)) {
            Ok(b) => b,
            Err(e) => return fail(format!("{rel}: {e}")),
        };
        let size = std::mem::size_of::<gmsky_protocol::Terrain>();
        if bytes.len() != size {
            return fail(format!("{rel}: {} bytes, a grid is {size}", bytes.len()));
        }
        let mut t = Box::new(gmsky_protocol::Terrain::default());
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), &mut *t as *mut _ as *mut u8, size);
        if t.n as usize > gmsky_protocol::TERRAIN_N {
            return fail(format!("{rel}: n = {} is too big", t.n));
        }
        (a.pushnumber)(l, t.id as f64);
        push_grid_rest(a, l, &t);
        8
    })
}

/// Keeps the last 20 grids as raw bytes in logs/terrain/grid_<id>.bin (the GmskyTerrain layout),
/// so a grid that crashes Garry's Mod's physics can be looked at and replayed (test 12).
fn record_grid(t: &gmsky_protocol::Terrain) {
    let dir = log_dir().join("terrain");
    let _ = std::fs::create_dir_all(&dir);
    let bytes = unsafe {
        std::slice::from_raw_parts(t as *const _ as *const u8, std::mem::size_of::<gmsky_protocol::Terrain>())
    };
    let _ = std::fs::write(dir.join(format!("grid_{}.bin", t.id)), bytes);
    if t.id > 20 {
        let _ = std::fs::remove_file(dir.join(format!("grid_{}.bin", t.id - 20)));
    }
}

/// gmsky.OriginEpoch() -> the origin epoch Skyrim has published (0 = none). Garry's Mod anchors to
/// it on its next WritePlayerState; Lua uses this to move the player home first (Phase 1 step 3c).
unsafe extern "C" fn l_origin_epoch(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let e = mapping().ok().and_then(|m| m.shared().origin.read().map(|(o, _)| o.epoch)).unwrap_or(0);
        (a.pushnumber)(l, e as f64);
        1
    })
}

/// gmsky.Reload(): re-runs this side's Lua script from the project folder.
unsafe extern "C" fn l_reload(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        run_side_script(a, l, side);
        0
    })
}

/// gmsky.WritePlayerState(tick, x, y, z, pitch, yaw, roll, eye_height, vx, vy, vz, flags, fov,
///                        deg_per_count, tick_ms) -> origin epoch in use (0 = no origin yet)
/// Stamps the tick with the PC's precise clock, so Skyrim can blend between ticks.
/// Also reports the Skyrim input already applied (from gmsky.ReadInput), so Skyrim can draw the
/// rest of the mouse movement ahead of Garry's Mod. deg_per_count 0 (or nil) = don't predict.
unsafe extern "C" fn l_write_player_state(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let pos = [n(2), n(3), n(4)];
        let Some(m) = writable_mapping() else { (a.pushnumber)(l, 0.0); return 1 };
        m.heartbeat();
        let s = m.shared();
        let mut link = LINK.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((o, _)) = s.origin.read() {
            if o.epoch != link.epoch {
                link.epoch = o.epoch;
                link.anchor = pos;
                log(side, &format!("new origin epoch {}: skyrim origin ({:.1}, {:.1}, {:.1}), anchor ({:.1}, {:.1}, {:.1})",
                    o.epoch, o.skyrim_origin[0], o.skyrim_origin[1], o.skyrim_origin[2], pos[0], pos[1], pos[2]));
            }
        }
        let (applied, input_frame) = {
            let t = INPUT.lock().unwrap_or_else(|e| e.into_inner());
            (t.base, t.applied_frame)
        };
        let (mx, my, _, _) = applied.unwrap_or((0, 0, 0, 0));
        s.player_state.write(&PlayerState {
            tick: n(1) as u64,
            origin_epoch: link.epoch,
            flags: n(12) as u32 | if applied.is_some() { PF_INPUT_APPLIED } else { 0 },
            anchor: link.anchor,
            pos,
            eye_height: n(8) as f32,
            pitch_deg: n(5) as f32,
            yaw_deg: n(6) as f32,
            roll_deg: n(7) as f32,
            vel: [n(9) as f32, n(10) as f32, n(11) as f32],
            fov_deg: n(13) as f32,
            input_frame,
            mouse_x_applied: mx,
            mouse_y_applied: my,
            deg_per_count: n(14) as f32,
            reserved0: 0,
            tick_qpc: now_qpc(),
            tick_ms: n(15) as f32,
            reserved1: 0,
        });
        link.writes += 1;
        (a.pushnumber)(l, link.epoch as f64);
        1
    })
}

/// gmsky.Status() -> connected, epoch, skyrim_frame, writes, error
unsafe extern "C" fn l_status(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
        match mapping() {
            Ok(m) => {
                (a.pushboolean)(l, m.peer_alive() as c_int);
                (a.pushnumber)(l, link.epoch as f64);
                let frame = m.shared().skyrim_state.read().map(|(k, _)| k.frame).unwrap_or(0);
                (a.pushnumber)(l, frame as f64);
                (a.pushnumber)(l, link.writes as f64);
                (a.settop)(l, (a.gettop)(l) + 1); // error = nil
            }
            Err(e) => {
                (a.pushboolean)(l, 0);
                (a.pushnumber)(l, 0.0);
                (a.pushnumber)(l, 0.0);
                (a.pushnumber)(l, link.writes as f64);
                lua::push_str(a, l, &e);
            }
        }
        5
    })
}
