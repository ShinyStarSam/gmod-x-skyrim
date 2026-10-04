//! Out-of-game test for the Garry's Mod side.
//!
//! Loads Garry's Mod's own lua_shared.dll from the install (read only), makes a "server" and a
//! "client" Lua state with a tiny fake of the Garry's Mod functions our scripts use, then loads
//! the stub exactly like `require("gmsky")` would (copies named gmsv_/gmcl_gmsky_win64.dll in
//! build/harness/), runs 200 fake ticks and checks what arrives in shared memory.
//!
//! Usage: gmod_harness            (needs `cargo build --release` first)
//! Env:   GMOD_DIR overrides the Garry's Mod install folder.

use gmsky_protocol::shm::{Mapping, Side};
use gmsky_protocol::Origin;
use std::ffi::{c_char, c_int, c_void, CString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[link(name = "kernel32")]
extern "system" {
    fn SetDllDirectoryW(path: *const u16) -> i32;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn GetLastError() -> u32;
}

type L = *mut c_void;

struct Lua {
    newstate: unsafe extern "C" fn() -> L,
    openlibs: unsafe extern "C" fn(L),
    loadbuffer: unsafe extern "C" fn(L, *const c_char, usize, *const c_char) -> c_int,
    pcall: unsafe extern "C" fn(L, c_int, c_int, c_int) -> c_int,
    tolstring: unsafe extern "C" fn(L, c_int, *mut usize) -> *const c_char,
    settop: unsafe extern "C" fn(L, c_int),
    close: unsafe extern "C" fn(L),
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

unsafe fn sym<T>(m: *mut c_void, name: &str) -> T {
    let c = CString::new(name).unwrap();
    let p = GetProcAddress(m, c.as_ptr());
    assert!(!p.is_null(), "missing export {name}");
    std::mem::transmute_copy(&p)
}

impl Lua {
    /// Runs code; returns Err(message) on a Lua error. If the chunk returns one value, it is
    /// converted to a string and returned.
    unsafe fn run(&self, l: L, code: &str) -> Result<Option<String>, String> {
        let name = CString::new("=harness").unwrap();
        let rc = (self.loadbuffer)(l, code.as_ptr() as *const c_char, code.len(), name.as_ptr());
        if rc != 0 || (self.pcall)(l, 0, 1, 0) != 0 {
            let e = self.str_at(l, -1);
            (self.settop)(l, 0);
            return Err(e.unwrap_or_default());
        }
        let v = self.str_at(l, -1);
        (self.settop)(l, 0);
        Ok(v)
    }

    unsafe fn str_at(&self, l: L, idx: c_int) -> Option<String> {
        let mut n = 0;
        let p = (self.tolstring)(l, idx, &mut n);
        if p.is_null() {
            return None;
        }
        Some(String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, n)).into_owned())
    }
}

