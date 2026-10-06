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
-- gmsky_overlay_hidewindow 0: the harness never asks to move a window (step 2d-2).
-- gmsky_overlay 0: the overlay (on by default since test 67) runs only where a check turns it on.
-- gmsky_render 0: Garry's Mod's objects go to Skyrim only where a check turns it on (step G3b).
HARNESS_CVARS = { gmsky_ground = "grid", gmsky_overlay_hidewindow = "0", gmsky_overlay = "0", gmsky_render = "0" }
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
	function c:GetButtons() return HARNESS_OWN_BUTTONS or 0 end
	function c:SetMouseWheel(n) self.wheel = n end
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
function ply:GetAimVector() return Vector(1, 0, 0) end
function ply:GetClass() return "player" end
function ply:GetHull() return Vector(-16, -16, 0), Vector(16, 16, 72) end
function ply:GetHullDuck() return Vector(-16, -16, 0), Vector(16, 16, 36) end
-- Weapons (step 3a): HARNESS_WEAPONS = list made with HARNESS_WEAPON, HARNESS_ACTIVE = held one.
function HARNESS_WEAPON(class, slot, pos)
	return { GetClass = function() return class end, GetSlot = function() return slot end,
		GetSlotPos = function() return pos end, GetOwner = function() return ply end }
