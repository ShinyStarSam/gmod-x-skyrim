//! The few Lua C functions we use, looked up at runtime from Garry's Mod's lua_shared.dll.
//! (Garry's Mod's Lua is LuaJIT 2.1, which uses the Lua 5.1 C API.)

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::OnceLock;

pub type State = c_void;
pub type CFn = unsafe extern "C" fn(*mut State) -> c_int;

pub const GLOBALSINDEX: c_int = -10002;
pub const fn upvalueindex(i: c_int) -> c_int {
    GLOBALSINDEX - i
}

#[allow(non_snake_case, dead_code)]
pub struct Api {
    pub pushcclosure: unsafe extern "C" fn(*mut State, CFn, c_int),
    pub setfield: unsafe extern "C" fn(*mut State, c_int, *const c_char),
    pub getfield: unsafe extern "C" fn(*mut State, c_int, *const c_char),
    pub createtable: unsafe extern "C" fn(*mut State, c_int, c_int),
    pub rawseti: unsafe extern "C" fn(*mut State, c_int, c_int),
    pub pushnumber: unsafe extern "C" fn(*mut State, f64),
    pub pushboolean: unsafe extern "C" fn(*mut State, c_int),
    pub pushlstring: unsafe extern "C" fn(*mut State, *const c_char, usize),
    pub tonumber: unsafe extern "C" fn(*mut State, c_int) -> f64,
    pub tolstring: unsafe extern "C" fn(*mut State, c_int, *mut usize) -> *const c_char,
    pub type_: unsafe extern "C" fn(*mut State, c_int) -> c_int,
    pub toboolean: unsafe extern "C" fn(*mut State, c_int) -> c_int,
    pub settop: unsafe extern "C" fn(*mut State, c_int),
    pub gettop: unsafe extern "C" fn(*mut State) -> c_int,
    pub loadbuffer: unsafe extern "C" fn(*mut State, *const c_char, usize, *const c_char) -> c_int,
    pub pcall: unsafe extern "C" fn(*mut State, c_int, c_int, c_int) -> c_int,
    pub rawgeti: unsafe extern "C" fn(*mut State, c_int, c_int),
    pub objlen: unsafe extern "C" fn(*mut State, c_int) -> usize,
    pub touserdata: unsafe extern "C" fn(*mut State, c_int) -> *mut c_void,
    pub pushnil: unsafe extern "C" fn(*mut State),
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

pub fn api() -> Result<&'static Api, String> {
    API.get_or_init(load).as_ref().map_err(|e| e.clone())
}

fn load() -> Result<Api, String> {
    let name: Vec<u16> = "lua_shared.dll".encode_utf16().chain(Some(0)).collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if module.is_null() {
        return Err("lua_shared.dll is not loaded in this process".into());
    }
    macro_rules! sym {
        ($n:literal) => {{
            let p = unsafe { GetProcAddress(module, concat!($n, "\0").as_ptr() as *const c_char) };
            if p.is_null() {
                return Err(format!("lua_shared.dll has no export {}", $n));
            }
            unsafe { std::mem::transmute(p) }
        }};
    }
    Ok(Api {
        pushcclosure: sym!("lua_pushcclosure"),
        setfield: sym!("lua_setfield"),
        getfield: sym!("lua_getfield"),
        createtable: sym!("lua_createtable"),
        rawseti: sym!("lua_rawseti"),
        pushnumber: sym!("lua_pushnumber"),
        pushboolean: sym!("lua_pushboolean"),
        pushlstring: sym!("lua_pushlstring"),
        tonumber: sym!("lua_tonumber"),
        tolstring: sym!("lua_tolstring"),
        type_: sym!("lua_type"),
        toboolean: sym!("lua_toboolean"),
        settop: sym!("lua_settop"),
        gettop: sym!("lua_gettop"),
        loadbuffer: sym!("luaL_loadbuffer"),
        pcall: sym!("lua_pcall"),
        rawgeti: sym!("lua_rawgeti"),
        objlen: sym!("lua_objlen"),
        touserdata: sym!("lua_touserdata"),
        pushnil: sym!("lua_pushnil"),
    })
}

// ---- Small safe-ish helpers. All take the state pointer Lua gave us. ----

pub unsafe fn push_str(a: &Api, l: *mut State, s: &str) {
    (a.pushlstring)(l, s.as_ptr() as *const c_char, s.len());
}

pub unsafe fn to_string(a: &Api, l: *mut State, idx: c_int) -> Option<String> {
    let mut len = 0usize;
    let p = (a.tolstring)(l, idx, &mut len);
    if p.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(p as *const u8, len);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// The raw bytes of a Lua string (binary data such as a PNG; no UTF-8 conversion).
pub unsafe fn to_bytes(a: &Api, l: *mut State, idx: c_int) -> Option<Vec<u8>> {
    let mut len = 0usize;
    let p = (a.tolstring)(l, idx, &mut len);
    if p.is_null() {
        return None;
    }
    Some(std::slice::from_raw_parts(p as *const u8, len).to_vec())
}

/// A Lua array of numbers (t[1] .. t[#t]) as f64s; None if the value isn't a table.
pub unsafe fn to_numbers(a: &Api, l: *mut State, idx: c_int) -> Option<Vec<f64>> {
    const LUA_TTABLE: c_int = 5;
    if (a.type_)(l, idx) != LUA_TTABLE {
        return None;
    }
    let n = (a.objlen)(l, idx);
    let mut out = Vec::with_capacity(n);
    for i in 1..=n as c_int {
        (a.rawgeti)(l, idx, i);
        out.push((a.tonumber)(l, -1));
        (a.settop)(l, -2);
    }
    Some(out)
}

pub unsafe fn set_field(a: &Api, l: *mut State, idx: c_int, key: &str) {
    let k = CString::new(key).unwrap();
    (a.setfield)(l, idx, k.as_ptr());
}

/// Compiles and runs a chunk. Returns the Lua error message on failure. Leaves the stack as it was.
pub unsafe fn run_chunk(a: &Api, l: *mut State, code: &[u8], chunk_name: &str) -> Result<(), String> {
    let top = (a.gettop)(l);
    let name = CString::new(format!("@{chunk_name}")).unwrap();
    let rc = (a.loadbuffer)(l, code.as_ptr() as *const c_char, code.len(), name.as_ptr());
    if rc == 0 && (a.pcall)(l, 0, 0, 0) == 0 {
        (a.settop)(l, top);
        return Ok(());
    }
    let msg = to_string(a, l, -1).unwrap_or_else(|| "(non-string error)".into());
    (a.settop)(l, top);
    Err(msg)
}

#[allow(dead_code)]
pub fn cstr_lossy(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}
