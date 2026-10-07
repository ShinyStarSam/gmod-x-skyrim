//! gmsky module: the Garry's Mod half of the link.
//!
//! Loaded by the stubs (gmod/stub) through gmsky_open(). Opens the shared memory, gives Lua a
//! `gmsky` table of functions, then runs gmod/lua/gmsky/<server|client>.lua from this project.
//! Logs: logs/gmod_server.log, logs/gmod_client.log

mod capture;
mod d3d;
mod hitboxes;
mod lua;
mod physlist;
mod present;
mod window;
mod shapes;
mod vtf;
mod render_out;

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
/// One reader per side (index = side): the server turns input into the player's command, the
/// client into Garry's Mod's key binds and its menu cursor (Phase 3 step 3a).
static INPUT: Mutex<[InputTrack; 2]> = Mutex::new([
    InputTrack { last_seq: 0, last_change_ms: 0, base: None, applied_frame: 0 },
    InputTrack { last_seq: 0, last_change_ms: 0, base: None, applied_frame: 0 },
]);

/// Step 3a: the client side's report to the server, (time in ms, a menu cursor is up). Both
/// sides run in this one module in single-player, so a static is enough.
static CLIENT_INPUT: Mutex<(u64, bool)> = Mutex::new((0, false));
/// The server trusts the client's report for this long.
const CLIENT_INPUT_FRESH_MS: u64 = 500;

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
    let funcs: [(&str, lua::CFn); 64] = [
        ("EndProcess", l_end_process),
        ("SkyrimExited", l_skyrim_exited),
        ("CloseGame", l_close_game),
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
        ("Anchor", l_anchor),
        ("ShiftAnchor", l_shift_anchor),
        ("PollShapes", l_poll_shapes),
        ("Sweep", l_sweep),
        ("ShapesKnown", l_shapes_known),
        ("ShapesBounds", l_shapes_bounds),
        ("ShapesStatus", l_shapes_status),
        ("TrisInBox", l_tris_in_box),
        ("RampsInBox", l_ramps_in_box),
        ("RampSweep", l_ramp_sweep),
        ("RampForget", l_ramp_forget),
        ("RampCheck", l_ramp_check),
        ("DumpTris", l_dump_tris),
        ("ShapesLattice", l_shapes_lattice),
        ("ShapesRegionsArrived", l_shapes_regions_arrived),
        ("ShapesChunkVersion", l_shapes_chunk_version),
        ("GraphicsInfo", l_graphics_info),
        ("WriteLogFile", l_write_log_file),
        ("D3DState", l_d3d_state),
        ("ReadGridFile", l_read_grid_file),
        ("OverlayHook", l_overlay_hook),
        ("OverlayStatus", l_overlay_status),
        ("OverlayEnable", l_overlay_enable),
        ("OverlayPrepared", l_overlay_prepared),
        ("OverlayShot", l_overlay_shot),
        ("OverlayBlur", l_overlay_blur),
        ("OverlayWindow", l_overlay_window),
        ("SetClientInput", l_set_client_input),
        ("ClientInput", l_client_input),
        ("ReadActors", l_read_actors),
        ("HitboxActors", l_hitbox_actors),
        ("HitTrace", l_hit_trace),
        ("HitPartAt", l_hit_part_at),
        ("HitPushOut", l_hit_push_out),
        ("HitTestStart", l_hit_test_start),
        ("HitTestReport", l_hit_test_report),
        ("SendHit", l_send_hit),
        ("ReadHurts", l_read_hurts),
        ("SkyrimFlags", l_skyrim_flags),
        ("TrisInfo", l_tris_info),
        ("ConsoleWindow", l_console_window),
        ("ConsoleKey", l_console_key),
        ("WaterZ", l_water_z),
        ("RenderTexture", l_render_texture),
        ("RenderModel", l_render_model),
        ("RenderFrame", l_render_frame),
        ("RenderStatus", l_render_status),
        ("PhysList", physlist::l_phys_list),
        ("PhysListPartners", physlist::l_phys_list_partners),
        ("PhysAddress", physlist::l_phys_address),
        ("PhysAlive", physlist::l_phys_alive),
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
    guarded(l, |a, side| {
        let mut all = INPUT.lock().unwrap_or_else(|e| e.into_inner());
        let t = &mut all[(side == SIDE_CLIENT) as usize];
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

/// gmsky.SetClientInput(cursor) (client, every frame while input comes from Skyrim): the client
/// is turning Skyrim's clicks, wheel and slot keys into Garry's Mod's binds; cursor = a menu
/// cursor is up (Skyrim's mouse moves it, so the view must not turn). gmsky.SetClientInput(nil)
/// withdraws the report at once.
unsafe extern "C" fn l_set_client_input(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let mut c = CLIENT_INPUT.lock().unwrap_or_else(|e| e.into_inner());
        *c = if (a.type_)(l, 1) <= 0 { (0, false) } else { (now_ms(), (a.toboolean)(l, 1) != 0) };
        0
    })
}

/// gmsky.ClientInput() -> fresh, cursor: whether the client reported within the last 0.5 s, and
/// whether its menu cursor is up.
unsafe extern "C" fn l_client_input(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let (at, cursor) = *CLIENT_INPUT.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = at != 0 && now_ms().saturating_sub(at) < CLIENT_INPUT_FRESH_MS;
        (a.pushboolean)(l, fresh as c_int);
        (a.pushboolean)(l, (fresh && cursor) as c_int);
        2
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

/// gmsky.Anchor() -> x, y, z: the Garry's Mod point that matches Skyrim's origin, or nothing
/// before the first origin. Skyrim's ground grids are relative to it (protocol 7).
unsafe extern "C" fn l_anchor(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
        if link.epoch == 0 {
            return 0;
        }
        for v in link.anchor {
            (a.pushnumber)(l, v);
        }
        3
    })
}