/// Just enough of Garry's Mod's Lua API for server.lua / client.lua.
const FAKE_GMOD: &str = r#"
print = function(...) HARNESS_PRINTED = table.concat({...}, " ") end  -- real print needs the game console
hook = { _h = {} }
function hook.Add(ev, name, fn) hook._h[ev] = hook._h[ev] or {}; hook._h[ev][name] = fn end
function hook.Run(ev, ...) for _, fn in pairs(hook._h[ev] or {}) do fn(...) end end
timer = { _t = {} }
function timer.Create(name, delay, reps, fn) timer._t[name] = fn end
function timer.Run(name) timer._t[name]() end
function timer.Simple(delay, fn) HARNESS_SIMPLE = fn end
function RunConsoleCommand(name, value) HARNESS_CONVARS = HARNESS_CONVARS or {}; HARNESS_CONVARS[name] = value end
function GetConVar(name) local v = (HARNESS_CONVARS or {})[name]; return v and { GetString = function() return v end } end
FCVAR_ARCHIVE = 128
function CreateConVar(name, default) return { GetFloat = function() return tonumber(default) end, GetBool = function() return tonumber(default) ~= 0 end } end
function math.NormalizeAngle(a) return (a + 180) % 360 - 180 end
function math.Clamp(v, lo, hi) return math.max(lo, math.min(hi, v)) end
-- Garry's Mod's real IN_* values
IN_ATTACK, IN_JUMP, IN_DUCK, IN_FORWARD, IN_BACK = 1, 2, 4, 8, 16
IN_MOVELEFT, IN_MOVERIGHT, IN_ATTACK2, IN_RELOAD, IN_SPEED, IN_WALK = 512, 1024, 2048, 8192, 131072, 262144
function HARNESS_NEW_CMD()
	local c = { calls = 0 }
	function c:ClearButtons() self.calls = self.calls + 1 end
	function c:ClearMovement() self.calls = self.calls + 1 end
	function c:SetButtons(b) self.buttons = b end
	function c:SetForwardMove(f) self.fwd = f end
	function c:SetSideMove(s) self.side = s end
	function c:SetViewAngles(a) self.p, self.y = a.p, a.y end
	-- the view the Garry's Mod window itself put in the command
	function c:GetViewAngles() return HARNESS_OWN_VIEW or { p = 10, y = 45, r = 0 } end
	return c
end
function IsValid(x) return x ~= nil end
MOVETYPE_WALK = 2
MOVETYPE_NOCLIP = 8
local t = 0
local function vec(x, y, z) return { x = x, y = y, z = z } end
local ply = {}
function ply:GetPos() return vec(100 + t, 50, -12700) end
function ply:EyeAngles() return { p = 10, y = 45, r = 0 } end
function ply:SetEyeAngles(a) HARNESS_EYE = { p = a.p, y = a.y } end
function ply:GetVelocity() return vec(66, 0, 0) end
function ply:IsOnGround() return true end
function ply:Crouching() return false end
function ply:Alive() return true end
function ply:GetMoveType() return MOVETYPE_WALK end
function ply:EyePos() return vec(100 + t, 50, -12700 + 64) end
function ply:GetFOV() return 75 end
function ply:SendLua(s) HARNESS_SENT_LUA = s end
player = { GetAll = function() return { ply } end }
HARNESS_PLY = ply
engine = { TickCount = function() return t end }
function CurTime() return HARNESS_TIME or 0 end
function HARNESS_ADVANCE() t = t + 1 end
"#;

