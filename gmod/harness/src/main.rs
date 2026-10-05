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
function hook.Run(ev, ...) for _, fn in pairs(hook._h[ev] or {}) do local r = fn(...) if r ~= nil then return r end end end
timer = { _t = {} }
function timer.Create(name, delay, reps, fn) timer._t[name] = fn end
function timer.Run(name) timer._t[name]() end
function timer.Simple(delay, fn) HARNESS_SIMPLE = fn end
function RunConsoleCommand(name, value) HARNESS_CONVARS = HARNESS_CONVARS or {}; HARNESS_CONVARS[name] = value end
function GetConVar(name) local v = (HARNESS_CONVARS or {})[name]; return v and { GetString = function() return v end } end
FCVAR_ARCHIVE = 128
FCVAR_REPLICATED = 8192
-- HARNESS_CVARS[name] overrides a convar's value. The step B ground tests run on the ray grid;
-- the step D tests switch gmsky_ground to shapes.
HARNESS_CVARS = { gmsky_ground = "grid" }
function CreateConVar(name, default)
	local function v() local o = HARNESS_CVARS[name] if o ~= nil then return o end return default end
	return { GetFloat = function() return tonumber(v()) end, GetBool = function() return tonumber(v()) ~= 0 end,
		GetString = function() return tostring(v()) end }
end
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
HARNESS_ENTS = {}
scripted_ents = { Register = function(t, name) HARNESS_SENT_CLASS = name HARNESS_ENTS[name] = t end }
concommand = { _c = {}, Add = function(name, fn) HARNESS_CONCOMMAND = name; concommand._c[name] = fn end }
function hook.Remove(ev, name) if hook._h[ev] then hook._h[ev][name] = nil end end
ents = { FindByClass = function() return {} end }
-- A fake entity: SetX(v) stores v as e.X; GetX() reads it back; the scripted entity's own
-- methods (HARNESS_ENTS[class]) work; anything else does nothing.
function ents.Create(class)
	local e = { class = class }
	function e:SetPos(p) self.pos = p end
	function e:GetPos() return self.pos or Vector() end
	function e:Spawn() HARNESS_SPAWNED = self end
	function e:Remove() self.removed = true end
	return setmetatable(e, { __index = function(t, k)
		local own = HARNESS_ENTS[class] and HARNESS_ENTS[class][k]
		if own then return own end
		if k:sub(1, 3) == "Set" then return function(self, v) rawset(self, k:sub(4), v) end end
		if k:sub(1, 3) == "Get" then return function(self) return rawget(self, k:sub(4)) end end
		return function() end
	end })
end
function LocalPlayer() return HARNESS_PLY end
local V = {}
V.__index = V
function Vector(x, y, z) return setmetatable({ x = x or 0, y = y or 0, z = z or 0 }, V) end
V.__add = function(a, b) return Vector(a.x + b.x, a.y + b.y, a.z + b.z) end
V.__sub = function(a, b) return Vector(a.x - b.x, a.y - b.y, a.z - b.z) end
V.__mul = function(a, k) return Vector(a.x * k, a.y * k, a.z * k) end
V.__unm = function(a) return Vector(-a.x, -a.y, -a.z) end
function V:Length() return math.sqrt(self.x * self.x + self.y * self.y + self.z * self.z) end
util = { TraceLine = function() return { Hit = false, HitPos = Vector() } end }
game = { GetMap = function() return "harness" end, GetWorld = function() return "world" end }
MOVETYPE_WALK = 2
MOVETYPE_NOCLIP = 8
local t = 0
local function vec(x, y, z) return { x = x, y = y, z = z } end
local ply = {}
-- HARNESS_POS (a Vector), when set, is where the player is, and SetPos moves it (re-centre test).
function ply:GetPos() return HARNESS_POS or vec(100 + t, 50, -12700) end
function ply:EyeAngles() return { p = 10, y = 45, r = 0 } end
function ply:SetEyeAngles(a) HARNESS_EYE = { p = a.p, y = a.y } end
function ply:GetVelocity() return Vector(66, 0, 0) end
function ply:IsOnGround() return true end
function ply:Crouching() return false end
function ply:Alive() return true end
function ply:GetMoveType() return HARNESS_MOVETYPE or MOVETYPE_WALK end
function ply:SetMoveType(m) HARNESS_MOVETYPE = m end
function ply:EyePos() return vec(100 + t, 50, -12700 + 64) end
function ply:GetFOV() return 75 end
function ply:GetViewPunchAngles() return { p = 2.5, y = -1, r = 0.5 } end
function ply:GetInfoNum(name, default) return default end
function ply:SendLua(s) HARNESS_SENT_LUA = s end
function ply:SetPos(p) HARNESS_SETPOS = p if HARNESS_POS then HARNESS_POS = Vector(p.x, p.y, p.z) end end
function ply:SetVelocity() end
function ply:GetGroundEntity() return nil end
player = { GetAll = function() return { ply } end }
HARNESS_PLY = ply
engine = { TickCount = function() return t end, TickInterval = function() return 0.015 end }
function CurTime() return HARNESS_TIME or 0 end
function SysTime() return os.clock() end
function ScrW() return 1280 end
function ScrH() return 720 end
function HARNESS_ADVANCE() t = t + 1 end
"#;