/// gmsky.ReadActors(last_seq) -> seq, skyrim_frame, list (Phase 3 step 3b): Skyrim's actors near the
/// player, nearest first. Each entry: { id = formId, flags, x, y, z (feet, Garry's Mod world
/// units: the anchor is added here), yaw, radius, height, hp (0..1), level, name }.
/// Returns false if nothing new since last_seq; nil if there is no table for the current origin
/// (none yet, or written for another origin epoch: positions would be wrong).
unsafe extern "C" fn l_read_actors(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let last = (a.tonumber)(l, 1) as u32;
        let Some((t, seq)) = mapping().ok().and_then(|m| m.shared().actors.read()) else { return 0 };
        if seq == last {
            (a.pushboolean)(l, 0);
            return 1;
        }
        let (epoch, anchor) = {
            let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
            (link.epoch, link.anchor)
        };
        if epoch == 0 || t.origin_epoch != epoch {
            return 0;
        }
        (a.pushnumber)(l, seq as f64);
        (a.pushnumber)(l, t.skyrim_frame as f64);
        let n = (t.count as usize).min(gmsky_protocol::MAX_ACTORS);
        (a.createtable)(l, n as c_int, 0);
        for (i, r) in t.actors[..n].iter().enumerate() {
            (a.createtable)(l, 0, 12);
            let num = |key: &str, v: f64| {
                (a.pushnumber)(l, v);
                lua::set_field(a, l, -2, key);
            };
            num("id", r.form_id as f64);
            num("flags", r.flags as f64);
            num("x", anchor[0] + r.pos[0] as f64);
            num("y", anchor[1] + r.pos[1] as f64);
            num("z", anchor[2] + r.pos[2] as f64);
            num("yaw", r.yaw_deg as f64);
            num("radius", r.radius as f64);
            num("height", r.height as f64);
            num("hp", r.health_frac as f64);
            num("level", r.level as f64);
            let end = r.name.iter().position(|&c| c == 0).unwrap_or(r.name.len());
            lua::push_str(a, l, &String::from_utf8_lossy(&r.name[..end]));
            lua::set_field(a, l, -2, "name");
            (a.rawseti)(l, -2, i as c_int + 1);
        }
        3
    })
}

/// Phase 3 step 3c-2: the newest NPC hitboxes from Skyrim (hitboxes.rs), shared by both sides.
static HITBOXES: Mutex<Option<hitboxes::Store>> = Mutex::new(None);

/// Runs f on the hitbox store after taking Skyrim's newest table, with the anchor; None if there
/// is no table for the current origin.
fn with_hitboxes<R>(f: impl FnOnce(&hitboxes::Store, [f64; 3]) -> R) -> Option<R> {
    let (epoch, anchor) = {
        let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
        (link.epoch, link.anchor)
    };
    let mut guard = HITBOXES.lock().unwrap_or_else(|e| e.into_inner());
    let store = guard.get_or_insert_with(hitboxes::Store::default);
    if let Ok(m) = mapping() {
        let (slot, act) = (&m.shared().hitboxes, &m.shared().actors);
        if slot.seq() != store.seq || act.seq() != store.actors_seq {
            if let Some((t, seq)) = slot.read() {
                let tab = act.read();
                store.load(&t, seq, tab.as_ref().map(|(a, s)| (a, *s)));
            }
        }
    }
    if epoch == 0 || store.origin_epoch != epoch {
        return None;
    }
    Some(f(store, anchor))
}

/// gmsky.HitboxActors(last_seq) -> seq, age_ms, list: the actors whose hitboxes Skyrim sent, each
/// { id, flags, parts, lox, loy, loz, hix, hiy, hiz } (box around its parts, world units).
/// false if nothing new since last_seq; nil if there is no table for the current origin.
unsafe extern "C" fn l_hitbox_actors(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let last = (a.tonumber)(l, 1) as u32;
        let pushed = with_hitboxes(|st, anchor| {
            if st.seq == last {
                (a.pushboolean)(l, 0);
                return 1;
            }
            (a.pushnumber)(l, st.seq as f64);
            let age = if st.qpc > 0 { qpc_ms(now_qpc() - st.qpc) } else { -1.0 };
            (a.pushnumber)(l, age);
            (a.createtable)(l, st.actors.len() as c_int, 0);
            for (i, act) in st.actors.iter().enumerate() {
                (a.createtable)(l, 0, 9);
                let num = |key: &str, v: f64| {
                    (a.pushnumber)(l, v);
                    lua::set_field(a, l, -2, key);
                };
                num("id", act.id as f64);
                num("flags", act.flags as f64);
                num("parts", act.parts.len() as f64);
                num("lox", act.lo[0] + anchor[0]);
                num("loy", act.lo[1] + anchor[1]);
                num("loz", act.lo[2] + anchor[2]);
                num("hix", act.hi[0] + anchor[0]);
                num("hiy", act.hi[1] + anchor[1]);
                num("hiz", act.hi[2] + anchor[2]);
                (a.rawseti)(l, -2, i as c_int + 1);
            }
            3
        });
        pushed.unwrap_or(0)
    })
}

/// gmsky.HitTrace(id, sx, sy, sz, dx, dy, dz, ex, ey, ez [, gear]) -> fraction, nx, ny, nz, group,
/// bone: where a trace from s (world units) along d first meets actor id's hitboxes. e = half size
/// of a moving box, s its centre (0, 0, 0 = a line: tested against the body parts; a box: against
/// the actor's fixed hull, none for corpses). gear = also hit gear (a carried bow). Nothing if it
/// misses.
unsafe extern "C" fn l_hit_trace(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let id = n(1) as u32;
        let gear = (a.toboolean)(l, 11) != 0;
        let hit = with_hitboxes(|st, anchor| {
            let s = [n(2) - anchor[0], n(3) - anchor[1], n(4) - anchor[2]];
            st.actor(id).and_then(|act| act.trace(s, [n(5), n(6), n(7)], [n(8), n(9), n(10)], gear))
        })
        .flatten();
        match hit {
            Some((f, nv, group, bone)) => {
                (a.pushnumber)(l, f);
                for v in nv {
                    (a.pushnumber)(l, v);
                }
                (a.pushnumber)(l, group as f64);
                (a.pushnumber)(l, bone as f64);
                6
            }
            None => 0,
        }
    })
}

/// gmsky.HitPartAt(id, x, y, z) -> group, bone, distance: actor id's body part nearest to a point
/// (world units), e.g. where damage landed. Nothing if there is no such actor.
unsafe extern "C" fn l_hit_part_at(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let id = n(1) as u32;
        let found = with_hitboxes(|st, anchor| {
            st.actor(id).and_then(|act| act.part_at([n(2) - anchor[0], n(3) - anchor[1], n(4) - anchor[2]]))
        })
        .flatten();
        match found {
            Some((group, bone, d)) => {
                (a.pushnumber)(l, group as f64);
                (a.pushnumber)(l, bone as f64);
                (a.pushnumber)(l, d);
                3
            }
            None => 0,
        }
    })
}