end
function ply:GetWeapons() return HARNESS_WEAPONS or {} end
function ply:GetActiveWeapon() return HARNESS_ACTIVE end
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
        // Phase 3 step 3a (rebuilt after the user's feedback): the client turns Skyrim's clicks,
        // wheel, 1-6 and Q into Garry's Mod's own binds (PlayerBindPress first, then the console
        // command); a menu cursor takes the mouse; the server takes attack from the command.
        {
            use gmsky_protocol::{IN_ATTACK, IN_CONSOLE, IN_MENU, IN_SLOT_SHIFT};
            lua.run(lc, r#"
                HARNESS_CMDS, HARNESS_GUI, HARNESS_POLYS = {}, {}, 0
                function HARNESS_PLY:ConCommand(c) HARNESS_CMDS[#HARNESS_CMDS + 1] = c end
                local function g(name) return function(a, b) HARNESS_GUI[#HARNESS_GUI + 1] = name .. tostring(a) .. (b and ("," .. b) or "") end end
                gui = { InternalCursorMoved = g("move"), InternalMousePressed = g("down"), InternalMouseReleased = g("up"), InternalMouseWheeled = g("wheel") }
                vgui = { CursorVisible = function() return HARNESS_CURSOR or false end }
                draw = { NoTexture = function() end }
                surface = { SetDrawColor = function() end, DrawPoly = function(p) HARNESS_POLYS = HARNESS_POLYS + 1 end }
                hook.Add("PlayerBindPress", "harness", function(p, bind) if HARNESS_TAKE == bind then return true end end)
            "#).unwrap();
            let mut frame = 12u64;
            let mut mx = 1300i64;
            let mut cstep = |buttons: u32, dmx: i64, wheel: i32, pre: &str| {
                frame += 1;
                mx += dmx;
                sky.shared().input.write(&Input { frame, active: 1, buttons, mouse_x_total: mx, wheel_total: wheel, noclip_presses: 8, ..Default::default() });
                lua.run(lc, &format!("HARNESS_CMDS, HARNESS_GUI = {{}}, {{}} {pre} hook.Run('Think') \
                    return table.concat(HARNESS_CMDS, ' ') .. '|' .. table.concat(HARNESS_GUI, ' ')"))
            };
            macro_rules! expect {
                ($r:expr, $want:expr, $what:expr) => {{
                    let r = $r;
                    check(r == Ok(Some(String::from($want))), &format!("client binds: {}: {r:?}", $what));
                }};
            }
            expect!(cstep(0, 0, 0, ""), "|", "first frame only sets the baseline");
            expect!(cstep(0, 0, -1, ""), "invnext|", "wheel down = invnext");
            expect!(cstep(0, 0, 1, ""), "invprev invprev|", "wheel up 2 notches = invprev twice");
            expect!(cstep(1 << IN_SLOT_SHIFT, 0, 1, ""), "slot1|", "key 1 = slot1");
            expect!(cstep(1 << IN_SLOT_SHIFT, 0, 1, ""), "|", "key 1 held: once only");
            expect!(cstep(3 << IN_SLOT_SHIFT, 0, 1, ""), "slot3|", "key 3 = slot3");
            expect!(cstep(IN_ATTACK, 0, 1, ""), "+attack|", "left button = +attack");
            expect!(cstep(0, 0, 1, ""), "-attack|", "released = -attack");
            expect!(cstep(IN_ATTACK, 0, 1, "HARNESS_TAKE = '+attack'"), "|", "a hook (weapon menu) takes +attack: no command");
            expect!(cstep(0, 0, 1, "HARNESS_TAKE = nil"), "|", "and no -attack after it");
            expect!(cstep(IN_MENU, 0, 1, ""), "+menu|", "Q = +menu (spawn menu)");
            expect!(cstep(IN_MENU, 40, 1, "HARNESS_CURSOR = true"), "|move640,360 move680,360", "menu cursor up: starts in the middle, Skyrim's mouse moves it");
            let r = lua.run(ls, "local f, c = gmsky.ClientInput() return tostring(f) .. ' ' .. tostring(c)");
            check(r == Ok(Some("true true".into())), &format!("server hears: client handles binds, cursor up: {r:?}"));
            expect!(cstep(IN_MENU | IN_ATTACK, 0, 3, ""), "|down107 wheel2", "click and wheel go to the menu, no +attack");
            let r = lua.run(lc, "hook.Run('DrawOverlay') return HARNESS_POLYS");
            check(r == Ok(Some("2".into())), &format!("cursor arrow drawn (outline + fill): {r:?}"));
            expect!(cstep(IN_MENU, 0, 3, ""), "|up107", "button released in the menu");
            expect!(cstep(0, 0, 3, "HARNESS_CURSOR = false"), "-menu|", "Q released = -menu, cursor gone");
            lua.run(lc, "HARNESS_POLYS = 0 hook.Run('DrawOverlay')").unwrap();
            check(lua.run(lc, "return HARNESS_POLYS") == Ok(Some("0".into())), "no cursor arrow without a menu cursor");
            lua.run(lc, "HARNESS_ACTIVE = HARNESS_WEAPON('weapon_physgun', 0, 2)").unwrap();
            expect!(cstep(IN_ATTACK, 0, 3, ""), "+attack|", "physgun: +attack");
            expect!(cstep(IN_ATTACK, 0, 5, ""), "|", "physgun holding: the wheel isn't turned into weapon binds");
            expect!(cstep(0, 0, 5, "HARNESS_ACTIVE = nil"), "-attack|", "physgun released");
            // F1 (after test 62): Garry's Mod's console; closes when it is closed or the window loses the front.
            lua.run(lc, "HARNESS_RT, HARNESS_FOCUS, HARNESS_CONSOLE = 0, true, false RealTime = function() return HARNESS_RT end                 system = { HasFocus = function() return HARNESS_FOCUS end } gui.IsConsoleVisible = function() return HARNESS_CONSOLE end                 HARNESS_REAL_RCC = RunConsoleCommand RunConsoleCommand = function(c) HARNESS_CMDS[#HARNESS_CMDS + 1] = 'run:' .. c end").unwrap();
            expect!(cstep(IN_CONSOLE, 0, 5, ""), "|", "F1 = Garry's Mod's console (its key, posted by the module)");
            expect!(cstep(IN_CONSOLE, 0, 5, "HARNESS_CONSOLE = true"), "|", "F1 held: once only; console showing");
            expect!(cstep(0, 0, 5, "HARNESS_CONSOLE = false"), "|", "console closed in Garry's Mod");
            check(lua.run(lc, "return tostring(gmsky.console.open)") == Ok(Some("false".into())), "console: closed when Garry's Mod's console closes");
            expect!(cstep(IN_CONSOLE, 0, 5, "HARNESS_CONSOLE = false"), "|", "F1 again");
            expect!(cstep(0, 0, 5, "HARNESS_CONSOLE = true HARNESS_RT = 2 HARNESS_FOCUS = false"), "run:hideconsole|", "clicked back into Skyrim: console hidden");
            check(lua.run(lc, "return tostring(gmsky.console.open)") == Ok(Some("false".into())), "console: closed when Garry's Mod's window loses the front");
            lua.run(lc, "HARNESS_CONSOLE = false RunConsoleCommand = HARNESS_REAL_RCC").unwrap();
            // Server: attack from the player's own command while the client handles binds.
            let mut sframe = 40u64;
            let mut srv = |buttons: u32| {
                sframe += 1;
                sky.shared().input.write(&Input { frame: sframe, active: 1, buttons, mouse_x_total: 1300, noclip_presses: 8, ..Default::default() });
                run_cmd("return tostring(HARNESS_CMD.buttons) .. ' ' .. tostring(HARNESS_CMD.wheel)")
            };
            lua.run(lc, "gmsky.SetClientInput(false)").unwrap();
            lua.run(ls, "HARNESS_OWN_BUTTONS = 0").unwrap();
            let r = srv(IN_ATTACK);
            check(r == Ok(Some("0 nil".into())), &format!("server: Skyrim's button alone doesn't fire (the client's binds decide): {r:?}"));
            lua.run(ls, "HARNESS_OWN_BUTTONS = 1").unwrap();
            let r = srv(0);
            check(r == Ok(Some("1 nil".into())), &format!("server: +attack in the command fires: {r:?}"));
            lua.run(lc, "gmsky.SetClientInput(nil)").unwrap();
            lua.run(ls, "HARNESS_OWN_BUTTONS = 0").unwrap();
            let r = srv(IN_ATTACK);
            check(r == Ok(Some("1 nil".into())), &format!("server: client silent: falls back to Skyrim's button: {r:?}"));
            // Physgun wheel on the server, and no view turning while a menu cursor is up.
            lua.run(ls, "HARNESS_ACTIVE = HARNESS_WEAPON('weapon_physgun', 0, 2)").unwrap();
            sky.shared().input.write(&Input { frame: 50, active: 1, buttons: IN_ATTACK, mouse_x_total: 1300, wheel_total: 0, noclip_presses: 8, ..Default::default() });
            run_cmd("").unwrap();
            sky.shared().input.write(&Input { frame: 51, active: 1, buttons: IN_ATTACK, mouse_x_total: 1300, wheel_total: -2, noclip_presses: 8, ..Default::default() });
            let r = run_cmd("return tostring(HARNESS_CMD.wheel)");
            check(r == Ok(Some("-2".into())), &format!("server: physgun holding gets the wheel: {r:?}"));
            lua.run(ls, "HARNESS_ACTIVE = nil").unwrap();
            lua.run(lc, "gmsky.SetClientInput(true)").unwrap();
            let before = run_cmd("return tostring(HARNESS_CMD.y)");
            sky.shared().input.write(&Input { frame: 52, active: 1, mouse_x_total: 1800, wheel_total: -2, noclip_presses: 8, ..Default::default() });
            let after = run_cmd("return tostring(HARNESS_CMD.y)");
            check(before.is_ok() && before == after, &format!("server: menu cursor up: the view doesn't turn ({before:?} -> {after:?})"));
            let cursor_flag = || {
                lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
                sky.shared().player_state.read().map(|(p, _)| p.flags & gmsky_protocol::PF_CURSOR != 0)
            };
            check(cursor_flag() == Some(true), "server: menu cursor up: Skyrim is told (no camera prediction)");
            lua.run(lc, "gmsky.SetClientInput(false)").unwrap();
            sky.shared().input.write(&Input { frame: 53, active: 1, mouse_x_total: 2300, wheel_total: -2, noclip_presses: 8, ..Default::default() });
            let turned = run_cmd("return tostring(HARNESS_CMD.y)");
            check(turned.is_ok() && turned != after, &format!("server: cursor gone: the same mouse move turns the view ({after:?} -> {turned:?})"));
            check(cursor_flag() == Some(false), "server: cursor gone: Skyrim may predict again");
            // Q held counts as a cursor from the first tick, before the client has seen the menu.
            lua.run(lc, "gmsky.SetClientInput(nil)").unwrap();
            sky.shared().input.write(&Input { frame: 54, active: 1, buttons: IN_MENU, mouse_x_total: 2800, wheel_total: -2, noclip_presses: 8, ..Default::default() });
            let q = run_cmd("return tostring(HARNESS_CMD.y)");
            check(q.is_ok() && q == turned, &format!("server: Q held, client not reporting yet: the view doesn't turn ({turned:?} -> {q:?})"));
            check(cursor_flag() == Some(true), "server: Q held: Skyrim is told at once");
            sky.shared().input.write(&Input { frame: 55, active: 1, mouse_x_total: 2800, wheel_total: -2, noclip_presses: 8, ..Default::default() });
            run_cmd("").unwrap();
            check(cursor_flag() == Some(false), "server: Q released: flag cleared");
            lua.run(ls, "HARNESS_OWN_BUTTONS = nil").unwrap();
        }
        sky.shared().input.write(&Input { frame: 30, active: 0, mouse_x_total: 1300, noclip_presses: 8, ..Default::default() });
        run_cmd("").unwrap();
        let r = lua.run(lc, "return tostring(HARNESS_CONVARS)");
        check(r == Ok(Some("nil".into())), &format!("client runs no console commands: {r:?}"));
        lua.run(lc, "HARNESS_CONVARS = { fps_max_nofocus = '300' } timer.Run('gmsky_client_status')").unwrap();

        // 8a. Skyrim's actors near the player (Phase 3 step 3b) reach Lua in Garry's Mod's world
        //     units (the anchor added), nearest first; a table for another origin is refused.
        {
            use gmsky_protocol::{Actor, Actors, ACT_HOSTILE, ACT_IN_COMBAT};
            let mut t = Box::new(Actors::default());
            t.count = 2;
            t.origin_epoch = 1;
            t.skyrim_frame = 77;
            let mut name = [0u8; 32];
            name[..6].copy_from_slice(b"Bandit");
            t.actors[0] = Actor { form_id: 0x0001A2B3, flags: ACT_HOSTILE | ACT_IN_COMBAT, pos: [100.0, 0.0, 0.0], yaw_deg: 90.0,
                radius: 18.0, height: 70.0, health_frac: 0.5, level: 9, reserved0: 0, name };
            let mut name2 = [0u8; 32];
            name2[..5].copy_from_slice(b"Guard");
            t.actors[1] = Actor { form_id: 0x000FF001, flags: 0, pos: [0.0, 300.0, 10.0], yaw_deg: -45.0,
                radius: 18.0, height: 70.0, health_frac: 1.0, level: 20, reserved0: 0, name: name2 };
            sky.shared().actors.write(&t);
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local A = gmsky.actors local ax, ay, az = gmsky.Anchor() local b = A.list[1] \
                return string.format('%d %s %.1f %.1f %.1f %.0f %s %d %.2f %s', #A.list, b.name, b.x - ax, b.y - ay, b.z - az, b.yaw, \
                A.FlagText(b.flags), b.level, b.hp, A.list[2].name)");
            check(r == Ok(Some("2 Bandit 100.0 0.0 0.0 90 hostile fighting 9 0.50 Guard".into())), &format!("actors: table read, anchor added: {r:?}"));
            let r = lua.run(ls, "local seq = gmsky.ReadActors(gmsky.actors.seq) return tostring(seq)");
            check(r == Ok(Some("false".into())), &format!("actors: nothing new since the last read: {r:?}"));
            let r = lua.run(ls, "return gmsky.actors.Status(HARNESS_PLY)");
            check(r.as_ref().map_or(false, |v| v.as_deref().map_or(false, |v| v.contains("actors 2 (tables read 1, +2 -0): Bandit 0001A2B3 lvl 9 at "))),
                &format!("actors: status line: {r:?}"));
            t.origin_epoch = 2;
            sky.shared().actors.write(&t);
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "return #gmsky.actors.list");
            check(r == Ok(Some("0".into())), &format!("actors: a table for another origin is refused: {r:?}"));
        }

        // 8a-2. Skyrim's hitboxes (Phase 3 step 3c-2): a stand-in per actor, traces answered from
        //       the body parts, damage logged with the part, removed when Skyrim stops sending it.
        {
            use gmsky_protocol::{HitActor, HitPart, Hitboxes, HBF_GEAR, HBG_CHEST, HBG_GEAR, HBG_HEAD, HBK_BOX, HBK_CAPSULE};
            let mut t = Box::new(Hitboxes::default());
            t.actor_count = 1;
            t.part_count = 3;
            t.origin_epoch = 1;
            t.actors[0] = HitActor { form_id: 0xFF00ABCD, first_part: 0, part_count: 3, flags: 0, reserved0: 0 };
            // Relative to the anchor: a body capsule, a head sphere, a bow in front (gear).
            t.parts[0] = HitPart { a: [0.0, 0.0, 10.0], b: [0.0, 0.0, 50.0], r: 8.0, kind: HBK_CAPSULE, group: HBG_CHEST, flags: 0, bone: 0 };
            t.parts[1] = HitPart { a: [0.0, 0.0, 64.0], b: [0.0, 0.0, 64.0], r: 5.0, kind: HBK_CAPSULE, group: HBG_HEAD, flags: 0, bone: 1 };
            t.parts[2] = HitPart { a: [-20.0, -5.0, 20.0], b: [-15.0, 5.0, 60.0], r: 0.0, kind: HBK_BOX, group: HBG_GEAR, flags: HBF_GEAR, bone: 2 };
            sky.shared().hitboxes.write(&t);
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local n, e = 0 for id, x in pairs(gmsky.standins.ents) do n = n + 1 e = x end HARNESS_SI = e \
                return string.format('%d %s %08X', n, e and e.class or '-', e and e:FormId() or 0)");
            check(r == Ok(Some("1 gmsky_actor FF00ABCD".into())), &format!("stand-ins: one gmsky_actor for the actor (formId above 2^31 kept): {r:?}"));
            // A bullet at head height, through the real entity code.
            let r = lua.run(ls, "local ax, ay, az = gmsky.Anchor() \
                local tr = HARNESS_SI:TestCollision(Vector(ax - 100, ay, az + 64), Vector(200, 0, 0), false, Vector(0, 0, 0)) \
                local g = gmsky.HitPartAt(HARNESS_SI:FormId(), tr.HitPos.x, tr.HitPos.y, tr.HitPos.z) \
                return string.format('%.1f %s %.0f', tr.HitPos.x - ax, gmsky.standins.GroupName(g), tr.Normal.x)");
            check(r == Ok(Some("-5.0 head -1".into())), &format!("stand-ins: a bullet hits the head: {r:?}"));
            let r = lua.run(ls, "local ax, ay, az = gmsky.Anchor() \
                local tr = HARNESS_SI:TestCollision(Vector(ax - 100, ay, az + 30), Vector(200, 0, 0), false, Vector(0, 0, 0)) \
                return string.format('%.1f', tr.HitPos.x - ax)");
            check(r == Ok(Some("-8.0".into())), &format!("stand-ins: the bow is gear: the bullet passes it and hits the chest: {r:?}"));
            let r = lua.run(ls, "local ax, ay, az = gmsky.Anchor() \
                return tostring((HARNESS_SI:TestCollision(Vector(ax - 100, ay, az + 58.5), Vector(200, 0, 0), false, Vector(0, 0, 0))))");
            check(r == Ok(Some("nil".into())), &format!("stand-ins: a shot between head and shoulders misses: {r:?}"));
            // The player's box (startpos = its bottom, extents 16 x 16 x 36) bumps into the body capsule.
            let r = lua.run(ls, "local ax, ay, az = gmsky.Anchor() \
                local tr = HARNESS_SI:TestCollision(Vector(ax - 100, ay, az), Vector(200, 0, 0), true, Vector(16, 16, 36)) \
                local off = HARNESS_CVARS.gmsky_actor_solid HARNESS_CVARS.gmsky_actor_solid = '0' \
                local none = HARNESS_SI:TestCollision(Vector(ax - 100, ay, az), Vector(200, 0, 0), true, Vector(16, 16, 36)) \
                HARNESS_CVARS.gmsky_actor_solid = off \
                return string.format('%.1f %s', tr.HitPos.x - ax, tostring(none))");
            check(r == Ok(Some("-24.0 nil".into())), &format!("stand-ins: the player bumps into the hull; gmsky_actor_solid 0 walks through: {r:?}"));
            // An NPC walked into the player (test 58): the player, 4 units inside the hull (around the
            // parts here: x -8..8), is shoved out along -x at 4 / 0.08 = 50 units/s; clear of it, nothing.
            let r = lua.run(ls, "local ax, ay, az = gmsky.Anchor() \
                local function run(x) local v = Vector(0, 0, 0) local mv = { GetOrigin = function() return Vector(ax + x, ay, az) end, \
                    GetVelocity = function() return v end, SetVelocity = function(_, nv) v = nv end, SetOrigin = function() end } \
                    hook.Run('SetupMove', HARNESS_PLY, mv) return v end \
                local held = gmsky.terrain.holding gmsky.terrain.holding = nil                 local a, b = run(-20), run(-30) gmsky.terrain.holding = held \
                return string.format('%.1f %.1f %.1f', a.x, a.y, b.x)");
            check(r == Ok(Some("-50.0 0.0 0.0".into())), &format!("stand-ins: an NPC walking into the player shoves them out: {r:?}"));
            // Damage on the stand-in: logged with the body part.
            lua.run(ls, "local ax, ay, az = gmsky.Anchor() HARNESS_SI:OnTakeDamage({ \
                GetDamagePosition = function() return Vector(ax - 5, ay, az + 64) end, GetDamage = function() return 75 end, \
                GetDamageType = function() return 2 end, GetAttacker = function() return HARNESS_PLY end, \
                GetInflictor = function() return HARNESS_PLY end, GetDamageForce = function() return Vector(300, 0, 0) end })").unwrap();
            // Step 3d: the hit is in the queue for Skyrim: 75 x 3 (gmsky_damage_scale) x 2 (head).
            {
                let ev = &sky.shared().hit_events;
                let head = ev.head.load(std::sync::atomic::Ordering::Acquire);
                let e = ev.get(head.wrapping_sub(1));
                check(head >= 1 && e.form_id == 0xFF00ABCD && (e.damage - 450.0).abs() < 1e-3 && (e.gmod_damage - 75.0).abs() < 1e-3
                        && e.group == 1 && e.kind == gmsky_protocol::HITK_BULLET && e.dmg_type == 2 && e.origin_epoch == 1
                        && (e.pos[0] + 5.0).abs() < 1e-3 && e.pos[1].abs() < 1e-3 && (e.pos[2] - 64.0).abs() < 1e-3
                        && (e.dir[0] - 1.0).abs() < 1e-6 && (e.force - 900.0).abs() < 1e-3,
                    &format!("hit event for Skyrim: head {head}, {e:?}"));
                // An explosion: no body part, x3 only.
                lua.run(ls, "local ax, ay, az = gmsky.Anchor() HARNESS_SI:OnTakeDamage({ \
                    GetDamagePosition = function() return Vector(ax - 5, ay, az + 64) end, GetDamage = function() return 40 end, \
                    GetDamageType = function() return 64 end, GetAttacker = function() return HARNESS_PLY end, \
                    GetInflictor = function() return HARNESS_PLY end, GetDamageForce = function() return Vector(0, 0, 0) end })").unwrap();
                let e = ev.get(ev.head.load(std::sync::atomic::Ordering::Acquire).wrapping_sub(1));
                check(e.kind == gmsky_protocol::HITK_EXPLOSION && e.group == 0 && e.bone == 255 && (e.damage - 120.0).abs() < 1e-3,
                    &format!("hit event: an explosion has no body part and isn't a head shot: {e:?}"));
            }
            let r = lua.run(ls, "return gmsky.standins.Status(HARNESS_PLY)");
            check(r.as_ref().map_or(false, |v| v.as_deref().map_or(false, |v| v.contains("stand-ins 1 (+1 -0") && v.contains("damage taken 2 (sent to Skyrim 2)"))),
                &format!("stand-ins: status line: {r:?}"));
            // Step 3e: Skyrim's NPCs hurt the player. The harness plays Skyrim: two hurt events (the
            // stand-in's power attack, then magic from an actor with no stand-in) become Garry's Mod
            // damage on a fake player; a burst of 35 loses the 3 oldest.
            {
                use gmsky_protocol::{HurtEvent, HURTK_MELEE, HURTK_MAGIC, HURTF_POWER};
                lua.run(ls, "gmsky.ReadHurts()").unwrap(); // take Skyrim's position
                let ring = &sky.shared().hurt_events;
                ring.push(&HurtEvent { attacker: 0xFF00ABCD, damage: 12.0, source: 0x12EB7, kind: HURTK_MELEE, flags: HURTF_POWER, reserved: 0 });
                ring.push(&HurtEvent { attacker: 0x00000123, damage: 4.0, source: 0, kind: HURTK_MAGIC, flags: 0, reserved: 0 });
                let r = lua.run(ls, "DamageInfo = function() local d = {} for _, k in ipairs({ 'Damage', 'DamageType', 'Attacker', 'Inflictor', \
                        'DamagePosition', 'DamageForce' }) do d['Set' .. k] = function(self, v) self[k] = v end end return d end \
                    local hp, got, vel = 100, {}, nil \
                    local P = { Health = function() return hp end, Armor = function() return 0 end, Alive = function() return true end, \
                        WorldSpaceCenter = function() return HARNESS_SI:GetPos() + Vector(100, 0, 0) end, \
                        TakeDamageInfo = function(_, d) got[#got + 1] = d hp = hp - d.Damage end, SetVelocity = function(_, v) vel = v end } \
                    gmsky.hurt.Update(P) \
                    local a, b = got[1], got[2] \
                    return string.format('%d %.1f %d %s %.0f %.1f %d %s %d', #got, a.Damage, a.DamageType, tostring(a.Attacker == HARNESS_SI), \
                        vel and vel.x or -1, b.Damage, b.DamageType, tostring(b.Attacker), hp)");
                check(r == Ok(Some("2 12.0 4 true 300 4.0 8 world 84".into())),
                    &format!("hurt: Skyrim's hits become damage from the stand-in (slash, power attack shoves) and magic from the world: {r:?}"));
                for i in 0..35 {
                    ring.push(&HurtEvent { attacker: 0, damage: i as f32, source: 0, kind: 3, flags: 0, reserved: 0 });
                }
                let r = lua.run(ls, "local list, lost = gmsky.ReadHurts() return #list .. ' ' .. lost .. ' ' .. list[1].damage \
                    .. ' ' .. #gmsky.ReadHurts()");
                check(r == Ok(Some("32 3 3 0".into())), &format!("hurt: a burst of 35 keeps the newest 32, 3 lost, nothing twice: {r:?}"));
                let r = lua.run(ls, "return gmsky.hurt.Status()");
                check(r.as_ref().map_or(false, |v| v.as_deref().map_or(false, |v| v.contains("hurt 2 times (16.0 Skyrim -> 16.0 Garry's Mod damage)"))),
                    &format!("hurt: status line: {r:?}"));
            }
            // Step 3f: the calm spot is kept while Skyrim says nobody fights the player and not
            // during a fight; dying and respawning puts the player back on it.
            {
                let mut st = gmsky_protocol::SkyrimState::default();
                st.frame = 5;
                sky.shared().skyrim_state.write(&st);
                let r = lua.run(ls, "Angle = Angle or function(p, y, r) return { p = p, y = y, r = r } end \
                    local R = gmsky.respawn R.calm, R.nextAt, gmsky.hurt.lastHurt = nil, 0, nil HARNESS_TIME = 100 \
                    local held = gmsky.terrain and gmsky.terrain.holding if gmsky.terrain then gmsky.terrain.holding = false end \
                    local _, epoch = gmsky.Status() R.Update(HARNESS_PLY, epoch) \
                    if gmsky.terrain then gmsky.terrain.holding = held end \
                    local ax, ay, az = gmsky.Anchor() local p, c = HARNESS_PLY:GetPos(), R.calm \
                    return c and string.format('%s %d', tostring(math.abs(c.rel.x - (p.x - ax)) < 1e-6 and math.abs(c.rel.z - (p.z - az)) < 1e-6), c.epoch) or 'none'");
                check(r == Ok(Some("true 1".into())), &format!("respawn: calm spot taken while nobody fights the player: {r:?}"));
                st.flags = gmsky_protocol::SF_IN_COMBAT;
                st.frame = 6;
                sky.shared().skyrim_state.write(&st);
                let r = lua.run(ls, "local R = gmsky.respawn local before = R.calm.time HARNESS_TIME = 101 R.nextAt = 0 \
                    local held = gmsky.terrain and gmsky.terrain.holding if gmsky.terrain then gmsky.terrain.holding = false end \
                    local _, epoch = gmsky.Status() R.Update(HARNESS_PLY, epoch) \
                    if gmsky.terrain then gmsky.terrain.holding = held end \
                    return tostring(R.calm.time == before) .. ' ' .. tostring(R.fighting)");
                check(r == Ok(Some("true true".into())), &format!("respawn: calm spot kept during a fight: {r:?}"));
                let r = lua.run(ls, "local R = gmsky.respawn local ax, ay, az = gmsky.Anchor() HARNESS_TIME = 105 \
                    local saved = HARNESS_POS HARNESS_POS = Vector(ax + 500, ay - 300, az + 20) \
                    hook.Run('PlayerDeath', HARNESS_PLY, nil, nil) HARNESS_POS = Vector(-96, 96, -16259) \
                    HARNESS_SETPOS = nil hook.Run('PlayerSpawn', HARNESS_PLY) \
                    local t, c = HARNESS_SETPOS, R.calm HARNESS_POS = saved \
                    return string.format('%s %.0f', tostring(t ~= nil and math.abs(t.x - (ax + c.rel.x)) < 1e-6 and math.abs(t.y - (ay + c.rel.y)) < 1e-6 \
                        and math.abs(t.z - (az + c.rel.z)) < 1e-6), HARNESS_EYE.y)");
                check(r == Ok(Some("true 45".into())), &format!("respawn: dying then respawning puts the player on the calm spot: {r:?}"));
                st.flags = 0;
                st.frame = 7;
                sky.shared().skyrim_state.write(&st);
            }
            // Step 4b: Skyrim's water. The harness plays Skyrim: a surface 100 units above the
            // anchor; Source's water movement takes over from waist deep (sinks with no keys, Jump
            // swims up), and not above the water.
            {
                let mut st = gmsky_protocol::SkyrimState::default();
                st.frame = 8;
                st.flags = gmsky_protocol::SF_WATER;
                st.water_z = 100.0;
                sky.shared().skyrim_state.write(&st);
                lua.run(ls, "FrameTime = function() return 0.015 end \
                    util.TraceHull = function(t) return { Hit = false, HitPos = t.endpos, Fraction = 1 } end \
                    local function A() return { Forward = function() return Vector(1, 0, 0) end, Right = function() return Vector(0, -1, 0) end } end \
                    function HARNESS_MV(z, jump, fmove) local ax, ay, az = gmsky.Anchor() local m = { o = Vector(ax, ay, az + z), v = Vector(0, 0, 0) } \
                        function m:GetOrigin() return self.o end function m:SetOrigin(o) self.o = o end \
                        function m:GetVelocity() return self.v end function m:SetVelocity(v) self.v = v end \
                        function m:GetMoveAngles() return A() end function m:GetForwardSpeed() return fmove or 0 end \
                        function m:GetSideSpeed() return 0 end function m:GetUpSpeed() return 0 end \
                        function m:GetMaxSpeed() return 200 end function m:KeyDown(k) return jump and k == IN_JUMP end return m end").unwrap();
                let r = lua.run(ls, "local held = gmsky.terrain and gmsky.terrain.holding if gmsky.terrain then gmsky.terrain.holding = false end                     local ax, ay, az = gmsky.Anchor() local w = gmsky.WaterZ() \
                    local m = HARNESS_MV(0, false) local took = hook.Run('Move', HARNESS_PLY, m) \
                    local j = HARNESS_MV(0, true) hook.Run('Move', HARNESS_PLY, j) \
                    local dry = HARNESS_MV(120, true) local tookDry = hook.Run('Move', HARNESS_PLY, dry)                     if gmsky.terrain then gmsky.terrain.holding = held end \
                    return string.format('%.1f %s %.1f %.1f %s %d %d', w - az, tostring(took), m.v.z, j.v.z, tostring(tookDry), \
                        gmsky.water.Level(HARNESS_PLY, az + 50, w), gmsky.water.Level(HARNESS_PLY, az + 70, w))");
                check(r == Ok(Some("100.0 true -7.2 24.0 nil 2 1".into())),
                    &format!("water: surface from Skyrim; under water: sinks with no keys, Jump swims up; above: Garry's Mod's own movement; waist / feet levels: {r:?}"));
                // After test 67: W (10,000 asked, as Garry's Mod sends it) + Space swims up and forward alike.
                let r = lua.run(ls, "local held = gmsky.terrain and gmsky.terrain.holding if gmsky.terrain then gmsky.terrain.holding = false end                     local m = HARNESS_MV(0, true, 10000) hook.Run('Move', HARNESS_PLY, m)                     if gmsky.terrain then gmsky.terrain.holding = held end return string.format('%.1f %.1f', m.v.x, m.v.z)");
                check(r == Ok(Some("17.0 17.0".into())),
                    &format!("water: W + Space swims forward and up at the same speed (keys scaled to the top speed first): {r:?}"));
                let r = lua.run(ls, "return gmsky.water.Status()");
                check(r.as_ref().map_or(false, |v| v.as_deref().map_or(false, |v| v.contains("water surface z") && v.contains("swam 3 ticks"))),
                    &format!("water: status line: {r:?}"));
                st.flags = 0;
                st.frame = 9;
                sky.shared().skyrim_state.write(&st);
                let r = lua.run(ls, "return tostring((gmsky.WaterZ()))");
                check(r == Ok(Some("nil".into())), &format!("water: none when Skyrim says no water: {r:?}"));
            }
            // Step 3c-3: gmsky_hitbox_test round trip. The harness plays Skyrim: it answers every ray
            // as "missed", so the report must show Garry's Mod's hits as "only Garry's Mod".
            lua.run(ls, "local real = util.TraceLine util.TraceLine = function() return { Hit = true, Entity = HARNESS_SI, HitPos = Vector() } end \
                HARNESS_SI.GetClass = function() return 'gmsky_actor' end \
                concommand._c.gmsky_hitbox_test(HARNESS_PLY) util.TraceLine = real").unwrap();
            match sky.shared().hit_test_request.read() {
                Some((req, _)) => {
                    check(req.id != 0 && req.form_id == 0xFF00ABCD && req.count == 441 && req.origin_epoch == 1,
                        &format!("hitbox test: request sent (id {}, {:08X}, {} rays, epoch {})", req.id, req.form_id, req.count, req.origin_epoch));
                    let mut res = Box::new(gmsky_protocol::HitTestResult::default());
                    res.id = req.id;
                    res.form_id = req.form_id;
                    res.count = req.count;
                    res.parts = 3;
                    for i in 0..441 {
                        res.answers[i].now_t = -1.0;
                    }
                    sky.shared().hit_test_result.write(&res);
                }
                None => check(false, "hitbox test: request sent"),
            }
            let r = lua.run(ls, "hook.Run('Tick') return tostring(HARNESS_PRINTED)");
            check(r.as_ref().map_or(false, |v| v.as_deref().map_or(false, |v| v.contains("hitbox test FF00ABCD: 441 rays over 3 parts") && v.contains(" only Garry's Mod, 0 only Skyrim"))),
                &format!("hitbox test: report after Skyrim's answer: {r:?}"));
            // Skyrim stops sending it: removed.
            t.actor_count = 0;
            t.part_count = 0;
            sky.shared().hitboxes.write(&t);
            lua.run(ls, "HARNESS_ADVANCE() hook.Run('Tick')").unwrap();
            let r = lua.run(ls, "local n = 0 for _ in pairs(gmsky.standins.ents) do n = n + 1 end return n .. ' ' .. tostring(HARNESS_SI.removed)");
            check(r == Ok(Some("0 true".into())), &format!("stand-ins: removed when Skyrim stops sending the actor: {r:?}"));
        }

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
            let r = lua.run(lc, "local saved = HARNESS_CVARS.gmsky_overlay HARNESS_CVARS.gmsky_overlay = nil                 local on = gmsky.liveoverlay.cvar:GetBool() HARNESS_CVARS.gmsky_overlay = saved return tostring(on)");
            check(r == Ok(Some("true".into())), &format!("gmsky_overlay is on by default (the user, after test 67): {r:?}"));
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
            // Step W1: the frame is wiped once, where Source starts drawing entities in the main view
            // (not in the 3D sky, a depth pass or another picture; a main view flagged "sky", as every
            // view of a 2D-sky map is (test 70), still counts); the view model then doesn't wipe again.
            // gmsky_overlay_world 0, or no main-view entity pass: wiped at the view model.
            // Step W4: alpha writes off in the see-through pass; after it, the camera is sent and the
            // depth drawn in the main view: entities with a model through DepthWrite (not effects
            // without one, our own stand-ins, the player's weapon), the reference square, the grab;
            // then the world layer is copied into its own picture and grabbed where the view model
            // starts. After test 72: solid models (and the view model) stamped alpha 1 with colour
            // writes off. With nothing to show: no camera, depth or world layer, only the wipes.
            let world_lua = r#"
                local target = nil
                local rec = function(name) return function(...) local t = {} for i = 1, select('#', ...) do local v = select(i, ...) t[i] = type(v) == "number" and string.format("%g", v) or tostring(v) end HARNESS_RENDER[#HARNESS_RENDER + 1] = name .. "(" .. table.concat(t, ",") .. ")" end end
                local function Ang(p, y, r) return { p = p, y = y, r = r, Forward = function() return Vector(1, 0, 0) end, Right = function() return Vector(0, -1, 0) end, Up = function() return Vector(0, 0, 1) end } end
                render.GetRenderTarget = function() return target end
                render.GetViewSetup = function() return { origin = Vector(10, 20, 30), angles = Ang(5, 0, 0), aspect = 16 / 9, fov = 90 } end
                render.PushRenderTarget = function(rt) target = rt HARNESS_RENDER[#HARNESS_RENDER + 1] = "push(" .. tostring(rt) .. ")" end
                render.PopRenderTarget = function() target = nil HARNESS_RENDER[#HARNESS_RENDER + 1] = "pop()" end
                render.MaterialOverride = rec("override")
                render.SetMaterial = rec("material")
                render.CopyRenderTargetToTexture = rec("copy")
                render.OverrideColorWriteEnable = rec("colorWrite")
                render.FogMode, render.FogStart, render.FogEnd, render.FogColor, render.FogMaxDensity = rec("fogMode"), rec("fogStart"), rec("fogEnd"), rec("fogColor"), rec("fogDensity")
                render.GetFogMode = function() return 0 end
                render.GetFogDistances = function() return 0, 0 end
                render.GetFogColor = function() return 0, 0, 0 end
                MATERIAL_FOG_NONE, MATERIAL_FOG_LINEAR = 0, 1
                bit = bit or { band = function(a, b) local r, k = 0, 1 while a > 0 and b > 0 do if a % 2 == 1 and b % 2 == 1 then r = r + k end a, b, k = math.floor(a / 2), math.floor(b / 2), k * 2 end return r end }
                function Material(name) return { IsError = function() return false end, GetInt = function(_, k) return (name:find("laser") and k == "$flags") and 128 or 0 end } end
                render.DrawQuadEasy = function(pos, n, w, h) HARNESS_RENDER[#HARNESS_RENDER + 1] = string.format("quad(%.1f,%.1f,%.1f,size %.2f)", pos.x, pos.y, pos.z, w) end
                function Color() return "white" end
                function GetRenderTargetEx(name, w, h, size, depth, flags, rtflags, fmt) return name .. "/" .. tostring(fmt) end
                function CreateMaterial(name, shader, kv) return name .. "/" .. shader .. "/" .. tostring(kv["$color_depth"]) end
                function ScrW() return 1280 end
                function ScrH() return 720 end
                RT_SIZE_LITERAL, MATERIAL_RT_DEPTH_SEPARATE, IMAGE_FORMAT_RGBA16161616F, MATERIAL_RT_DEPTH_NONE, IMAGE_FORMAT_BGRA8888 = 5, 1, 24, 2, 12
                local function Ent(class, model, owner) return { GetClass = function() return class end, GetModel = function() return model end,
                    IsDormant = function() return false end, GetNoDraw = function() return false end, IsWeapon = function() return class == "weapon_x" end,
                    GetOwner = function() return owner end, DrawModel = function() HARNESS_RENDER[#HARNESS_RENDER + 1] = "draw(" .. class .. ")" end } end
                local world = { Ent("prop_physics", "models/a.mdl"), Ent("class CLuaEffect", nil), Ent("gmsky_actor", "models/b.mdl"), Ent("weapon_x", "models/w.mdl", "someone") }
                local everything = {}
                ents.GetAll = function() return everything end
                local grabWorld, grabDepth, view = gmsky.OverlayGrabWorld, gmsky.OverlayGrabDepth, gmsky.OverlayView
                gmsky.OverlayGrabWorld = rec("grabWorld")
                gmsky.OverlayGrabDepth = rec("grabDepth")
                gmsky.OverlayView = rec("view")
                local function frame(world, passes)
                    HARNESS_CVARS.gmsky_overlay, HARNESS_CVARS.gmsky_overlay_world = "1", world
                    hook.Run("PreRender")
                    HARNESS_RENDER = {}
                    for _, p in ipairs(passes) do
                        target = p.rt
                        local r = hook.Run("PreDrawOpaqueRenderables", p.depth or false, p.sky or false, p.sky3d or false)
                        if r ~= nil then return "returned " .. tostring(r) end
                        hook.Run("PreDrawTranslucentRenderables", p.depth or false, p.sky or false, p.sky3d or false)
                        hook.Run("PostDrawTranslucentRenderables", p.depth or false, p.sky or false, p.sky3d or false)
                        target = nil
                    end
                    hook.Run("PreDrawViewModel") hook.Run("PostDrawViewModel", HARNESS_VM) hook.Run("PreDrawHUD") hook.Run("PostRender")
                    local clears = 0
                    for _, c in ipairs(HARNESS_RENDER) do if c:find("^Clear") then clears = clears + 1 end end
                    return tostring(gmsky.liveoverlay.frame.cleared) .. " x" .. clears
                end
                gmsky.liveoverlay.wipes = {}
                gmsky.liveoverlay.lastActivity = nil
                local out = {
                    frame("1", { { sky3d = true, sky = true }, { depth = true }, { rt = "reflection" }, { sky = true } }),
                    frame("1", { { sky3d = true, sky = true } }),
                    frame("0", { {} }),
                }
                local wipes = {}
                for k, v in pairs(gmsky.liveoverlay.wipes) do wipes[#wipes + 1] = k .. "=" .. v end
                table.sort(wipes)
                everything = world
                HARNESS_VM = Ent("viewmodel", "models/v.mdl")
                local subs = {}
                HARNESS_VM.GetMaterials = function() return { "models/rpg/body", "models/rpg/laser" } end
                HARNESS_VM.GetSubMaterial = function(_, i) return subs[i] or "" end
                HARNESS_VM.SetSubMaterial = function(_, i, m) subs[i] = m HARNESS_RENDER[#HARNESS_RENDER + 1] = "sub(" .. i .. "," .. tostring(m) .. ")" end
                -- Drawing the view model runs its "drawn" hook again, as in the game (test 73's stack overflow).
                HARNESS_VM.DrawModel = function(self) HARNESS_RENDER[#HARNESS_RENDER + 1] = "draw(viewmodel)" hook.Run("PostDrawViewModel", self) end
                frame("1", { { sky = true } })
                HARNESS_VM = nil
                local w4 = table.concat(HARNESS_RENDER, " ")
                gmsky.OverlayGrabWorld, gmsky.OverlayGrabDepth, gmsky.OverlayView = grabWorld, grabDepth, view
                everything = {}
                HARNESS_CVARS.gmsky_overlay, HARNESS_CVARS.gmsky_overlay_world = "0", nil
                render.GetRenderTarget, render.GetViewSetup, render.CopyRenderTargetToTexture, render.OverrideColorWriteEnable = nil, nil, nil, nil
                render.FogMode, render.GetFogMode, Material = nil, nil, nil
                hook.Run("PreRender")
                return table.concat(out, " / ") .. " | " .. table.concat(wipes, " ") .. " | " .. w4
            "#;
            let r = lua.run(lc, world_lua);
            // The reference square: depth pixel 4 of 640 x 360 is ndc (-0.9875, 0.9778); tan(45) = 1,
            // 1 / (16/9) = 0.5625 vertically; 1000 away: (10 + 1000, 20 + 987.5, 30 + 550.0);
            // 8 pixels = 8 * 2 * 1000 * 1 / 640 = 25 units.
            let want = "entities x2 / view model x1 / view model x1 | 3D sky pass skipped=2 depth pass skipped=1 nothing to show=1 \
                        other picture skipped=1 wiped at entities=1 wiped at view model=2 | \
                        depthAlpha(false) Clear(0,0,0,0,true,true) alphaWrite(true,false) \
                        colorWrite(true,false) alphaWrite(true,true) override(gmsky_alphastamp/UnlitGeneric/nil) draw(prop_physics) override(nil) colorWrite(false) \
                        alphaWrite(true,false) view(10,20,30,5,0,0,90,1.77778) \
                        alphaWrite(false) push(gmsky_depth_640x360/24) Clear(0,0,0,0,true,true) \
                        fogMode(1) fogStart(0) fogEnd(20000) fogColor(255,255,255) fogDensity(1) \
                        material(gmsky_depthfog/UnlitGeneric/nil) quad(510.0,513.8,305.0,size 12.50) \
                        material(gmsky_depthfog/UnlitGeneric/nil) quad(4010.0,3870.0,2230.0,size 100.00) \
                        material(gmsky_depthfog/UnlitGeneric/nil) quad(510.0,513.8,30.0,size 12.50) material(gmsky_alphastamp/UnlitGeneric/nil) quad(510.0,488.7,305.0,size 12.50) \
                        override(gmsky_depthfog/UnlitGeneric/nil) draw(prop_physics) override(nil) fogMode(0) fogStart(0) fogEnd(0) fogColor(0,0,0) grabDepth(500,4000) pop() alphaWrite(true,false) \
                        copy(gmsky_world_1280x720/12) push(gmsky_world_1280x720/12) grabWorld() pop() alphaWrite(false) Clear(0,0,0,0,false,false) \
                        colorWrite(true,false) alphaWrite(true,true) sub(0,!gmsky_alphastamp) sub(1,!gmsky_alphastamp) sub(1,!gmsky_nodraw) draw(viewmodel) alphaWrite(false) sub(0,nil) sub(1,nil) colorWrite(false) alphaWrite(false) \
                        alphaWrite(true,true) depthAlpha(true) alphaWrite(false)";
            check(r == Ok(Some(want.into())), &format!("steps W1-W4: entity-pass wipe, see-through alpha off, camera, depth of models with the reference square, world layer: {r:?}"));
            // After test 44: with the overlay on, a halo is drawn between a copy of the frame's alpha
            // and a restore of it (colour kept, alpha replaced); with gmsky_overlay_halofix 0, or
            // with the overlay off, the halo is drawn untouched.
            let halo_lua = r#"
                local rec = function(name) return function(...) local t = {} for i = 1, select('#', ...) do t[i] = tostring((select(i, ...))) end HARNESS_RENDER[#HARNESS_RENDER + 1] = name .. "(" .. table.concat(t, ",") .. ")" end end
                render.CopyRenderTargetToTexture = rec("copy")
                render.OverrideBlend = function(on, ...) HARNESS_RENDER[#HARNESS_RENDER + 1] = "blend(" .. tostring(on) .. (on and ("," .. table.concat({ ... }, ",")) or "") .. ")" end
                render.SetMaterial = rec("material")
                render.DrawScreenQuad = rec("quad")
                BLEND_ZERO, BLEND_ONE, BLENDFUNC_ADD = 0, 1, 0
                RT_SIZE_LITERAL, MATERIAL_RT_DEPTH_NONE, IMAGE_FORMAT_RGBA8888 = 5, 2, 0
                function GetRenderTargetEx(name) return { GetName = function() return name end } end
                function CreateMaterial(name) return name end
                halo = { Render = function(entry) HARNESS_RENDER[#HARNESS_RENDER + 1] = "HALO" end }
                local out = {}
                for _, step in ipairs({ { "1", "1" }, { "1", "0" }, { "0", "1" } }) do
                    HARNESS_CVARS.gmsky_overlay, HARNESS_CVARS.gmsky_overlay_halofix = step[1], step[2]
                    hook.Run("PreRender")
                    hook.Run("PreDrawViewModel")
                    HARNESS_RENDER = {}
                    halo.Render({})
                    out[#out + 1] = table.concat(HARNESS_RENDER, " ")
                    hook.Run("PreDrawHUD") hook.Run("PostRender")
                end
                HARNESS_CVARS.gmsky_overlay_halofix = nil
                hook.Run("PreRender")
                return table.concat(out, " / ")
            "#;
            let r = lua.run(lc, halo_lua);
            let want = "copy(table: X) HALO alphaWrite(true,true) blend(true,0,1,0,1,0,0) material(gmsky_alphastore_1280x720_mat) quad() blend(false) alphaWrite(false,true) / HALO / HALO";
            let got = r.clone().ok().flatten().map(|s| {
                // The render target is a table: its address changes every run.
                let i = s.find("copy(table: ").map(|i| i + 12);
                match i { Some(i) => { let j = s[i..].find(')').map(|j| i + j).unwrap_or(i); format!("{}X{}", &s[..i], &s[j..]) } None => s }
            });
            check(got.as_deref() == Some(want), &format!("halo keeps the overlay's alpha (on), untouched with halofix 0 / overlay off: {r:?}"));
            // Step 2d-2: only Source's own window (class Valve001) is ever moved; the harness has none.
            let r = lua.run(lc, "return gmsky.OverlayWindow(true) .. ' / ' .. gmsky.OverlayWindow(false)");
            check(r == Ok(Some("no Garry's Mod window found (no visible Valve001 window in this process) / Garry's Mod's window wasn't moved".into())),
                  &format!("OverlayWindow without a Garry's Mod window: {r:?}"));
            // Step 2c-1: a snapshot request without an open picture block says what to do.
            let r = lua.run(lc, "return gmsky.OverlayShot()");
            check(r == Ok(Some("no overlay block open: switch gmsky_overlay 1 on first".into())), &format!("OverlayShot without a block: {r:?}"));
            // Patched HOLOHUD 2 copy (tools/build_holohud.py), if built: its render.lua with our
            // patch appended compiles in Garry's Mod's Lua (compiled only, not run).
            let holo = root.join("build").join("holohud2_gmsky").join("lua").join("holohud2").join("render.lua");
            if let Ok(code) = std::fs::read_to_string(&holo) {
                // "do return end" first: the whole file is compiled, nothing of it runs.
                let r = lua.run(lc, &format!("do return 'compiles' end\n{code}"));
                check(r == Ok(Some("compiles".into())), &format!("patched HOLOHUD render.lua compiles: {r:?}"));
            }
        }

        // 8e. Geometry route step G3b: render.lua sends a prop's model, texture and bone matrices
        //     through the real module into the render block (read here as Skyrim would). The
        //     player's own weapon, view model, our stand-ins, hidden and far objects are left out.
        {
            use gmsky_protocol::render::*;
            let anchor = match sky.shared().player_state.read() { Some((p, _)) => p.anchor, None => [0.0; 3] };
            let skyrim = RenderMapping::open().expect("open the render block");
            skyrim.attach_reader();
            // A 4x4 BGRA8888 VTF 7.2, every pixel B=10 G=20 R=30 A=255.
            let mut vtf: Vec<u8> = b"VTF\0".to_vec();
            for v in [7u32, 2, 80] { vtf.extend_from_slice(&v.to_le_bytes()); }
            vtf.extend_from_slice(&4u16.to_le_bytes());
            vtf.extend_from_slice(&4u16.to_le_bytes());
            vtf.extend_from_slice(&[0; 32]);
            vtf.extend_from_slice(&12u32.to_le_bytes());
            vtf.push(1);
            vtf.extend_from_slice(&u32::MAX.to_le_bytes());
            vtf.extend_from_slice(&[0, 0, 1, 0, 0, 0, 0]);
            vtf.resize(80, 0);
            vtf.extend(std::iter::repeat([10u8, 20, 30, 255]).take(16).flatten());
            let vtf_lua: String = vtf.iter().map(|b| format!("\\x{b:02X}")).collect();
            let (bx, by, bz) = (anchor[0] + 10.0, anchor[1] + 20.0, anchor[2] + 30.0);
            let setup = format!(r#"
                HARNESS_CVARS.gmsky_render = "1"
                local frame = 100
                function FrameNumber() return frame end
                HARNESS_FILES = {{ ["materials/harness/box.vtf"] = "{vtf_lua}" }}
                file = {{ Read = function(name, path) HARNESS_READ_PATH = path return HARNESS_FILES[name] end }}
                local function M(t) return {{ Unpack = function() return 1, 0, 0, t[1], 0, 1, 0, t[2], 0, 0, 1, t[3], 0, 0, 0, 1 end }} end
                function Material(name)
                    return {{ IsError = function() return false end,
                        GetInt = function(_, k) return k == "$flags" and 0x100 or 0 end,
                        GetFloat = function() return 0 end,
                        GetTexture = function() return {{ IsError = function() return false end, GetName = function() return "harness/box" end }} end }}
                end
                local corner = function(x) return {{ pos = Vector(x, 0, 0), normal = Vector(0, 0, 1), u = x, v = 0, weights = {{ {{ bone = 0, weight = 1 }} }} }} end
                util.GetModelMeshes = function(model, lod, mask, skin)
                    HARNESS_MESHES_ASKED = (HARNESS_MESHES_ASKED or 0) + 1
                    return {{ {{ material = "harness/box", triangles = {{ corner(0), corner(1), corner(0) }} }} }}, {{ [0] = {{ matrix = M({{ -1, 0, 0 }}), parent = -1 }} }}
                end
                local function Ent(class, model, at, extra)
                    return {{ GetClass = function() return class end, GetModel = function() return model end, GetSkin = function() return 0 end,
                        IsDormant = function() return false end, GetNoDraw = function() return extra == "nodraw" end, IsWeapon = function() return class == "weapon_mine" end,
                        GetOwner = function() return class == "weapon_mine" and HARNESS_PLY or nil end, GetColor = function() return {{ r = 255, g = 255, b = 255, a = 255 }} end,
                        GetPos = function() return at end, SetupBones = function() end, EntIndex = function() return 42 end,
                        GetNW2Bool = function(_, k) return k == "gmsky_held" and extra == "held" end,
                        GetBoneMatrix = function(_, b) return b == 0 and M({{ at.x, at.y, at.z }}) or nil end,
                        GetWorldTransformMatrix = function() return M({{ at.x, at.y, at.z }}) end }}
                end
                local near = Vector({bx}, {by}, {bz})
                local realView = render.GetViewSetup
                render.GetViewSetup = function() return {{ origin = Vector({bx} - 5, {by}, {bz}), angles = {{ p = 5, y = 90, r = 0 }} }} end
                local realEye = HARNESS_PLY.EyePos
                HARNESS_PLY.EyePos = function() return near end
                getmetatable(near).DistToSqr = function(a, b) local d = a - b return d.x * d.x + d.y * d.y + d.z * d.z end
                ents.GetAll = function() return {{
                    Ent("prop_physics", "models/harness/box.mdl", near, "held"),
                    Ent("gmsky_actor", "models/harness/box.mdl", near),
                    Ent("viewmodel", "models/harness/box.mdl", near),
                    Ent("weapon_mine", "models/harness/box.mdl", near),
                    Ent("prop_physics", "models/harness/box.mdl", near + Vector(9000, 0, 0)),
                    Ent("prop_physics", "models/harness/box.mdl", near, "nodraw"),
                    Ent("func_brush", "*3", near),
                }} end
                hook.Run("PostDrawOpaqueRenderables", false, false, false)
                hook.Run("PostDrawOpaqueRenderables", false, false, false)  -- the same frame: not sent twice
                frame = 101
                hook.Run("PostDrawOpaqueRenderables", false, true, true)  -- the 3D sky: not sent
                hook.Run("PostDrawOpaqueRenderables", true, false, false)  -- a depth pass: not sent
                hook.Run("PostDrawOpaqueRenderables", false, true, false)  -- a 2D-sky map's main view (test 78): sent
                local line = gmsky.render.Status()
                HARNESS_CVARS.gmsky_render = "0"
                HARNESS_PLY.EyePos = realEye
                render.GetViewSetup = realView
                return HARNESS_MESHES_ASKED .. " " .. HARNESS_READ_PATH .. " | " .. line
            "#);
            let r = lua.run(lc, &setup);
            let ok = matches!(&r, Ok(Some(s)) if s.starts_with("1 GAME | render: on; 2 frames, 1.0 objects a frame, 1.0 held (gmsky_render_held 1), 0 skipped") && s.contains("models 1 ok, 0 can't be drawn") && s.contains("textures 1 (1 new, 0 failed)"));
            check(ok, &format!("render.lua: one prop wanted, its model read once, its texture from GAME: {r:?}"));
            let mut kinds = Vec::new();
            let mut frames = Vec::new();
            let mut model_ok = false;
            let mut texture_ok = false;
            skyrim.read(u64::MAX, |kind, p| {
                kinds.push(kind);
                match kind {
                    MSG_TEXTURE => {
                        let (t, px) = decode_texture(p).unwrap();
                        texture_ok = t.id == 1 && (t.width, t.height, t.format) == (4, 4, TEX_RGBA8) && px[0..4] == [30, 20, 10, 255];
                    }
                    MSG_MODEL => {
                        let m = decode_model(p).unwrap();
                        model_ok = m.model.id == 1 && m.model.bone_count == 1 && m.indices.len() == 3 && m.parts.len() == 1
                            && m.parts[0].texture_id == 1 && m.parts[0].flags == MAT_ALPHATEST && m.parts[0].alpha_ref == 0.5
                            && m.vertices[1].pos == [1.0, 0.0, 0.0] && m.vertices[1].u == 1.0 && m.vertices[1].weights[0] == 255;
                    }
                    MSG_FRAME => frames.push(decode_frame(p).unwrap()),
                    _ => {}
                }
            }).unwrap();
            check(kinds.first() == Some(&MSG_RESET) && kinds.iter().filter(|&&k| k == MSG_FRAME).count() == 2,
                  &format!("render.lua: RESET first, one FRAME per Garry's Mod frame (also on a 2D-sky map; not for the 3D sky, depth or a repeat): {kinds:?}"));
            check(texture_ok, "render.lua: the VTF reaches Skyrim as a 4x4 RGBA8 texture (BGRA swapped)");
            check(model_ok, "render.lua: the model reaches Skyrim (1 triangle, alpha-tested part on texture 1, corner on bone 0)");
            let first = frames.first().map(|(f, o, m)| (f.frame, f.origin_epoch, o.len(), o.first().map(|o| (o.model_id, o.color, o.entity, o.flags)), m.first().map(|m| [m.m[0][3], m.m[1][3], m.m[2][3]])));
            // Bone at anchor + (10, 20, 30), poseToBone moves by -1 in x: (9, 20, 30) from the anchor.
            check(first == Some((100, 1, 1, Some((1, 0xFFFF_FFFF, 42, OBJ_HELD)), Some([9.0, 20.0, 30.0]))),
                  &format!("render.lua: the frame has the held prop (entity 42) at bone x poseToBone minus the anchor, origin epoch 1: {first:?}"));
            let view = frames.first().map(|(f, _, _)| (f.flags, f.view_eye, f.view_angles));
            check(view == Some((FRAME_VIEW, [5.0, 20.0, 30.0], [5.0, 90.0, 0.0])), &format!("render.lua: the frame carries Garry's Mod's view, relative to the anchor: {view:?}"));
            // The server marks what the physgun picks up, and unmarks it on drop.
            let r = lua.run(ls, "local e = {} function e:SetNW2Bool(k, v) self[k] = v end                 hook.Run('OnPhysgunPickup', HARNESS_PLY, e) local a = e.gmsky_held hook.Run('PhysgunDrop', HARNESS_PLY, e)                 hook.Run('GravGunOnPickedUp', HARNESS_PLY, e) local b = e.gmsky_held hook.Run('GravGunOnDropped', HARNESS_PLY, e)                 return tostring(a) .. ' ' .. tostring(b) .. ' ' .. tostring(e.gmsky_held)");
            check(r == Ok(Some("true true false".into())), &format!("render.lua (server): physgun / gravity gun pick-up marks held, drop unmarks: {r:?}"));
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
    check(server_log.contains("stand-in for ? FF00ABCD: 3 parts") && server_log.contains("STAND-IN HIT ? FF00ABCD: bullet head, 75.0 damage (type 2) -> 450.0 Skyrim damage sent"),
          "gmod_server.log: stand-in made and its damage logged with the body part");
    check(server_log.contains("HURT by ? FF00ABCD: 12.0 Skyrim -> 12.0 Garry's Mod damage, melee, power attack, health 100 -> 88")
            && server_log.contains("HURT by ? 00000123: 4.0 Skyrim -> 4.0 Garry's Mod damage, magic, health 88 -> 84, armour 0 (attacker unknown)"),
          "gmod_server.log: each hurt from Skyrim logged");
    let client_log = std::fs::read_to_string(log_dir.join("gmod_client.log")).unwrap_or_default();
    check(client_log.contains("console: F1 from Skyrim: no Garry's Mod window found") && client_log.contains("console: no Garry's Mod window found") && client_log.contains("console: closed (the console was closed)")
            && client_log.contains("console: closed (Garry's Mod's window lost the front)"),
          "gmod_client.log: console opened and closed both ways");
    check(server_log.contains("respawn: a fight started; calm spot kept from 1.0 s ago") && server_log.contains("DIED (killed by ?)")
            && server_log.contains("last calm spot 5.0 s ago") && server_log.contains("RESPAWN at the last calm spot (5.0 s before dying)"),
          "gmod_server.log: fight, death and respawn logged");
    check(server_log.contains("water: SWIMMING at feet z") && server_log.contains("water: out of the water at feet z"),
          "gmod_server.log: into and out of the water logged");
    check(server_log.contains("actor near: Bandit 0001A2B3 lvl 9 (hostile fighting)") && server_log.contains("actors: table dropped (2 actors)"),
          "gmod_server.log: actors appearing and the dropped table logged");
    check(server_log.contains("attack from Garry's Mod's own binds") && server_log.contains("attack from Skyrim's buttons")
            && server_log.contains("weapon now weapon_physgun (was nothing)"),
          "gmod_server.log: attack source and weapon changes logged");
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
