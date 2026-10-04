//! gmsky module: the Garry's Mod half of the link.
//!
//! Loaded by the stubs (gmod/stub) through gmsky_open(). Opens the shared memory, gives Lua a
//! `gmsky` table of functions, then runs gmod/lua/gmsky/<server|client>.lua from this project.
//! Logs: logs/gmod_server.log, logs/gmod_client.log

mod lua;

use gmsky_protocol::shm::{now_ms, Mapping, Side};
use gmsky_protocol::PlayerState;
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

fn root() -> PathBuf {
    ROOT.get().cloned().unwrap_or_else(|| PathBuf::from("."))
}

fn log(side: u32, msg: &str) {
    let mut logs = LOGS.lock().unwrap_or_else(|e| e.into_inner());
    let slot = &mut logs[(side == SIDE_CLIENT) as usize];
    if slot.is_none() {
        let dir = root().join("logs");
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
    let funcs: [(&str, lua::CFn); 6] = [
        ("Log", l_log),
        ("Heartbeat", l_heartbeat),
        ("WritePlayerState", l_write_player_state),
        ("Status", l_status),
        ("Root", l_root),
        ("Reload", l_reload),
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
    let rel = format!("gmod/lua/gmsky/{}.lua", side_name(side));
    let path = root().join(&rel);
    match std::fs::read(&path) {
        Ok(code) => match lua::run_chunk(a, l, &code, &rel) {
            Ok(()) => log(side, &format!("ran {rel}")),
            Err(e) => log(side, &format!("ERROR in {rel}: {e}")),
        },
        Err(e) => log(side, &format!("ERROR reading {}: {e}", path.display())),
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

/// gmsky.Root() -> project folder
unsafe extern "C" fn l_root(l: *mut State) -> c_int {
    guarded(l, |a, _| {
        lua::push_str(a, l, &root().display().to_string());
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

/// gmsky.WritePlayerState(tick, x, y, z, pitch, yaw, roll, eye_height, vx, vy, vz, flags, fov)
/// -> origin epoch in use (0 = Skyrim hasn't sent an origin yet)
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
        s.player_state.write(&PlayerState {
            tick: n(1) as u64,
            origin_epoch: link.epoch,
            flags: n(12) as u32,
            anchor: link.anchor,
            pos,
            eye_height: n(8) as f32,
            pitch_deg: n(5) as f32,
            yaw_deg: n(6) as f32,
            roll_deg: n(7) as f32,
            vel: [n(9) as f32, n(10) as f32, n(11) as f32],
            fov_deg: n(13) as f32,
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