/// gmsky.HitPushOut(x, y, z, ext, height) -> dx, dy, depth, id: the player's box (feet at x, y, z,
/// half width ext) inside a living NPC's hull: the horizontal way out of the deepest overlap and
/// how deep it is. Nothing if it overlaps none.
unsafe extern "C" fn l_hit_push_out(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let found = with_hitboxes(|st, anchor| st.push_out([n(1) - anchor[0], n(2) - anchor[1], n(3) - anchor[2]], n(4), n(5))).flatten();
        match found {
            Some((dir, depth, id)) => {
                (a.pushnumber)(l, dir[0]);
                (a.pushnumber)(l, dir[1]);
                (a.pushnumber)(l, depth);
                (a.pushnumber)(l, id as f64);
                4
            }
            None => 0,
        }
    })
}

/// Phase 3 step 3c-3: a hitbox test waiting for Skyrim's answer.
struct PendingTest {
    id: u32,
    form_id: u32,
    plan: hitboxes::TestPlan,
    started_ms: u64,
    table_age_ms: f64,
}
static HIT_TEST: Mutex<Option<PendingTest>> = Mutex::new(None);

/// gmsky.HitTestStart(id, fx, fy, fz) -> test id, or false and why: lays a 21 Ã— 21 grid of rays along
/// the view (f) over actor id, answers them from Garry's Mod's copy and asks Skyrim (server only).
unsafe extern "C" fn l_hit_test_start(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let id = n(1) as u32;
        let planned = with_hitboxes(|st, _| {
            let age = if st.qpc > 0 { qpc_ms(now_qpc() - st.qpc) } else { -1.0 };
            (st.actor(id).and_then(|act| hitboxes::plan_test(act, [n(2), n(3), n(4)])), st.origin_epoch, age)
        });
        let why = match (planned, writable_mapping()) {
            (Some((Some(plan), epoch, age)), Some(m)) => {
                let mut req = Box::new(gmsky_protocol::HitTestRequest::default());
                // Never 0, and different after a Garry's Mod restart (Skyrim answers each id once).
                req.id = (now_ms() as u32) | 1;
                req.form_id = id;
                req.origin_epoch = epoch;
                req.count = plan.rays.len().min(gmsky_protocol::HT_MAX_RAYS) as u32;
                for (i, (o, d)) in plan.rays.iter().take(req.count as usize).enumerate() {
                    req.rays[i] = gmsky_protocol::HitTestRay { o: [o[0] as f32, o[1] as f32, o[2] as f32], d: [d[0] as f32, d[1] as f32, d[2] as f32] };
                }
                m.shared().hit_test_request.write(&req);
                let test_id = req.id;
                *HIT_TEST.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(PendingTest { id: test_id, form_id: id, plan, started_ms: now_ms(), table_age_ms: age });
                (a.pushnumber)(l, test_id as f64);
                return 1;
            }
            (Some((None, _, _)), _) => "that NPC has no hitboxes in the newest table",
            (None, _) => "no hitbox table for the current origin",
            (_, None) => "this process doesn't write the Garry's Mod side",
        };
        (a.pushboolean)(l, 0);
        lua::push_str(a, l, why);
        2
    })
}

/// gmsky.HitTestReport() -> text: the comparison once Skyrim has answered the pending test (or
/// "no answer" after 2 s); nothing while waiting or when no test is pending.
unsafe extern "C" fn l_hit_test_report(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let mut pending = HIT_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let Some(p) = pending.as_ref() else { return 0 };
        let res = mapping().ok().and_then(|m| m.shared().hit_test_result.read()).map(|(r, _)| r);
        let text = match res {
            Some(r) if r.id == p.id => format!("hitbox test {:08X}: {}; Garry's Mod's table was {:.1} ms old; answered after {} ms",
                p.form_id, hitboxes::report(&p.plan, &r), p.table_age_ms, now_ms().saturating_sub(p.started_ms)),
            _ if now_ms().saturating_sub(p.started_ms) > 2000 => format!("hitbox test {:08X}: no answer from Skyrim in 2 s", p.form_id),
            _ => return 0,
        };
        *pending = None;
        lua::push_str(a, l, &text);
        1
    })
}

/// gmsky.SendHit(id, damage, gmod_damage, x, y, z, dx, dy, dz, force, dmg_type, group, bone, kind)
/// -> events sent so far, or nothing: a hit on Skyrim's NPC id for Skyrim (Phase 3 step 3d).
/// x, y, z: where it landed (world units); d: its direction. Server side; nothing before an origin.
unsafe extern "C" fn l_send_hit(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let (epoch, anchor) = {
            let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
            (link.epoch, link.anchor)
        };
        let Some(m) = writable_mapping() else { return 0 };
        if epoch == 0 {
            return 0;
        }
        let e = gmsky_protocol::HitEvent {
            form_id: n(1) as u32,
            damage: n(2) as f32,
            gmod_damage: n(3) as f32,
            pos: [(n(4) - anchor[0]) as f32, (n(5) - anchor[1]) as f32, (n(6) - anchor[2]) as f32],
            dir: [n(7) as f32, n(8) as f32, n(9) as f32],
            force: n(10) as f32,
            dmg_type: n(11) as u32,
            origin_epoch: epoch,
            group: n(12) as u8,
            bone: n(13) as u8,
            kind: n(14) as u8,
            flags: 0,
        };
        let head = m.shared().hit_events.push(&e);
        (a.pushnumber)(l, head as f64);
        1
    })
}

/// Phase 3 step 3e: the server's read position in Skyrim's hurt ring (None = not read yet).
static HURT_TAIL: Mutex<Option<u32>> = Mutex::new(None);