/// Collision ring messages, as Skyrim's Collision.cpp writes them (gmsky_protocol.h).
fn clear_msg(col_epoch: u32, origin_epoch: u32) -> Vec<u8> {
    let mut v = col_epoch.to_le_bytes().to_vec();
    v.extend_from_slice(&origin_epoch.to_le_bytes());
    v
}

fn region_msg(r: [i32; 3], col_epoch: u32, size: f32, tris: &[[f32; 9]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(40 + 40 * tris.len());
    for x in r {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&col_epoch.to_le_bytes());
    v.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    for x in r {
        v.extend_from_slice(&(x as f32 * size).to_le_bytes()); // lattice starts at 0 here
    }
    v.extend_from_slice(&size.to_le_bytes());
    for t in tris {
        for x in t {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&0u32.to_le_bytes());
    }
    v
}

/// Puts each triangle into every region (size `size`, lattice at 0) its box comes within 1 unit
/// of, as Skyrim sends them.
fn regions_of(tris: &[[f32; 9]], size: f32) -> std::collections::BTreeMap<[i32; 3], Vec<[f32; 9]>> {
    let mut m: std::collections::BTreeMap<[i32; 3], Vec<[f32; 9]>> = Default::default();
    for t in tris {
        let lo = |k: usize| ((t[k].min(t[3 + k]).min(t[6 + k]) - 1.0) / size).floor() as i32;
        let hi = |k: usize| ((t[k].max(t[3 + k]).max(t[6 + k]) + 1.0) / size).floor() as i32;
        for x in lo(0)..=hi(0) {
            for y in lo(1)..=hi(1) {
                for z in lo(2)..=hi(2) {
                    m.entry([x, y, z]).or_default().push(*t);
                }
            }
        }
    }
    m
}

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
                check(p.flags == 0x65, "flags = on ground + alive + drive camera + smooth");
                check(p.tick_qpc > 0 && (p.tick_ms - 15.0).abs() < 1e-4, &format!("tick stamp {} and length {} ms", p.tick_qpc, p.tick_ms));
                check(p.vel == [66.0, 0.0, 0.0], "velocity");
                check(p.punch_deg == [2.5, -1.0, 0.5], &format!("view punch {:?}", p.punch_deg));
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
        // V in Skyrim (step 3e): each new press toggles noclip through the PlayerNoClip hook.
        lua.run(ls, "hook.Add('PlayerNoClip', 'harness', function(p, on) HARNESS_NOCLIP_ASKED = on return true end)").unwrap();
        sky.shared().input.write(&Input { frame: 8, active: 1, mouse_x_total: 1300, noclip_presses: 4, ..Default::default() });
        run_cmd("").unwrap();
        sky.shared().input.write(&Input { frame: 9, active: 1, mouse_x_total: 1300, noclip_presses: 5, ..Default::default() });
        let r = run_cmd("return tostring(HARNESS_NOCLIP_ASKED) .. ' ' .. HARNESS_MOVETYPE");
        check(r == Ok(Some("true 8".into())), &format!("V: noclip on (presses before input started don't count): {r:?}"));
        sky.shared().input.write(&Input { frame: 10, active: 1, mouse_x_total: 1300, noclip_presses: 6, ..Default::default() });
        let r = run_cmd("return tostring(HARNESS_NOCLIP_ASKED) .. ' ' .. HARNESS_MOVETYPE");
        check(r == Ok(Some("false 2".into())), &format!("V again: noclip off: {r:?}"));
        sky.shared().input.write(&Input { frame: 11, active: 1, mouse_x_total: 1300, noclip_presses: 8, ..Default::default() });
        let r = run_cmd("return HARNESS_MOVETYPE");
        check(r == Ok(Some("2".into())), &format!("two presses in one tick: on and off again: {r:?}"));
        sky.shared().input.write(&Input { frame: 12, active: 0, mouse_x_total: 1300, noclip_presses: 8, ..Default::default() });
        run_cmd("").unwrap();
        let r = lua.run(lc, "return tostring(HARNESS_CONVARS)");
        check(r == Ok(Some("nil".into())), &format!("client runs no console commands: {r:?}"));
        lua.run(lc, "HARNESS_CONVARS = { fps_max_nofocus = '300' } timer.Run('gmsky_client_status')").unwrap();

        // 8b. A ground grid from Skyrim reaches Lua (Phase 1 step 3b) and becomes the ground
        //     (re-plan step B: triangles in one gmsky_field entity, no physics objects).
        {
            let mut t = Box::new(gmsky_protocol::Terrain::default());
            t.id = 7;
            t.origin_epoch = 1;
            t.n = 65;
            t.misses = 1;
            // Relative to the anchor (protocol 7): the anchor is (101, 50, -12700), so this grid is
            // centred at (100, 50) with ground at about z -12000 in Garry's Mod.
            t.center = [-1.0, 0.0];
            t.spacing = 25.5;
            for (k, h) in t.height.iter_mut().enumerate() { *h = 700.0 + k as f32 * 0.01; }
            t.height[0] = f32::NAN;
            sky.shared().terrain.write(&t);
            lua.run(ls, "hook.Run('Think')").unwrap();
            let r = lua.run(ls, "local g = gmsky.terrain.grid return string.format('%d %d %d %.1f %d', g.id, g.n, #g.h, g.h[2], g.h[1] ~= g.h[1] and 1 or 0)");
            check(r == Ok(Some("7 65 4225 700.0 1".into())), &format!("terrain grid reaches Lua: {r:?}"));
            // Height lookup (centre point = index 32*65+32).
            let r = lua.run(ls, "local T, g = gmsky.terrain, gmsky.terrain.grid return string.format('%.2f %s', T.GridHeight(g, -1, 0), tostring(T.GridHeight(g, -1 + 2000, 0)))");
            check(r == Ok(Some("721.12 nil".into())), &format!("grid height lookup (relative to the anchor): {r:?}"));
            // One tick: grid 7 is the ground (the miss leaves out one square: 8,190 triangles), and the
            // held player is let go on top of it.
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local T = gmsky.terrain return string.format('%s %d %d %s %.0f', T.ent.class, T.ent.GridId, T.ent.field.squares, tostring(T.holding), HARNESS_SETPOS.z)");
            let ok = matches!(&r, Ok(Some(s)) if s.starts_with("gmsky_field 7 4095 false -119"));
            check(ok, &format!("grid 7 is the ground after one tick, player let go on top: {r:?}"));
            // The entity answers a player-sized box falling onto it (startpos = bottom of the box).
            let r = lua.run(ls, "local T = gmsky.terrain local tr = HARNESS_ENTS.gmsky_field.TestCollision(T.ent, Vector(100, 50, -11900), Vector(0, 0, -200), true, Vector(16, 16, 36), 0) \
                return string.format('%s %s %s', tostring(tr.Fraction > 0.39 and tr.Fraction < 0.40), tostring(math.abs(tr.HitPos.z + 11978.5) < 0.5), tostring(tr.Normal.z > 0.99))");
            check(r == Ok(Some("true true true".into())), &format!("ground entity's TestCollision stops a falling box on the grid: {r:?}"));
            // A long ray (tested in 128-unit sections) slanting down onto the grid: same hit point as
            // its short last part, and only the squares along it get triangles.
            let r = lua.run(ls, "local T = gmsky.terrain local f = gmsky.field.FromGrid(T.grids[7])                 local fr = gmsky.field.Sweep(f, -700, 0, 1700, 0, 0, 0, 1400, 0, -2000)                 local z = 1700 - 2000 * fr local x = -700 + 1400 * fr                 return string.format('%s %s', tostring(math.abs(z - T.GridHeight(T.grids[7], x, 0)) < 0.1), tostring(f.made < 300))");
            check(r == Ok(Some("true true".into())), &format!("long slanted ray hits the surface and makes < 300 triangles: {r:?}"));
            // A vertical ray exactly on a grid line (relative y = 0) between two squares still hits.
            let r = lua.run(ls, "local T = gmsky.terrain local F = gmsky.field local f = F.FromGrid(T.grids[7]) \
                local fr = F.Sweep(f, -15, 0, 800, 0, 0, 0, 0, 0, -200) return tostring(fr ~= nil)");
            check(r == Ok(Some("true".into())), &format!("ray along the edge between two squares hits: {r:?}"));
            // Grid 8, one piece along x: the ground on the next tick.
            t.id = 8;
            t.center = [-1.0 + 16.0 * 25.5, 0.0];
            sky.shared().terrain.write(&t);
            lua.run(ls, "hook.Run('Think') HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "return tostring(gmsky.terrain.ent.GridId)");
            check(r == Ok(Some("8".into())), &format!("grid 8 is the ground on the next tick: {r:?}"));
            // Grid 9 far away: no ground under the player: held. Noclip lets go; walking again holds.
            t.id = 9;
            t.center = [5000.0, 5000.0];
            sky.shared().terrain.write(&t);
            lua.run(ls, "hook.Run('Think') HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local T = gmsky.terrain local a = tostring(T.holding) .. ' ' .. tostring(T.holdWhy) \
                HARNESS_MOVETYPE = MOVETYPE_NOCLIP HARNESS_ADVANCE() hook.Run('Tick') local b = tostring(T.holding) \
                HARNESS_MOVETYPE = MOVETYPE_WALK HARNESS_ADVANCE() hook.Run('Tick') return a .. ' ' .. b .. ' ' .. tostring(T.holding)");
            check(r == Ok(Some("true no ground false true".into())), &format!("no ground: held; noclip lets go; walking holds again: {r:?}"));
            // Grid 10 back under the player: let go, with its speed given back.
            t.id = 10;
            t.center = [-1.0, 0.0];
            sky.shared().terrain.write(&t);
            lua.run(ls, "hook.Run('Think') HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "return tostring(gmsky.terrain.holding)");
            check(r == Ok(Some("false".into())), &format!("grid 10 under the player: let go: {r:?}"));
            // Re-centre (re-plan step C): home (0, 0, 0), limit 500; the player at (900.5, 50, -11950),
            // ~8 above grid 10, moves by (-900, -50, 11950) to (0.5, 0, 0), the anchor (101, 50, -12700)
            // by the same to (-799, 0, -750), and the ground entity with it. Player minus anchor stays
            // (799.5, 0, 750), so Skyrim's position of the player doesn't change; the ground is still
            // under the player.
            lua.run(ls, "local T = gmsky.terrain T.home = Vector(0, 0, 0) T.recentreAt = { GetFloat = function() return 500 end } \
                HARNESS_POS = Vector(900.5, 50, -11950) HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            match sky.shared().player_state.read() {
                Some((p, _)) => check(p.pos == [0.5, 0.0, 0.0] && p.anchor == [-799.0, 0.0, -750.0],
                    &format!("re-centre: player state pos {:?} anchor {:?} (player minus anchor unchanged)", p.pos, p.anchor)),
                None => check(false, "player state after re-centre"),
            }
            let r = lua.run(ls, "local T = gmsky.terrain local e = T.ent:GetPos() \
                local tr = HARNESS_ENTS.gmsky_field.TestCollision(T.ent, Vector(0.5, 0, 100), Vector(0, 0, -200), true, Vector(16, 16, 36), 0) \
                local want = e.z + T.GridHeight(T.grids[10], 0.5 - e.x, 0 - e.y) \
                return string.format('%.1f %.1f %.1f %d %s %s', e.x, e.y, e.z, T.recentres, tostring(T.holding), tostring(tr ~= nil and math.abs(tr.HitPos.z - want) < 1))");
            check(r == Ok(Some("-799.0 0.0 -750.0 1 false true".into())), &format!("re-centre: ground entity moved with the anchor, player not held, still stands on grid 10: {r:?}"));
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick') HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "return tostring(gmsky.terrain.recentres)");
            check(r == Ok(Some("1".into())), &format!("re-centre: only once (the player is home now): {r:?}"));
            lua.run(ls, "HARNESS_POS = nil gmsky.terrain.home = nil").unwrap();
            // The module still records grids (logs/terrain), and reads them back.
            let r = lua.run(ls, "local id, e, n, cx, cy, sp, m, h = gmsky.ReadGridFile('build/harness/logs/terrain/grid_7.bin') \
                return string.format('%d %d %d %.1f %.1f %.1f %d %d %.1f', id, e, n, cx, cy, sp, m, #h, h[2])");
            check(r == Ok(Some("7 1 65 -1.0 0.0 25.5 1 4225 700.0".into())), &format!("ReadGridFile reads a recorded grid: {r:?}"));
            let r = lua.run(ls, "local ok, err = gmsky.ReadGridFile('../secret.bin') return tostring(ok) .. ' ' .. err");
            check(r == Ok(Some("false bad path \"../secret.bin\"".into())), &format!("ReadGridFile refuses '..': {r:?}"));
        }

        // 8d. Skyrim's collision shapes (re-plan step D): collision ring -> the module's store ->
        //     gmsky.Sweep from the ground entity. The anchor is (-799, 0, -750) since the
        //     re-centre test; the player is put at world (0, 0, 0) = (799, 0, 750) relative.
        {
            use gmsky_protocol::{COL_CLEAR, COL_REGION, PF_SHAPES};
            lua.run(ls, "HARNESS_CVARS.gmsky_ground = 'shapes' HARNESS_POS = Vector(0, 0, 0) HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local T = gmsky.terrain return string.format('%s %s %s', tostring(T.ent:GetShapes()), tostring(T.holding), tostring(T.holdWhy))");
            check(r == Ok(Some("true true waiting for Skyrim's collision".into())), &format!("shapes: ground entity switched to shapes; player held until collision arrives: {r:?}"));
            match sky.shared().player_state.read() {
                Some((p, _)) => check(p.flags & PF_SHAPES != 0, &format!("shapes: player state asks Skyrim for shapes (flags {:#x})", p.flags)),
                None => check(false, "player state in shapes mode"),
            }
            // Skyrim: clear, then a floor at relative z 740 (x 700..900, y -100..100) in the two
            // regions it touches, and the (empty) regions below them.
            let floor = [[700.0, -100.0, 740.0, 900.0, -100.0, 740.0, 900.0, 100.0, 740.0f32],
                         [700.0, -100.0, 740.0, 900.0, 100.0, 740.0, 700.0, 100.0, 740.0]];
            check(sky.ring_write(COL_CLEAR, &clear_msg(1, 1)), "shapes: clear written to the ring");
            for ry in [0, -1] {
                sky.ring_write(COL_REGION, &region_msg([2, ry, 2], 1, 272.0, &floor));
                sky.ring_write(COL_REGION, &region_msg([2, ry, 1], 1, 272.0, &[]));
            }
            lua.run(ls, "hook.Run('Think') HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local T = gmsky.terrain local n, t = gmsky.ShapesStatus(false) return string.format('%d %d %s', n, t, tostring(T.holding))");
            check(r == Ok(Some("4 4 false".into())), &format!("shapes: 4 regions (4 triangles) arrive; player let go: {r:?}"));
            // Feet at world (0, 0, 50) falling 100: the floor is at world z 740 - 750 = -10.
            let r = lua.run(ls, "local T = gmsky.terrain local tr = HARNESS_ENTS.gmsky_field.TestCollision(T.ent, Vector(0, 0, 50), Vector(0, 0, -100), true, Vector(16, 16, 36), 0) \
                return tr and string.format('%.4f %.1f %.2f', tr.Fraction, tr.HitPos.z, tr.Normal.z) or 'nil'");
            check(r == Ok(Some("0.5997 -10.0 1.00".into())), &format!("shapes: entity's TestCollision stops a falling box on Skyrim's floor: {r:?}"));
            let r = lua.run(ls, "local e = gmsky.terrain.ent local a, b = e:GetBoundsLo(), e:GetBoundsHi() return string.format('%d %d %d %d %d %d', a.x, a.y, a.z, b.x, b.y, b.z)");
            check(r == Ok(Some("543 -273 271 817 273 817".into())), &format!("shapes: entity bounds cover the regions: {r:?}"));

            // The Rust sweep gives the same answers as field.lua's on the same triangles: a hill
            // (64 x 64 squares) and 400 random triangles, as f32 (what the ring carries).
            let mut tris: Vec<[f32; 9]> = Vec::new();
            let hill = |x: f64, y: f64| (120.0 * (1.0 + ((x - 2000.0) / 250.0).sin()) + 80.0 * (1.0 + (y / 180.0).cos())) as f32;
            for i in 0..64 {
                for j in 0..64 {
                    let (x0, y0) = (1184.0 + i as f64 * 25.5, -816.0 + j as f64 * 25.5);
                    let (x1, y1) = (x0 + 25.5, y0 + 25.5);
                    let p = |x: f64, y: f64| [x as f32, y as f32, hill(x, y)];
                    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1));
                    tris.push([a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]]);
                    tris.push([a[0], a[1], a[2], c[0], c[1], c[2], d[0], d[1], d[2]]);
                }
            }
            let mut seed = 12345u64;
            let mut rnd = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((seed >> 33) as f64) / (1u64 << 31) as f64 };
            for _ in 0..400 {
                let (cx, cy, cz) = (1200.0 + rnd() * 1600.0, -800.0 + rnd() * 1600.0, rnd() * 450.0);
                let mut t = [0f32; 9];
                for v in 0..3 {
                    t[v * 3] = (cx + (rnd() - 0.5) * 120.0) as f32;
                    t[v * 3 + 1] = (cy + (rnd() - 0.5) * 120.0) as f32;
                    t[v * 3 + 2] = (cz + (rnd() - 0.5) * 120.0) as f32;
                }
                tris.push(t);
            }
            sky.ring_write(COL_CLEAR, &clear_msg(2, 1));
            let regions = regions_of(&tris, 272.0);
            let mut written = true;
            for (r, list) in &regions {
                written &= sky.ring_write(COL_REGION, &region_msg(*r, 2, 272.0, list));
            }
            check(written, &format!("shapes: {} regions ({} triangles) written to the ring", regions.len(), tris.len()));
            lua.run(ls, "hook.Run('Think')").unwrap();
            // The same triangles into a field.lua field (exact f64 copies of the f32 values).
            lua.run(ls, "HARNESS_TRIS = {}").unwrap();
            for chunk in tris.chunks(500) {
                let nums: Vec<String> = chunk.iter().flat_map(|t| t.iter().map(|x| format!("{:?}", *x as f64))).collect();
                lua.run(ls, &format!("local t = HARNESS_TRIS for _, v in ipairs({{{}}}) do t[#t + 1] = v end", nums.join(","))).unwrap();
            }
            let r = lua.run(ls, "local F, T = gmsky.field, HARNESS_TRIS local f = F.New() \
                for i = 1, #T, 9 do F.AddTri(f, {T[i], T[i+1], T[i+2]}, {T[i+3], T[i+4], T[i+5]}, {T[i+6], T[i+7], T[i+8]}) end \
                math.randomseed(7) local cases, hits, bad, ties, lt, rt = 0, 0, 0, 0, 0, 0 \
                local ext = { {16, 16, 36}, {16, 16, 18}, {0, 0, 0}, {8, 8, 36} } local lens = { 4, 40, 300, 3000 } \
                for k = 1, 3000 do \
                  local cx, cy, cz = 1200 + math.random() * 1600, -800 + math.random() * 1600, math.random() * 500 \
                  local e = ext[math.random(4)] local L = lens[math.random(4)] \
                  local dx, dy, dz = math.random() - 0.5, math.random() - 0.5, math.random() - 0.7 \
                  local n = math.sqrt(dx * dx + dy * dy + dz * dz) dx, dy, dz = dx / n * L, dy / n * L, dz / n * L \
                  local t0 = os.clock() local a, ax, ay, az = F.Sweep(f, cx, cy, cz, e[1], e[2], e[3], dx, dy, dz) lt = lt + os.clock() - t0 \
                  t0 = os.clock() local b, bx, by, bz = gmsky.Sweep(cx, cy, cz, e[1], e[2], e[3], dx, dy, dz) rt = rt + os.clock() - t0 \
                  cases = cases + 1 if a then hits = hits + 1 end \
                  if (a == nil) ~= (b == nil) or (a and math.abs(a - b) > 1e-9) then bad = bad + 1 \
                  elseif a and (math.abs(ax - bx) > 1e-9 or math.abs(ay - by) > 1e-9 or math.abs(az - bz) > 1e-9) then ties = ties + 1 end \
                end \
                return string.format('%d %d %d %.4f %.4f %d', cases, hits, bad, lt * 1000 / cases, rt * 1000 / cases, ties)");
            // (A "tie": same fraction, another triangle's normal. The box touches several triangles at
            // the same moment, e.g. it starts inside a corner; which one is reported depends on the
            // order they are looked at, and either is right.)
            println!("shapes: Rust vs Lua sweeps (cases hits mismatches lua-ms rust-ms ties): {r:?}");
            let ok = matches!(&r, Ok(Some(s)) if { let v: Vec<&str> = s.split(' ').collect(); v.len() == 6 && v[2] == "0" && v[1].parse::<u32>().unwrap_or(0) > 300 });
            check(ok, &format!("shapes: Rust sweep = field.lua sweep in 3000 random sweeps over {} regions: {r:?}", regions.len()));
            // A new "clear" drops everything; the player is held again (in the air above nothing).
            sky.ring_write(COL_CLEAR, &clear_msg(3, 1));
            lua.run(ls, "hook.Run('Think') HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local n, t = gmsky.ShapesStatus(true) return string.format('%d %d %s', n, t, tostring(gmsky.terrain.holding))");
            check(r == Ok(Some("0 0 true".into())), &format!("shapes: clear drops every region; player held: {r:?}"));
            lua.run(ls, "HARNESS_CVARS.gmsky_ground = 'grid' HARNESS_POS = nil gmsky.terrain.holding = false").unwrap();
        }

        // 9. Phase 2a: test pictures are saved only as plain names under logs/overlay/, byte for byte.
        {
            let r = lua.run(lc, "local ok, err = gmsky.WriteLogFile('../escape.png', 'x') return tostring(ok) .. ' ' .. tostring(err)");
            check(r == Ok(Some("false bad file name \"../escape.png\"".into())), &format!("WriteLogFile refuses a path: {r:?}"));
            let r = lua.run(lc, r"return tostring(gmsky.WriteLogFile('t_bin.png', 'ab\0cd\255'))");
            let bytes = std::fs::read(log_dir.join("overlay").join("t_bin.png")).unwrap_or_default();
            check(r == Ok(Some("true".into())) && bytes == b"ab\0cd\xff", &format!("WriteLogFile keeps binary data: {r:?} {bytes:?}"));
            // 2D alpha test (idea 1): with no game graphics loaded, D3DState says so instead of crashing.
            let r = lua.run(lc, "return gmsky.D3DState()");
            check(r == Ok(Some("not found: shaderapidx9.dll is not loaded".into())), &format!("D3DState without the game: {r:?}"));
            // Step 2b-1: without the game's graphics the Present hook isn't installed, says why,
            // and its status stays nil (so the 5 s status timer logs nothing).
            let r = lua.run(lc, "return gmsky.OverlayHook()");
            check(r == Ok(Some("hook NOT installed, device not found: shaderapidx9.dll is not loaded".into())), &format!("OverlayHook without the game: {r:?}"));
            let r = lua.run(lc, "concommand._c.gmsky_overlay_probe(nil, 'gmsky_overlay_probe', {}) timer.Run('gmsky_liveoverlay_status') return tostring((gmsky.OverlayStatus()))");
            check(r == Ok(Some("nil".into())), &format!("gmsky_overlay_probe and its status timer without the game: {r:?}"));
            // Step 2b-3: gmsky_overlay 1 prepares each frame (a fake render library records the
            // calls): cleared once where the view model starts (or at the HUD without one), alpha
            // forced for the HUD, everything set back after the frame; nothing when it is off.
            let frame_lua = r#"
                HARNESS_RENDER = {}
                local function rec(name) return function(...) HARNESS_RENDER[#HARNESS_RENDER + 1] = name .. "(" .. table.concat((function(...) local t = {} for i = 1, select('#', ...) do t[i] = tostring((select(i, ...))) end return t end)(...), ",") .. ")" end end
                render = { SetWriteDepthToDestAlpha = rec("depthAlpha"), Clear = rec("Clear"), OverrideAlphaWriteEnable = rec("alphaWrite") }
                local function frame(viewmodel)
                    hook.Run("PreRender")
                    if viewmodel then hook.Run("PreDrawViewModel") hook.Run("PreDrawPlayerHands") end
                    hook.Run("PreDrawHUD")
                    hook.Run("PostRender")
                    hook.Run("DrawOverlay")
                end
                local out = {}
                for _, step in ipairs({ { "1", true }, { "1", false }, { "0", true } }) do
                    HARNESS_CVARS.gmsky_overlay = step[1]
                    HARNESS_RENDER = {}
                    frame(step[2])
                    out[#out + 1] = table.concat(HARNESS_RENDER, " ") .. " | forced=" .. tostring(gmsky.alphaForced)
                end
                hook.Run("PreRender")
                return table.concat(out, " / ")
            "#;
            let r = lua.run(lc, frame_lua);
            let want = "depthAlpha(false) Clear(0,0,0,0,true,true) alphaWrite(true,true) depthAlpha(true) alphaWrite(false) | forced=false / \
                        depthAlpha(false) Clear(0,0,0,0,true,true) alphaWrite(true,true) depthAlpha(true) alphaWrite(false) | forced=false /  | forced=false";
            check(r == Ok(Some(want.into())), &format!("gmsky_overlay 1 prepares each frame, 0 leaves it alone: {r:?}"));
            // Patched HOLOHUD 2 copy (tools/build_holohud.py), if built: its render.lua with our
            // patch appended compiles in Garry's Mod's Lua (compiled only, not run).
            let holo = root.join("build").join("holohud2_gmsky").join("lua").join("holohud2").join("render.lua");
            if let Ok(code) = std::fs::read_to_string(&holo) {
                // "do return end" first: the whole file is compiled, nothing of it runs.
                let r = lua.run(lc, &format!("do return 'compiles' end\n{code}"));
                check(r == Ok(Some("compiles".into())), &format!("patched HOLOHUD render.lua compiles: {r:?}"));
            }
        }

        // 8c. Collision field (re-plan step A): swept box against triangles.
        {
            let sweep = |setup: &str, args: &str| lua.run(ls, &format!(
                "local F = gmsky.field local f = F.New() {setup}                  local fr, nx, ny, nz = F.Sweep(f, {args})                  if not fr then return 'nil' end return string.format('%.4f %.2f %.2f %.2f', fr, nx + 0, ny + 0, nz + 0)"));
            let floor = "F.AddQuad(f, {-100, -100, 0}, {100, -100, 0}, {100, 100, 0}, {-100, 100, 0})";
            let r = sweep(floor, "0, 0, 46, 16, 16, 36, 0, 0, -20");
            check(r == Ok(Some("0.4984 0.00 0.00 1.00".into())), &format!("field: box falls onto a floor, stops 1/32 above it: {r:?}"));
            let r = sweep(floor, "0, 0, 50, 0, 0, 0, 0, 0, -100");
            check(r == Ok(Some("0.4997 0.00 0.00 1.00".into())), &format!("field: ray hits the floor: {r:?}"));
            let r = sweep(floor, "0, 0, 37, 16, 16, 36, 100, 0, 0");
            check(r == Ok(Some("nil".into())), &format!("field: walking 1 unit above the floor touches nothing: {r:?}"));
            let r = sweep("F.AddBox(f, {50, -4, 0}, {58, 4, 100})", "0, 0, 37, 16, 16, 36, 100, 0, 0");
            check(r == Ok(Some("0.3397 -1.00 0.00 0.00".into())), &format!("field: walking into a post stops at its face: {r:?}"));
            let r = sweep(floor, "0, 0, 30, 16, 16, 36, 0, 0, 10");
            check(r == Ok(Some("nil".into())), &format!("field: a box inside the floor moving out is let go: {r:?}"));
            let r = sweep(floor, "0, 0, 30, 16, 16, 36, 0, 0, -10");
            check(r == Ok(Some("0.0000 0.00 0.00 1.00".into())), &format!("field: a box inside the floor moving further in is stopped: {r:?}"));
            // 50-degree ramp rising along +x: the hit normal leans back at 50 degrees (z = cos 50 = 0.64).
            let r = sweep("local run = 150 / math.tan(math.rad(50)) F.AddQuad(f, {0, -100, 0}, {run, -100, 150}, {run, 100, 150}, {0, 100, 0})",
                "-40, 0, 37, 16, 16, 36, 100, 0, 0");
            let ok = matches!(&r, Ok(Some(s)) if s.ends_with("-0.77 0.00 0.64"));
            check(ok, &format!("field: walking into a 50-degree ramp hits its slope: {r:?}"));
            let r = lua.run(ls, "local F = gmsky.field local f = F.BuildScene(1, Vector(0, 0, 0)) return tostring(#f.tris)");
            check(r == Ok(Some("8352".into())), &format!("field: test scene triangle count: {r:?}"));
            // Speed: 2,000 player-sized sweeps over the hill (~100 a tick is a busy tick).
            let r = lua.run(ls, "local F = gmsky.field local f = F.BuildScene(1, Vector(0, 0, 0)) local t0 = os.clock()                 for k = 1, 2000 do F.Sweep(f, -900 + k % 300, (k * 7) % 600 - 300, 300, 16, 16, 36, 10, 5, -300) end                 return string.format('%.4f', (os.clock() - t0) * 1000 / 2000)");
            println!("field: ms per sweep over the hill: {r:?}");
        }

        // 9. The auto-loader (garrysmod/lua/autorun/server): waits for the player's first real
        //    command, then requires gmsky once. Fresh Lua state, gmsky not loaded.
        let la = (lua.newstate)();
        (lua.openlibs)(la);
        lua.run(la, FAKE_GMOD).expect("fake gmod env");
        lua.run(la, "game = { SinglePlayer = function() return true end } HARNESS_REQUIRED = 0             function require(m) HARNESS_REQUIRED = HARNESS_REQUIRED + 1 gmsky = {} end             function hook.Remove(ev, name) hook._h[ev][name] = nil end").unwrap();
        let autoload = std::fs::read_to_string(root.join("gmod").join("autorun").join("gmsky_autoload.lua")).unwrap();
        check(lua.run(la, &autoload).is_ok(), "gmsky_autoload.lua runs");
        let r = lua.run(la, "HARNESS_SIMPLE = nil hook.Run('SetupMove', HARNESS_PLY, nil, { IsForced = function() return true end })             return tostring(HARNESS_SIMPLE)");
        check(r == Ok(Some("nil".into())), &format!("autoload: forced (not real) command loads nothing: {r:?}"));
        let r = lua.run(la, "hook.Run('SetupMove', HARNESS_PLY, nil, { IsForced = function() return false end }) HARNESS_SIMPLE()             hook.Run('SetupMove', HARNESS_PLY, nil, { IsForced = function() return false end })             return HARNESS_REQUIRED .. ' ' .. tostring(hook._h.SetupMove.gmsky_autoload)");
        check(r == Ok(Some("1 nil".into())), &format!("autoload: first real command requires gmsky once, hook removed: {r:?}"));
        (lua.close)(la);
        let la = (lua.newstate)();
        (lua.openlibs)(la);
        lua.run(la, FAKE_GMOD).expect("fake gmod env");
        lua.run(la, "game = { SinglePlayer = function() return false end }").unwrap();
        lua.run(la, &autoload).unwrap();
        let r = lua.run(la, "return tostring(hook._h.SetupMove)");
        check(r == Ok(Some("nil".into())), &format!("autoload: does nothing outside single-player: {r:?}"));
        (lua.close)(la);

        for (l, close, _) in states {
            close(l);
            (lua.close)(l);
        }
    }

    let server_log = std::fs::read_to_string(log_dir.join("gmod_server.log")).unwrap_or_default();
    check(server_log.contains("server.lua loaded"), "gmod_server.log: server.lua loaded");
    check(server_log.contains("ran gmod/lua/gmsky/terrain.lua"), "gmod_server.log: terrain.lua runs on the server");
    check(server_log.contains("ran gmod/lua/gmsky/field.lua"), "gmod_server.log: field.lua runs on the server");
    check(server_log.contains("terrain: grid 7 is the ground now (4095 squares, "), "gmod_server.log: grid 7 is the ground");
    check(server_log.contains("terrain: grid 8 is the ground now (4095 squares, "), "gmod_server.log: grid 8 is the ground");
    check(server_log.contains("terrain: hold released (noclip) after "), "gmod_server.log: noclip ends a hold");
    check(server_log.contains("terrain: hold released (no ground) after 1 ticks, speed (66, 0, 0) given back"),
          "gmod_server.log: a no-ground hold gives the speed back");
    check(server_log.contains("noclip ON (V in Skyrim) at (") && server_log.contains("noclip OFF (V in Skyrim) at ("),
          "gmod_server.log: noclip toggles logged");
    check(server_log.contains("terrain: HOLD at") && server_log.contains("terrain: ground ready under the player (grid 7): released at z"),
          "gmod_server.log: held without ground, released when grid 7 is built");
    check(server_log.contains("terrain: grid 7 from Skyrim (origin epoch 1): 65x65 points 25.5 apart, centre (-1.0, 0.0), ground z 700.0..742.2 (relative to the anchor), 1 misses (1 NaN)"),
          "gmod_server.log: terrain grid logged");
    check(server_log.contains("new origin epoch 1"), "gmod_server.log: new origin epoch 1");
    check(server_log.contains("origin: RE-CENTRE #1 (player more than 500 from home (0, 0, 0)): player (900.50, 50.00, -11950.00) -> (0.50, 0.00, 0.00), shift (-900, -50, 11950)")
          && server_log.contains("player minus anchor (799.500, 0.000, 750.000) before, (799.500, 0.000, 750.000) after (must match)"),
          "gmod_server.log: re-centre logged with matching player-minus-anchor");
    check(server_log.contains("anchor shifted by (-900.0, -50.0, 11950.0): (101.0, 50.0, -12700.0) -> (-799.0, 0.0, -750.0), origin epoch 1 unchanged"),
          "gmod_server.log: module logs the anchor shift");
    check(server_log.contains("origin: 2 ticks after re-centre #1: player (0.5, 0.0, 0.0) speed (66, 0, 0)"),
          "gmod_server.log: speed logged after the re-centre");
    // Jump log: the harness puts the player 800 units away in one tick (before the re-centre),
    // which no speed explains; the re-centre itself is not a jump (positions relative to the anchor).
    check(server_log.contains("JUMP tick ") && server_log.contains("speed explains (1.0, 0.0, 0.0)"),
          "gmod_server.log: a teleport no speed explains is logged as a JUMP");
    check(server_log.matches("JUMP tick ").count() == 1, &format!("gmod_server.log: exactly one JUMP ({})", server_log.matches("JUMP tick ").count()));
    check(server_log.contains("shapes: clear (collision epoch 1, origin epoch 1): 0 regions dropped")
          && server_log.contains("terrain: the ground is Skyrim's collision shapes now (gmsky_ground shapes)")
          && server_log.contains("has arrived: released"),
          "gmod_server.log: shapes clear, switch and release logged");
    check(server_log.matches("ERROR").count() == 1 && server_log.contains("another process (pid 424242)"),
          "gmod_server.log: exactly one ERROR, the expected takeover message");
    let client_log = std::fs::read_to_string(log_dir.join("gmod_client.log")).unwrap_or_default();
    check(client_log.contains("client frames in last 5 s: 200 (40 fps) fps_max_nofocus=(no such setting)"),
          "gmod_client.log: counted 200 frames");
    check(client_log.contains("fps_max_nofocus=300"), "gmod_client.log: fps_max_nofocus logged");
    check(client_log.contains("ran gmod/lua/gmsky/terrain.lua"), "gmod_client.log: terrain.lua runs on the client");
    check(client_log.contains("ran gmod/lua/gmsky/overlay.lua") && client_log.contains("graphics: ") && client_log.contains(" modules loaded; Vulkan "),
          "gmod_client.log: overlay.lua runs on the client and logs the graphics DLLs");
    check(client_log.contains("ran gmod/lua/gmsky/overlay2d.lua"), "gmod_client.log: overlay2d.lua runs on the client");
    check(client_log.contains("ran gmod/lua/gmsky/liveoverlay.lua")
          && client_log.contains("overlay probe (") && client_log.contains("mat_queue_mode ?, screen 1280x720): hook NOT installed"),
          "gmod_client.log: liveoverlay.lua runs on the client and the probe logs why it couldn't hook");
    check(client_log.contains("live overlay ON (mat_queue_mode ?, screen 1280x720): hook NOT installed")
          && client_log.contains("live overlay first frame: cleared at view model, hook order PostRender, DrawOverlay")
          && client_log.contains("live overlay OFF"),
          "gmod_client.log: gmsky_overlay on / first frame / off logged");
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