fn main() {
    let gmod = PathBuf::from(std::env::var("GMOD_DIR")
        .unwrap_or_else(|_| r"B:\SteamLibrary\steamapps\common\GarrysMod".into()));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let mut failures = 0;
    let mut check = |ok: bool, what: &str| {
        println!("{} {what}", if ok { "ok  " } else { "FAIL" });
        if !ok { failures += 1; }
    };

    // Log to build/harness/logs, never to logs/ (that holds the real in-game logs).
    let log_dir = root.join("build").join("harness").join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::env::set_var("GMSKY_LOG_DIR", &log_dir);

    unsafe {
        // 1. Garry's Mod's own Lua (lua_shared.dll needs tier0.dll / vstdlib.dll from bin/win64).
        SetDllDirectoryW(wide(&gmod.join("bin").join("win64")).as_ptr());
        let lua_dll = gmod.join("garrysmod").join("bin").join("win64").join("lua_shared.dll");
        let m = LoadLibraryW(wide(&lua_dll).as_ptr());
        if m.is_null() {
            println!("FAIL could not load {} (error {})", lua_dll.display(), GetLastError());
            std::process::exit(1);
        }
        let lua = Lua {
            newstate: sym(m, "luaL_newstate"),
            openlibs: sym(m, "luaL_openlibs"),
            loadbuffer: sym(m, "luaL_loadbuffer"),
            pcall: sym(m, "lua_pcall"),
            tolstring: sym(m, "lua_tolstring"),
            settop: sym(m, "lua_settop"),
            close: sym(m, "lua_close"),
        };
        println!("loaded {}", lua_dll.display());

        // 2. Be Skyrim: publish origin epoch 1 before Garry's Mod starts.
        let sky = Mapping::open(Side::Skyrim).expect("open shared memory");
        sky.shared().origin.write(&Origin { epoch: 1, worldspace_form_id: 0x3C, skyrim_origin: [19000.0, -7000.0, -3300.0] });
        sky.heartbeat();

        // 3. Copy the stub under both require() names and load each like Garry's Mod would.
        let stub = root.join("target").join("release").join("gmsky_stub.dll");
        let dir = root.join("build").join("harness");
        std::fs::create_dir_all(&dir).unwrap();
        let mut states = Vec::new();
        for (file, is_server) in [("gmsv_gmsky_win64.dll", true), ("gmcl_gmsky_win64.dll", false)] {
            let dst = dir.join(file);
            std::fs::copy(&stub, &dst).expect("copy stub (build with `cargo build --release` first)");
            let h = LoadLibraryW(wide(&dst).as_ptr());
            check(!h.is_null(), &format!("LoadLibrary {file}"));
            let open: unsafe extern "C" fn(L) -> c_int = sym(h, "gmod13_open");
            let close: unsafe extern "C" fn(L) -> c_int = sym(h, "gmod13_close");
            let l = (lua.newstate)();
            (lua.openlibs)(l);
            lua.run(l, FAKE_GMOD).expect("fake gmod env");
            lua.run(l, if is_server { "SERVER = true CLIENT = false" } else { "SERVER = false CLIENT = true" }).unwrap();
            open(l);
            let side = lua.run(l, "return gmsky and gmsky.side or 'gmsky table missing'");
            check(side == Ok(Some(if is_server { "server" } else { "client" }.into())),
                  &format!("{file}: gmsky.side = {side:?}"));
            states.push((l, close, is_server));
        }
        let (ls, lc) = (states[0].0, states[1].0);

        // 4. 200 ticks.
        for _ in 0..200 {
            sky.heartbeat();
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            lua.run(lc, "hook.Run('Think')").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        check(lua.run(ls, "timer.Run('gmsky_status')").is_ok(), "server status timer runs");
        check(lua.run(lc, "timer.Run('gmsky_client_status')").is_ok(), "client status timer runs");
        let sent = lua.run(ls, "return HARNESS_SENT_LUA");
        check(sent == Ok(Some(r#"require("gmsky")"#.into())), &format!("server asked client to load: {sent:?}"));
        let status = lua.run(ls, "local c, e, f, w, err = gmsky.Status() return tostring(c)..' '..e..' '..w..' '..tostring(err)");
        check(status == Ok(Some("true 1 200 nil".into())), &format!("gmsky.Status() = {status:?}"));
        check(lua.run(ls, "gmsky.Reload()").is_ok(), "gmsky.Reload() runs");

        // 5. What did Skyrim receive?
        match sky.shared().player_state.read() {
            Some((p, _)) => {
                println!("received: {p:?}");
                check(p.tick == 200, "tick == 200");
                check(p.origin_epoch == 1, "origin_epoch == 1");
                check(p.anchor == [101.0, 50.0, -12700.0], "anchor = position on first tick");
                check(p.pos == [300.0, 50.0, -12700.0], "pos == (300, 50, -12700)");
                check(p.yaw_deg == 45.0 && p.pitch_deg == 10.0, "angles");
                check(p.eye_height == 64.0 && p.fov_deg == 75.0, "eye height and fov");
                check(p.flags == 0x25, "flags = on ground + alive + drive camera (gmsky_camera 1)");
                check(p.vel == [66.0, 0.0, 0.0], "velocity");
            }
            None => check(false, "Skyrim received a player state"),
        }
        check(sky.peer_alive(), "Skyrim sees Garry's Mod heartbeat");

        // 6. Another Garry's Mod-side process takes over: we must stop writing, then resume
        //    when it exits (pid back to 0).
        use std::sync::atomic::Ordering;
        let h = &sky.shared().header;
        let seq_before = sky.shared().player_state.seq();
        h.gmod_pid.store(424242, Ordering::Release);
        lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
        check(sky.shared().player_state.seq() == seq_before, "no writes while another process owns the side");
        check(h.gmod_pid.load(Ordering::Acquire) == 424242, "other owner's pid left alone");
        h.gmod_pid.store(0, Ordering::Release);
        lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
        check(sky.shared().player_state.seq() == seq_before + 2, "writes resume after the other process exits");
        check(h.gmod_pid.load(Ordering::Acquire) == std::process::id(), "side re-claimed");

        // 7. Client heartbeat keeps the link alive without server ticks (paused single-player).
        h.gmod_heartbeat.store(0, Ordering::Release);
        lua.run(lc, "hook.Run('Think')").unwrap();
        check(sky.peer_alive(), "client Think alone keeps the heartbeat fresh");

        // 8. Input from Skyrim's window turns into the player's command.
        use gmsky_protocol::{Input, IN_FORWARD, IN_SPEED};
        let run_cmd = |code: &str| lua.run(ls, &format!(
            "HARNESS_CMD = HARNESS_NEW_CMD() hook.Run('StartCommand', HARNESS_PLY, HARNESS_CMD) {code}"));
        sky.heartbeat();
        sky.shared().input.write(&Input { frame: 1, active: 1, buttons: IN_FORWARD | IN_SPEED, mouse_x_total: 1000, ..Default::default() });
        run_cmd("").unwrap();
        sky.shared().input.write(&Input { frame: 2, active: 1, buttons: IN_FORWARD | IN_SPEED, mouse_x_total: 1100, mouse_y_total: -50, ..Default::default() });
        let r = run_cmd("return string.format('%d %d %d %.2f %.2f', HARNESS_CMD.buttons, HARNESS_CMD.fwd, HARNESS_CMD.side, HARNESS_CMD.y, HARNESS_CMD.p)");
        // buttons IN_FORWARD(8) | IN_SPEED(131072); yaw 45 - 100*0.022*3 = 38.4; pitch 10 - 50*0.066 = 6.7
        check(r == Ok(Some("131080 10000 0 38.40 6.70".into())), &format!("forward+sprint and mouse look: {r:?}"));
        // The next player state tells Skyrim which input is already in the view (for prediction).
        lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
        match sky.shared().player_state.read() {
            Some((p, _)) => check(p.flags & gmsky_protocol::PF_INPUT_APPLIED != 0 && p.input_frame == 2
                    && p.mouse_x_applied == 1100 && p.mouse_y_applied == -50 && (p.deg_per_count - 0.066).abs() < 1e-6,
                &format!("player state reports applied input: flags={:#x} frame={} mouse=({}, {}) k={}",
                    p.flags, p.input_frame, p.mouse_x_applied, p.mouse_y_applied, p.deg_per_count)),
            None => check(false, "player state after input"),
        }
        let r = lua.run(ls, "return string.format('%.2f %.2f', HARNESS_EYE.y, HARNESS_EYE.p)");
        check(r == Ok(Some("38.40 6.70".into())), &format!("Garry's Mod window's view kept in step: {r:?}"));
        // Input stops (Skyrim menu). The window's own view (yaw 45) still lags: hold ours.
        sky.shared().input.write(&Input { frame: 3, active: 0, mouse_x_total: 1100, ..Default::default() });
        let r = run_cmd("return string.format('%d %.2f %.2f', HARNESS_CMD.calls, HARNESS_CMD.y, HARNESS_CMD.p)");
        check(r == Ok(Some("0 38.40 6.70".into())), &format!("input off: buttons left alone, view held: {r:?}"));
        // The window's view is still behind on the next tick: keep holding.
        let r = run_cmd("return tostring(HARNESS_CMD.y)");
        check(r == Ok(Some("38.4".into())), &format!("still held while the window lags: {r:?}"));
        // The window caught up: hand over, the command is left completely alone.
        lua.run(ls, "HARNESS_OWN_VIEW = { p = 6.7, y = 38.4, r = 0 }").unwrap();
        let r = run_cmd("return tostring(HARNESS_CMD.y)");
        check(r == Ok(Some("nil".into())), &format!("window caught up: command left alone: {r:?}"));
        lua.run(ls, "HARNESS_OWN_VIEW = nil").unwrap();
        sky.shared().input.write(&Input { frame: 4, active: 1, buttons: IN_FORWARD, mouse_x_total: 1100, ..Default::default() });
        run_cmd("").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let r = run_cmd("return HARNESS_CMD.calls");
        check(r == Ok(Some("0".into())), &format!("input not rewritten for 300 ms counts as stale: {r:?}"));
        // The window never catches up: give up after 0.5 s and leave the command alone.
        // (after a pause the mouse baseline restarts, so the turn needs two active ticks)
        sky.shared().input.write(&Input { frame: 5, active: 1, mouse_x_total: 1200, ..Default::default() });
        run_cmd("").unwrap();
        sky.shared().input.write(&Input { frame: 6, active: 1, mouse_x_total: 1300, ..Default::default() });
        run_cmd("").unwrap();
        sky.shared().input.write(&Input { frame: 7, active: 0, mouse_x_total: 1300, ..Default::default() });
        let r = run_cmd("return tostring(HARNESS_CMD.y)");
        check(r == Ok(Some("38.4".into())), &format!("second menu: view held: {r:?}"));
        let r = run_cmd("HARNESS_TIME = 1 return tostring(HARNESS_CMD.y)");
        check(r == Ok(Some("38.4".into())), &format!("held until the time is up: {r:?}"));
        let r = run_cmd("return tostring(HARNESS_CMD.y)");
        check(r == Ok(Some("nil".into())), &format!("after 0.5 s: command left alone: {r:?}"));
        let r = lua.run(lc, "return tostring(HARNESS_CONVARS)");
        check(r == Ok(Some("nil".into())), &format!("client runs no console commands: {r:?}"));
        lua.run(lc, "HARNESS_CONVARS = { fps_max_nofocus = '300' } timer.Run('gmsky_client_status')").unwrap();

        for (l, close, _) in states {
            close(l);
            (lua.close)(l);
        }
    }

    let server_log = std::fs::read_to_string(log_dir.join("gmod_server.log")).unwrap_or_default();
    check(server_log.contains("server.lua loaded"), "gmod_server.log: server.lua loaded");
    check(server_log.contains("new origin epoch 1"), "gmod_server.log: new origin epoch 1");
    check(server_log.matches("ERROR").count() == 1 && server_log.contains("another process (pid 424242)"),
          "gmod_server.log: exactly one ERROR, the expected takeover message");
    let client_log = std::fs::read_to_string(log_dir.join("gmod_client.log")).unwrap_or_default();
    check(client_log.contains("client frames in last 5 s: 200 (40 fps) fps_max_nofocus=(no such setting)"),
          "gmod_client.log: counted 200 frames");
    check(client_log.contains("fps_max_nofocus=300"), "gmod_client.log: fps_max_nofocus logged");
    let server_log = std::fs::read_to_string(log_dir.join("gmod_server.log")).unwrap_or_default();
    check(server_log.contains("input from Skyrim: ON") && server_log.contains("input from Skyrim: OFF, holding yaw=38.4 pitch=6.7"),
          "gmod_server.log: input ON and OFF logged");
    check(server_log.contains("view handed to the Garry's Mod window after 2 held ticks (its view yaw=38.4 pitch=6.7, 0.0 degrees from ours)"),
          "gmod_server.log: hand-over logged");
    check(server_log.contains("after 2 held ticks (its view yaw=45.0 pitch=10.0, 6.6 degrees from ours)"),
          "gmod_server.log: hold gives up after 0.5 s");

    println!("{}", if failures == 0 { "HARNESS PASSED" } else { "HARNESS FAILED" });
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