/// gmsky.ReadHurts() -> list, lost: the hurt events Skyrim wrote since the last call (Skyrim's NPCs
/// hurting the player), oldest first: { attacker, damage, source, kind, flags }; lost = events
/// that were overwritten before this read. Nothing without a link. The first call only takes
/// Skyrim's current position (old hurts never land). Server side only (one read position).
unsafe extern "C" fn l_read_hurts(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let Ok(m) = mapping() else { return 0 };
        let ring = &m.shared().hurt_events;
        let head = ring.head.load(std::sync::atomic::Ordering::Acquire);
        let mut tail = HURT_TAIL.lock().unwrap_or_else(|e| e.into_inner());
        let mut from = match *tail {
            Some(t) if t <= head => t,
            _ => head, // first read, or a new block (head went back)
        };
        let mut lost = 0u32;
        if head - from > gmsky_protocol::HURT_MAX as u32 {
            lost = head - from - gmsky_protocol::HURT_MAX as u32;
            from = head - gmsky_protocol::HURT_MAX as u32;
        }
        *tail = Some(head);
        (a.createtable)(l, (head - from) as c_int, 0);
        for (k, i) in (from..head).enumerate() {
            let e = ring.get(i);
            (a.createtable)(l, 0, 5);
            let num = |key: &str, v: f64| {
                (a.pushnumber)(l, v);
                lua::set_field(a, l, -2, key);
            };
            num("attacker", e.attacker as f64);
            num("damage", e.damage as f64);
            num("source", e.source as f64);
            num("kind", e.kind as f64);
            num("flags", e.flags as f64);
            (a.rawseti)(l, -2, k as c_int + 1);
        }
        (a.pushnumber)(l, lost as f64);
        2
    })
}

/// gmsky.SkyrimFlags() -> flags, frame: Skyrim's GMSKY_SF_* bits (1 = someone fights the player,
/// step 3f) and its frame; nothing before Skyrim has written its state.
unsafe extern "C" fn l_skyrim_flags(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let Some((k, _)) = mapping().ok().and_then(|m| m.shared().skyrim_state.read()) else { return 0 };
        (a.pushnumber)(l, k.flags as f64);
        (a.pushnumber)(l, k.frame as f64);
        2
    })
}

/// QueryPerformanceCounter ticks to milliseconds.
fn qpc_ms(ticks: i64) -> f64 {
    #[link(name = "kernel32")]
    extern "system" {
        fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
    }
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f) };
    if f > 0 { ticks as f64 * 1000.0 / f as f64 } else { -1.0 }
}

/// gmsky.ShiftAnchor(dx, dy, dz) -> x, y, z: re-centre (DESIGN.md re-plan step C). Lua moves the
/// player by the same amount in the same tick, so Skyrim's position of the player doesn't change.
/// Returns nothing (and shifts nothing) before the first origin.
unsafe extern "C" fn l_shift_anchor(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let d = [(a.tonumber)(l, 1), (a.tonumber)(l, 2), (a.tonumber)(l, 3)];
        let mut link = LINK.lock().unwrap_or_else(|e| e.into_inner());
        if link.epoch == 0 {
            return 0;
        }
        let before = link.anchor;
        for c in 0..3 {
            link.anchor[c] += d[c];
        }
        log(side, &format!("anchor shifted by ({:.1}, {:.1}, {:.1}): ({:.1}, {:.1}, {:.1}) -> ({:.1}, {:.1}, {:.1}), origin epoch {} unchanged",
            d[0], d[1], d[2], before[0], before[1], before[2], link.anchor[0], link.anchor[1], link.anchor[2], link.epoch));
        for v in link.anchor {
            (a.pushnumber)(l, v);
        }
        3
    })
}

// ---- Skyrim's collision shapes (DESIGN.md re-plan step D; shapes.rs) ----

/// One store for both sides: the server and client Lua share this module in one process.
static SHAPES: Mutex<Option<shapes::Store>> = Mutex::new(None);

fn with_shapes<R>(f: impl FnOnce(&mut shapes::Store) -> R) -> R {
    let mut g = SHAPES.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(shapes::Store::default))
}

/// The origin epoch Garry's Mod is anchored to (shapes are only used for that one).
fn anchored_epoch() -> u32 {
    LINK.lock().unwrap_or_else(|e| e.into_inner()).epoch
}

/// gmsky.PollShapes([x, y, z]) -> messages, bytes: reads what Skyrim put in the collision ring
/// (up to 8 MB per call) into the store; with the player's position (relative to the anchor),
/// drops regions far from it. Only the process that writes the Garry's Mod side reads the ring.
unsafe extern "C" fn l_poll_shapes(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let Some(m) = writable_mapping() else { return 0 };
        let mut notes = Vec::new();
        let (count, bytes) = with_shapes(|st| {
            let mut count = 0u32;
            let r = m.ring_read(8 << 20, |kind, p| {
                count += 1;
                if let Some(note) = st.apply(kind, p) {
                    notes.push(note);
                }
            });
            let bytes = match r {
                Ok(b) => b,
                Err(e) => {
                    notes.push(format!("ERROR {e}"));
                    0
                }
            };
            if (a.type_)(l, 1) == 3 {
                st.evict([(a.tonumber)(l, 1), (a.tonumber)(l, 2), (a.tonumber)(l, 3)]);
            }
            (count, bytes)
        });
        for n in notes {
            log(side, &n);
        }
        (a.pushnumber)(l, count as f64);
        (a.pushnumber)(l, bytes as f64);
        2
    })
}

/// gmsky.Sweep(cx, cy, cz, ex, ey, ez, dx, dy, dz) -> fraction, nx, ny, nz, or nothing:
/// where a box (centre c, half size e, relative to the anchor) moving by d first touches
/// Skyrim's triangles. Same rules as field.lua's F.Sweep.
unsafe extern "C" fn l_sweep(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let epoch = anchored_epoch();
        let hit = with_shapes(|st| {
            if epoch == 0 || st.origin_epoch != epoch {
                return None;
            }
            st.sweep([n(1), n(2), n(3)], [n(4), n(5), n(6)], [n(7), n(8), n(9)])
        });
        match hit {
            Some((f, nv)) => {
                (a.pushnumber)(l, f);
                for v in nv {
                    (a.pushnumber)(l, v);
                }
                4
            }
            None => 0,
        }
    })
}

/// gmsky.TrisInBox(lx, ly, lz, hx, hy, hz, max [, clip]) -> list, count, cut, thin, shortest,
/// smallestAngle: Skyrim's triangles touching the box (relative to the anchor) as a flat list of
/// numbers, 9 per triangle (3 corners x, y, z), at most `max` triangles, duplicates removed. With
/// clip, every triangle is cut exactly at the box's faces (`cut` = how many were cut). The last
/// three describe the returned triangles' shape (shapes::mesh_quality). Empty when not anchored
/// to the store's epoch.
unsafe extern "C" fn l_tris_in_box(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let max = n(7).max(0.0) as usize;
        let clip = (a.toboolean)(l, 8) != 0;
        let epoch = anchored_epoch();
        let (tris, cut) = with_shapes(|st| {
            if epoch == 0 || st.origin_epoch != epoch {
                return (Vec::new(), 0);
            }
            st.tris_in_box([n(1), n(2), n(3)], [n(4), n(5), n(6)], max, clip)
        });
        let (thin, shortest, smallest) = shapes::mesh_quality(&tris);
        (a.createtable)(l, (tris.len() * 9) as c_int, 0);
        let mut i = 1;
        for t in &tris {
            for v in t {
                (a.pushnumber)(l, *v);
                (a.rawseti)(l, -2, i);
                i += 1;
            }
        }
        let shortest = if tris.is_empty() { 0.0 } else { shortest };
        for v in [tris.len() as f64, cut as f64, thin as f64, shortest, smallest] {
            (a.pushnumber)(l, v);
        }
        6
    })
}

