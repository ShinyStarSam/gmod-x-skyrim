//! gmsky loader stub.
//!
//! This DLL is copied ONCE into garrysmod/lua/bin under two names:
//!   gmsv_gmsky_win64.dll  (server side, loaded by `require("gmsky")` in server Lua)
//!   gmcl_gmsky_win64.dll  (client side, loaded by `require("gmsky")` in client Lua)
//! It must never need to change. All it does:
//!   1. work out which side it is from its own file name,
//!   2. copy <project>/target/release/gmsky_module.dll to <project>/build/gmod/loaded/ under a
//!      per-process name (so the original can be rebuilt while Garry's Mod runs),
//!   3. load that copy and hand over to gmsky_open / gmsky_close.
//! The project folder is baked in at build time; the GMSKY_ROOT environment variable overrides it.
//! Log: <project>/logs/stub.log

use std::ffi::{c_char, c_int, c_void, OsString};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const STUB_ABI_VERSION: u32 = 1;
const SIDE_SERVER: u32 = 0;
const SIDE_CLIENT: u32 = 1;

type OpenFn = unsafe extern "C" fn(*mut c_void, u32, *const u16) -> c_int;
type CloseFn = unsafe extern "C" fn(*mut c_void, u32) -> c_int;
type AbiFn = extern "C" fn() -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExW(flags: u32, addr: *const u16, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn GetLastError() -> u32;
    fn GetCurrentProcessId() -> u32;
}
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;

static CLOSE: Mutex<Option<usize>> = Mutex::new(None);

fn project_root() -> PathBuf {
    if let Some(r) = std::env::var_os("GMSKY_ROOT") {
        return PathBuf::from(r);
    }
    // gmod/stub -> project root, fixed at build time.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn log(root: &Path, msg: &str) {
    let dir = root.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("stub.log")) {
        let _ = writeln!(f, "[pid {}] {}", unsafe { GetCurrentProcessId() }, msg);
    }
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// This DLL's own path, from the address of a function inside it.
fn own_path() -> Option<PathBuf> {
    unsafe {
        let mut module = std::ptr::null_mut();
        let ok = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            own_path as *const u16, &mut module);
        if ok == 0 {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let n = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
        if n == 0 {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buf[..n])))
    }
}

fn side_from_name(path: &Option<PathBuf>) -> u32 {
    let name = path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase());
    match name {
        Some(n) if n.starts_with("gmcl_") => SIDE_CLIENT,
        _ => SIDE_SERVER,
    }
}

unsafe fn sym(module: *mut c_void, name: &str) -> *mut c_void {
    let c = format!("{name}\0");
    GetProcAddress(module, c.as_ptr() as *const c_char)
}

/// Loads the shadow copy of the real module. Returns (open, close) or an error message.
unsafe fn load_module(root: &Path) -> Result<(OpenFn, CloseFn), String> {
    let src = root.join("target").join("release").join("gmsky_module.dll");
    let dir = root.join("build").join("gmod").join("loaded");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let pid = GetCurrentProcessId();
    let shadow = dir.join(format!("gmsky_module_{pid}.dll"));
    if !shadow.exists() {
        // Clean up copies from earlier runs (files still in use simply fail to delete).
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        std::fs::copy(&src, &shadow).map_err(|e| format!("copy {} -> {}: {e}", src.display(), shadow.display()))?;
    }
    let module = LoadLibraryW(wide(&shadow).as_ptr());
    if module.is_null() {
        return Err(format!("LoadLibraryW({}) failed, error {}", shadow.display(), GetLastError()));
    }
    let abi = sym(module, "gmsky_abi_version");
    let open = sym(module, "gmsky_open");
    let close = sym(module, "gmsky_close");
    if abi.is_null() || open.is_null() || close.is_null() {
        return Err("module is missing gmsky_abi_version / gmsky_open / gmsky_close".into());
    }
    let abi: AbiFn = std::mem::transmute(abi);
    let v = abi();
    if v != STUB_ABI_VERSION {
        return Err(format!("module ABI version {v}, stub expects {STUB_ABI_VERSION}"));
    }
    Ok((std::mem::transmute::<*mut c_void, OpenFn>(open), std::mem::transmute::<*mut c_void, CloseFn>(close)))
}

#[no_mangle]
pub unsafe extern "C" fn gmod13_open(l: *mut c_void) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let root = project_root();
        let me = own_path();
        let side = side_from_name(&me);
        log(&root, &format!("gmod13_open side={} stub={:?} root={}", side, me, root.display()));
        match load_module(&root) {
            Ok((open, close)) => {
                *CLOSE.lock().unwrap_or_else(|e| e.into_inner()) = Some(close as usize);
                let rc = open(l, side, wide(&root).as_ptr());
                log(&root, &format!("gmsky_open returned {rc}"));
                rc
            }
            Err(e) => {
                log(&root, &format!("ERROR: {e}"));
                0
            }
        }
    }));
    r.unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn gmod13_close(l: *mut c_void) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let side = side_from_name(&own_path());
        if let Some(f) = *CLOSE.lock().unwrap_or_else(|e| e.into_inner()) {
            let close: CloseFn = std::mem::transmute(f);
            close(l, side);
        }
        0
    }));
    r.unwrap_or(0)
}