/// gmsky.RampsInBox(lx, ly, lz, hx, hy, hz [, id]) -> flat, count, fingerprint, noses,
/// strips, bottoms, bottom ends in the air, tops, tops not flush, steepest: S2i-2's ramps over
/// Skyrim's stairs for a chunk's box (relative to the anchor; shapes::Store::ramps_in_box). `flat`
/// holds 9 numbers per triangle; the fingerprint is a hex string. With `id` (S2i-4: the ramp
/// entity's number), this side also keeps the triangles for gmsky.RampSweep. Nothing when not
/// anchored to the store's epoch.
unsafe extern "C" fn l_ramps_in_box(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let epoch = anchored_epoch();
        let Some(r) = with_shapes(|st| (epoch != 0 && st.origin_epoch == epoch).then(|| st.ramps_in_box([n(1), n(2), n(3)], [n(4), n(5), n(6)]))) else {
            return 0;
        };
        if (a.type_)(l, 7) == 3 {
            with_ramps(|k| k.keep(side, (a.tonumber)(l, 7) as i32, epoch, &r.tris));
        }
        (a.createtable)(l, (r.tris.len() * 9) as c_int, 0);
        let mut i = 1;
        for t in &r.tris {
            for v in t {
                (a.pushnumber)(l, *v);
                (a.rawseti)(l, -2, i);
                i += 1;
            }
        }
        (a.pushnumber)(l, r.tris.len() as f64);
        lua::push_str(a, l, &format!("{:016x}", r.hash));
        for v in [r.noses as f64, r.strips as f64, r.bottoms as f64, r.bottoms_floating as f64, r.tops as f64, r.tops_off as f64, r.steepest] {
            (a.pushnumber)(l, v);
        }
        10
    })
}

static RAMPS: Mutex<Option<shapes::RampSets>> = Mutex::new(None);

fn with_ramps<R>(f: impl FnOnce(&mut shapes::RampSets) -> R) -> R {
    let mut g = RAMPS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(shapes::RampSets::default))
}

/// gmsky.RampSweep(id, bx, by, bz, ex, ey, ez, dx, dy, dz) -> fraction, nx, ny, nz, or nothing:
/// where a box (centre b, half size e, relative to the anchor) moving by d first touches this
/// side's kept ramp `id` (S2i-4; same rules as gmsky.Sweep). Nothing for a ramp kept in another
/// anchor epoch.
unsafe extern "C" fn l_ramp_sweep(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let epoch = anchored_epoch();
        let hit = with_ramps(|k| k.sweep(side, n(1) as i32, epoch, [n(2), n(3), n(4)], [n(5), n(6), n(7)], [n(8), n(9), n(10)]));
        match hit {
            Some((f, nv)) => {
                (a.pushnumber)(l, f);
                for v in nv {
                    (a.pushnumber)(l, v);
                }
                4
            }
            None => 0,
        }
    })
}

/// gmsky.RampForget(id): this side drops its kept ramp `id` (S2i-4).
unsafe extern "C" fn l_ramp_forget(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let id = (a.tonumber)(l, 1) as i32;
        with_ramps(|k| k.forget(side, id));
        0
    })
}

/// gmsky.DumpTris(lx, ly, lz, hx, hy, hz, name) -> count, path: every stored triangle touching the
/// box written to logs/tris/<name>.txt (shapes::Store::dump). The first dump of a session empties
/// logs/tris/ (earlier runs' dumps are in their saved log folders). Nothing when not anchored.
unsafe extern "C" fn l_dump_tris(l: *mut State) -> c_int {
    static CLEARED: std::sync::Once = std::sync::Once::new();
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let name: String = lua::to_string(a, l, 7).unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
        let epoch = anchored_epoch();
        let Some((count, text)) = with_shapes(|st| (epoch != 0 && st.origin_epoch == epoch).then(|| st.dump([n(1), n(2), n(3)], [n(4), n(5), n(6)]))) else {
            return 0;
        };
        let dir = log_dir().join("tris");
        CLEARED.call_once(|| {
            let _ = std::fs::remove_dir_all(&dir);
        });
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.txt", if name.is_empty() { "dump".into() } else { name }));
        let ok = std::fs::write(&path, text).is_ok();
        (a.pushnumber)(l, count as f64);
        lua::push_str(a, l, &format!("{}{}", path.display(), if ok { "" } else { " (WRITE FAILED)" }));
        2
    })
}

/// gmsky.RampCheck(lx, ly, lz, hx, hy, hz) -> line: why the stair triangles in the box did or
/// didn't make ramps (shapes::Store::ramp_check; for the STUCK lines). Nothing when not anchored.
unsafe extern "C" fn l_ramp_check(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let epoch = anchored_epoch();
        match with_shapes(|st| (epoch != 0 && st.origin_epoch == epoch).then(|| st.ramp_check([n(1), n(2), n(3)], [n(4), n(5), n(6)]))) {
            Some(line) => {
                lua::push_str(a, l, &line);
                1
            }
            None => 0,
        }
    })
}

/// gmsky.TrisInfo(lx, ly, lz, hx, hy, hz, max) -> list, count: Skyrim's triangles touching the box
/// (relative to the anchor), 6 numbers each: lowest z, highest z, normal x, y, z, stair ramp (1/0).
/// Empty when not anchored to the store's epoch. For the stuck detector (S2e).
unsafe extern "C" fn l_tris_info(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i);
        let max = n(7).max(0.0) as usize;
        let epoch = anchored_epoch();
        let tris = with_shapes(|st| {
            if epoch == 0 || st.origin_epoch != epoch {
                return Vec::new();
            }
            st.tris_info([n(1), n(2), n(3)], [n(4), n(5), n(6)], max)
        });
        (a.createtable)(l, (tris.len() * 6) as c_int, 0);
        let mut i = 1;
        for (lo, hi, nv, flags) in &tris {
            for v in [*lo, *hi, nv[0], nv[1], nv[2], *flags as f64] {
                (a.pushnumber)(l, v);
                (a.rawseti)(l, -2, i);
                i += 1;
            }
        }
        (a.pushnumber)(l, tris.len() as f64);
        2
    })
}

/// gmsky.ShapesLattice() -> bx, by, bz, size: where Skyrim's region (0, 0, 0) starts (relative to
/// the anchor) and the region size, in Garry's Mod units; nothing before the first region arrived
/// or when not anchored to the store's epoch.
unsafe extern "C" fn l_shapes_lattice(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let epoch = anchored_epoch();
        match with_shapes(|st| if epoch != 0 && st.origin_epoch == epoch { st.lattice() } else { None }) {
            Some((base, size)) => {
                for v in [base[0], base[1], base[2], size] {
                    (a.pushnumber)(l, v);
                }
                4
            }
            None => 0,
        }
    })
}

/// gmsky.ShapesRegionsArrived(rx0, ry0, rz0, rx1, ry1, rz1) -> arrived, total: how many of the
/// regions in that inclusive range (Skyrim's region numbers) have arrived from Skyrim.
/// gmsky.ShapesChunkVersion(rx0, ry0, rz0, rx1, ry1, rz1) -> arrived, total, fingerprint ("hex")
/// of the regions in that range (S2: a physics chunk is rebuilt only when the fingerprint changes).
/// Nothing has arrived (fingerprint "-") while not anchored to the store's origin.
unsafe extern "C" fn l_shapes_chunk_version(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i).floor() as i32;
        let epoch = anchored_epoch();
        let (arrived, total, hash) = with_shapes(|st| {
            let r = st.chunk_version([n(1), n(2), n(3)], [n(4), n(5), n(6)]);
            if epoch != 0 && st.origin_epoch == epoch { (r.0, r.1, Some(r.2)) } else { (0, r.1, None) }
        });
        (a.pushnumber)(l, arrived as f64);
        (a.pushnumber)(l, total as f64);
        lua::push_str(a, l, &hash.map_or("-".to_string(), |h| format!("{h:016x}")));
        3
    })
}

unsafe extern "C" fn l_shapes_regions_arrived(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let n = |i: c_int| (a.tonumber)(l, i).floor() as i32;
        let epoch = anchored_epoch();
        let (arrived, total) = with_shapes(|st| {
            let r = st.regions_arrived([n(1), n(2), n(3)], [n(4), n(5), n(6)]);
            if epoch != 0 && st.origin_epoch == epoch { r } else { (0, r.1) }
        });
        (a.pushnumber)(l, arrived as f64);
        (a.pushnumber)(l, total as f64);
        2
    })
}

/// gmsky.ShapesKnown(x, y, z) -> true if Skyrim's collision at that point (relative to the
/// anchor) and just below it has arrived.
unsafe extern "C" fn l_shapes_known(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let epoch = anchored_epoch();
        let p = [(a.tonumber)(l, 1), (a.tonumber)(l, 2), (a.tonumber)(l, 3)];
        let known = with_shapes(|st| epoch != 0 && st.origin_epoch == epoch && st.known(p));
        (a.pushboolean)(l, known as c_int);
        1
    })
}

/// gmsky.ShapesBounds() -> lo x, y, z, hi x, y, z around every stored region (relative to the
/// anchor), or nothing.
unsafe extern "C" fn l_shapes_bounds(l: *mut State) -> c_int {
    guarded(l, |a, _| match with_shapes(|st| st.bounds()) {
        Some((lo, hi)) => {
            for v in lo.into_iter().chain(hi) {
                (a.pushnumber)(l, v);
            }
            6
        }
        None => 0,
    })
}

/// gmsky.ShapesStatus(reset[, x, y, z]) -> regions, triangles, line: a status line for the log
/// (since the last reset). With the player's position: what is stored where it is.
unsafe extern "C" fn l_shapes_status(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let reset = (a.toboolean)(l, 1) != 0;
        let at = ((a.type_)(l, 2) == 3).then(|| [(a.tonumber)(l, 2), (a.tonumber)(l, 3), (a.tonumber)(l, 4)]);
        let pending = mapping().map(|m| m.ring_pending()).unwrap_or(0);
        let (regions, tris, line) = with_shapes(|st| {
            let s = st.stats;
            let here = match at {
                Some(p) => match st.tris_at(p) {
                    Some(n) => format!(", {} triangles in the player's region, collision here {}", n,
                                       if st.known(p) { "arrived" } else { "NOT arrived (region below missing)" }),
                    None => ", the player's region has NOT arrived".into(),
                },
                None => String::new(),
            };
            let line = format!("shapes: {} regions, {} triangles ({} stair helper, {} stair material) stored, collision epoch {} origin epoch {}{} | in: {} messages, {} regions, {} triangles, {:.0} KB, {} ignored, {} evicted, ring {:.0} KB behind | sweeps {} hits {} triangles tested {} Rust {:.2} ms (slowest {:.0} us)",
                st.regions(), st.tris(), st.stair_helpers(), st.stairs(), st.col_epoch, st.origin_epoch, here,
                s.messages, s.regions_in, s.tris_in, s.bytes as f64 / 1024.0, s.ignored, s.evicted, pending as f64 / 1024.0,
                s.sweeps, s.hits, s.tested, s.us / 1000.0, s.worst_us);
            if reset {
                st.stats = shapes::Stats::default();
            }
            (st.regions(), st.tris(), line)
        });
        (a.pushnumber)(l, regions as f64);
        (a.pushnumber)(l, tris as f64);
        lua::push_str(a, l, &line);
        3
    })
}

// ---- Phase 2a: the overlay experiment (DESIGN.md "Phase 2 steps") ----

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32EnumProcessModules(process: isize, modules: *mut isize, cb: u32, needed: *mut u32) -> i32;
    fn K32GetModuleFileNameExW(process: isize, module: isize, name: *mut u16, size: u32) -> u32;
}

/// Full paths of every DLL loaded in this process.
fn loaded_modules() -> Vec<String> {
    unsafe {
        let me = GetCurrentProcess();
        let mut mods = vec![0isize; 1024];
        let mut needed = 0u32;
        if K32EnumProcessModules(me, mods.as_mut_ptr(), (mods.len() * 8) as u32, &mut needed) == 0 {
            return Vec::new();
        }
        mods.truncate((needed as usize / 8).min(mods.len()));
        mods.iter()
            .filter_map(|&m| {
                let mut buf = [0u16; 520];
                let n = K32GetModuleFileNameExW(me, m, buf.as_mut_ptr(), buf.len() as u32);
                (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
            })
            .collect()
    }
}

/// gmsky.GraphicsInfo() -> line: which Direct3D 9 this Garry's Mod draws with: d3d9.dll's path and
/// size, and whether a translation layer (DXVK: vulkan-1.dll / dxgi.dll next to d3d9) or other
/// graphics DLLs are loaded. Read only; nothing is hooked.
unsafe extern "C" fn l_graphics_info(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let mods = loaded_modules();
        let interesting = ["d3d9", "d3d11", "dxgi", "vulkan-1", "dxvk", "shaderapi", "materialsystem", "opengl32", "nvapi", "amdxx", "igd", "renderdoc", "reshade"];
        let mut lines: Vec<String> = mods.iter()
            .filter(|m| {
                let name = m.rsplit('\\').next().unwrap_or(m).to_ascii_lowercase();
                interesting.iter().any(|k| name.contains(k))
            })
            .map(|m| {
                let size = std::fs::metadata(m).map(|md| md.len()).unwrap_or(0);
                format!("{m} ({size} bytes)")
            })
            .collect();
        lines.sort();
        let vulkan = mods.iter().any(|m| m.to_ascii_lowercase().ends_with("vulkan-1.dll"));
        let line = format!("graphics: {} modules loaded; Vulkan {} (DXVK would load it); graphics DLLs: {}",
            mods.len(), if vulkan { "LOADED" } else { "not loaded" }, lines.join("; "));
        lua::push_str(a, l, &line);
        1
    })
}

/// gmsky.WriteLogFile(name, data) -> true | false, error: saves binary data (a test picture) to
/// logs/overlay/<name>. Only plain file names (no folders, no ".."), never into the game folder.
unsafe extern "C" fn l_write_log_file(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let name = lua::to_string(a, l, 1).unwrap_or_default();
        let ok_name = !name.is_empty() && name.len() < 64
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            && !name.contains("..");
        let fail = |msg: String| {
            (a.pushboolean)(l, 0);
            lua::push_str(a, l, &msg);
            2
        };
        if !ok_name {
            return fail(format!("bad file name {name:?}"));
        }
        let Some(data) = lua::to_bytes(a, l, 2) else { return fail("no data".into()) };
        let dir = log_dir().join("overlay");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return fail(format!("create {}: {e}", dir.display()));
        }
        match std::fs::write(dir.join(&name), &data) {
            Ok(()) => {
                (a.pushboolean)(l, 1);
                1
            }
            Err(e) => fail(format!("write {name}: {e}")),
        }
    })
}

/// gmsky.D3DState() -> line: Garry's Mod's Direct3D 9 blending settings right now (alpha writes,
/// blend rules, render target format), for the 2D alpha test. Read only (d3d.rs). Only meaningful
/// with mat_queue_mode 0 (Source drawing on the Lua thread); overlay2d.lua checks that.
unsafe extern "C" fn l_d3d_state(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &d3d::state());
        1
    })
}

/// gmsky.OverlayHook() -> line for the log. Step 2b-1: finds Garry's Mod's Direct3D 9 device and
/// hooks its Present (present.rs). Safe to call again (does nothing once hooked).
unsafe extern "C" fn l_overlay_hook(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &present::install());
        1
    })
}

/// gmsky.OverlayStatus() -> nil if not hooked, otherwise the Present hook's numbers since the last call.
unsafe extern "C" fn l_overlay_status(l: *mut State) -> c_int {
    guarded(l, |a, _| match present::status() {
        Some(s) => { lua::push_str(a, l, &s); 1 }
        None => 0,
    })
}

/// gmsky.OverlayEnable(on): step 2b-3, start / stop copying each prepared frame into the overlay
/// picture block (capture.rs; needs gmsky.OverlayHook first).
unsafe extern "C" fn l_overlay_enable(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        capture::enable((a.toboolean)(l, 1) != 0);
        0
    })
}

/// gmsky.OverlayPrepared(): Lua prepared this frame for Skyrim (cleared behind the view model,
/// alpha forced); the next Present copies it.
unsafe extern "C" fn l_overlay_prepared(l: *mut State) -> c_int {
    guarded(l, |_, _| {
        capture::mark_prepared();
        0
    })
}

/// gmsky.OverlayBlur(on): Skyrim blurs its picture behind see-through parts of ours (flag in each
/// picture's header).
unsafe extern "C" fn l_overlay_blur(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        capture::set_blur((a.toboolean)(l, 1) != 0);
        0
    })
}

/// gmsky.OverlayWindow(off_screen) -> line for the log: step 2d-2, moves Garry's Mod's window just past
/// the screens (true) or back where it was (false) (window.rs).
unsafe extern "C" fn l_overlay_window(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &window::set_off_screen((a.toboolean)(l, 1) != 0));
        1
    })
}

/// gmsky.ConsoleWindow(front) -> line for the log: Garry's Mod's window over Skyrim's and in front
/// (true), or back off-screen with Skyrim in front (false); for Garry's Mod's console (F1).
unsafe extern "C" fn l_console_window(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let pid = mapping().map(|m| m.shared().header.skyrim_pid.load(std::sync::atomic::Ordering::Acquire)).unwrap_or(0);
        lua::push_str(a, l, &window::console_front((a.toboolean)(l, 1) != 0, pid));
        1
    })
}

/// gmsky.SkyrimExited() -> pid: once, after the Skyrim process seen earlier has ended; nothing
/// otherwise (window::skyrim_exited).
unsafe extern "C" fn l_skyrim_exited(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let pid = mapping().map(|m| m.shared().header.skyrim_pid.load(std::sync::atomic::Ordering::Acquire)).unwrap_or(0);
        match window::skyrim_exited(pid) {
            Some(gone) => {
                (a.pushnumber)(l, gone as f64);
                1
            }
            None => 0,
        }
    })
}

/// gmsky.CloseGame() -> line for the log: asks Garry's Mod's window to close (window::close_game).
unsafe extern "C" fn l_close_game(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &window::close_game());
        1
    })
}

/// gmsky.EndProcess(): ends Garry's Mod's process at once (window::end_process). The log line
/// is written by Lua before.
unsafe extern "C" fn l_end_process(l: *mut State) -> c_int {
    guarded(l, |_, _| {
        window::end_process();
        0
    })
}

/// gmsky.WaterZ() -> z: the water surface where Skyrim's player is, in Garry's Mod's world units
/// (step 4b); nothing when there is no water, or Skyrim's origin isn't the one we're anchored to.
unsafe extern "C" fn l_water_z(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        let Ok(m) = mapping() else { return 0 };
        let (epoch, anchor) = {
            let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
            (link.epoch, link.anchor)
        };
        let origin = m.shared().origin.read().map(|(o, _)| o.epoch).unwrap_or(0);
        let Some((k, _)) = m.shared().skyrim_state.read() else { return 0 };
        if epoch == 0 || origin != epoch || k.flags & gmsky_protocol::SF_WATER == 0 {
            return 0;
        }
        (a.pushnumber)(l, anchor[2] + k.water_z as f64);
        1
    })
}

/// gmsky.ConsoleKey() -> line for the log: presses Garry's Mod's console key (~) in its own window.
unsafe extern "C" fn l_console_key(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &window::press_console_key());
        1
    })
}

/// gmsky.OverlayShot() -> line for the log: asks Skyrim to save its next drawn frame (step 2c-1).
unsafe extern "C" fn l_overlay_shot(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &capture::request_shot());
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
///                        deg_per_count, tick_ms, punch_pitch, punch_yaw, punch_roll)
///                        -> origin epoch in use (0 = no origin yet)
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
            let all = INPUT.lock().unwrap_or_else(|e| e.into_inner());
            let t = &all[SIDE_SERVER as usize];
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
            punch_deg: [n(16) as f32, n(17) as f32, n(18) as f32],
            reserved2: 0,
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

// ---- Geometry route step G3b (DESIGN.md Â§12): Garry's Mod's objects to Skyrim (render_out.rs) ----

static RENDER_OUT: Mutex<Option<render_out::RenderOut>> = Mutex::new(None);
static RENDER_OPEN_TRY: Mutex<u64> = Mutex::new(0);

/// Runs `f` with the render writer, opening the render block first if needed (every 2 s until it
/// works). Err with why if it isn't open.
fn with_render_out<R>(side: u32, f: impl FnOnce(&mut render_out::RenderOut) -> R) -> Result<R, String> {
    let mut guard = RENDER_OUT.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let mut last = RENDER_OPEN_TRY.lock().unwrap_or_else(|e| e.into_inner());
        if now_ms() - *last < 2000 {
            return Err("render block not open (retrying every 2 s)".into());
        }
        *last = now_ms();
        match gmsky_protocol::render::RenderMapping::open() {
            Ok(m) => {
                log(side, &format!("render block open (session {})", std::process::id()));
                *guard = Some(render_out::RenderOut::new(m, std::process::id()));
            }
            Err(e) => {
                log(side, &format!("ERROR opening the render block: {e}"));
                return Err(e);
            }
        }
    }
    Ok(f(guard.as_mut().unwrap()))
}

/// Pushes (true, line) or (false, why).
unsafe fn push_result(a: &Api, l: *mut State, r: Result<String, String>) -> c_int {
    let (ok, line) = match r { Ok(s) => (true, s), Err(e) => (false, e) };
    (a.pushboolean)(l, ok as c_int);
    lua::push_str(a, l, &line);
    2
}

/// gmsky.RenderTexture(id, vtf_bytes [, linear]) -> ok, line: decodes a .vtf and sends it to
/// Skyrim; `linear` true for a normal map (not a colour).
unsafe extern "C" fn l_render_texture(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let id = (a.tonumber)(l, 1) as u32;
        let r = match lua::to_bytes(a, l, 2) {
            Some(bytes) => {
                let linear = (a.toboolean)(l, 3) != 0;
                with_render_out(side, |o| o.add_texture(id, &bytes, linear)).and_then(|r| r)
            }
            None => Err(format!("texture {id}: no file data")),
        };
        push_result(a, l, r)
    })
}

/// gmsky.RenderModel(id, bone_count, vertices, parts, bind_poses) -> ok, line: flat number tables
/// (20 per corner, 19 per part, 12 per bone; render_out.rs).
unsafe extern "C" fn l_render_model(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let (id, bones) = ((a.tonumber)(l, 1) as u32, (a.tonumber)(l, 2) as u32);
        let r = match (lua::to_numbers(a, l, 3), lua::to_numbers(a, l, 4), lua::to_numbers(a, l, 5)) {
            (Some(v), Some(p), Some(b)) => with_render_out(side, |o| o.add_model(id, bones, &v, &p, &b)).and_then(|r| r),
            _ => Err(format!("model {id}: vertices, parts and bind poses must be tables")),
        };
        push_result(a, l, r)
    })
}

/// gmsky.RenderFrame(frame_number, flat [, eye_x, eye_y, eye_z, pitch, yaw, roll [, switches]]) -> objects
/// sent: per object model id, colour, bone count, entity, flags, 12 numbers per bone (world); the
/// optional view is Garry's Mod's camera for this frame. Uses the anchor and origin epoch this
/// module holds.
unsafe extern "C" fn l_render_frame(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        const LUA_TNUMBER: c_int = 3;
        let frame = (a.tonumber)(l, 1) as u64;
        let flat = lua::to_numbers(a, l, 2).unwrap_or_default();
        let view = if (3..=8).all(|i| (a.type_)(l, i) == LUA_TNUMBER) {
            let n = |i: c_int| (a.tonumber)(l, i);
            Some(([n(3), n(4), n(5)], [n(6), n(7), n(8)]))
        } else {
            None
        };
        let (epoch, anchor) = {
            let link = LINK.lock().unwrap_or_else(|e| e.into_inner());
            (link.epoch, link.anchor)
        };
        let switches = if (a.type_)(l, 9) == LUA_TNUMBER { (a.tonumber)(l, 9) as u32 } else { 0 };
        let n = with_render_out(side, |o| o.frame(frame, &flat, epoch, anchor, view, switches)).unwrap_or(0);
        (a.pushnumber)(l, n as f64);
        1
    })
}

/// gmsky.RenderStatus() -> line for the log (counts since the last call).
unsafe extern "C" fn l_render_status(l: *mut State) -> c_int {
    guarded(l, |a, side| {
        let line = with_render_out(side, |o| o.status()).unwrap_or_else(|e| format!("render out: {e}"));
        lua::push_str(a, l, &line);
        1
    })
}
