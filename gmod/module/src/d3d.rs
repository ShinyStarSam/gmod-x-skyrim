//! Phase 2a after test 31 (DESIGN.md "2a after test 31", idea 1): reads Garry's Mod's Direct3D 9
//! blending settings, to see why its 2D drawing writes no alpha. **Read only:** nothing is hooked,
//! patched or set.
//!
//! The Direct3D 9 device (the object every draw goes through) isn't handed to Lua, so it is found:
//! Source's shaderapidx9.dll keeps a pointer to it in its data. To know what a device looks like,
//! a throwaway "null" device (it can't draw anything; the usual way overlay tools learn this) is
//! made once on a hidden window and released at once: its function table gives the addresses of a
//! real device's functions. Then every 8-byte value in shaderapidx9.dll's data is tried as a
//! pointer to an object whose function table has the same functions (memory read with
//! ReadProcessMemory on our own process, so a wrong pointer gives an error, not a crash). Nothing
//! is called on an object unless its functions match; then it is asked whether it is a device
//! (QueryInterface). Re-checked on every call (the device is replaced when the game resets it,
//! e.g. on a resolution change). Garry's Mod's own device is only read, never changed.
//!
//! Step 2b-1 (DESIGN.md "Step 2b in detail"): test 32 found no device with a plain device's
//! functions. Most likely Garry's Mod's is a Direct3D 9Ex device, so a throwaway Ex device is made
//! too, and a candidate matching either is asked. A failed search isn't repeated for 10 s and
//! lists every candidate. `device()` hands the device to present.rs (the Present hook).

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Instant;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcess() -> isize;
    fn ReadProcessMemory(process: isize, addr: *const c_void, buf: *mut c_void, size: usize, read: *mut usize) -> i32;
    fn GetProcAddress(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
}

#[link(name = "user32")]
extern "system" {
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
        parent: *mut c_void, menu: *mut c_void, inst: *mut c_void, param: *mut c_void) -> *mut c_void;
    fn DestroyWindow(hwnd: *mut c_void) -> i32;
}

/// IID_IDirect3DDevice9 {D0223B96-BF7A-43FD-92BD-A43B0D82B9EB}
const IID_DEVICE9: [u8; 16] = [
    0x96, 0x3B, 0x22, 0xD0, 0x7A, 0xBF, 0xFD, 0x43, 0x92, 0xBD, 0xA4, 0x3B, 0x0D, 0x82, 0xB9, 0xEB,
];
/// IID_IDirect3DDevice9Ex {B18B10CE-2649-405A-870F-95F777D4313A}
const IID_DEVICE9EX: [u8; 16] = [
    0xCE, 0x10, 0x8B, 0xB1, 0x49, 0x26, 0x5A, 0x40, 0x87, 0x0F, 0x95, 0xF7, 0x77, 0xD4, 0x31, 0x3A,
];
/// IDirect3DDevice9 has 119 functions; slots used here (d3d9.h order).
const DEVICE_FUNCS: usize = 119;
pub(crate) const QUERY_INTERFACE: usize = 0;
pub(crate) const RELEASE: usize = 2;
const GET_CREATION_PARAMETERS: usize = 9;
const GET_RENDER_TARGET: usize = 38;
const GET_RENDER_STATE: usize = 58;
/// IDirect3DSurface9::GetDesc
pub(crate) const SURFACE_GET_DESC: usize = 12;
/// A failed search is repeated at most this often (test 32: 27-35 ms per search, every call).
const RETRY_AFTER_FAIL_S: f64 = 10.0;

// D3DRENDERSTATETYPE values
const RS_ALPHATESTENABLE: u32 = 15;
const RS_SRCBLEND: u32 = 19;
const RS_DESTBLEND: u32 = 20;
const RS_ALPHABLENDENABLE: u32 = 27;
const RS_COLORWRITEENABLE: u32 = 168;
const RS_BLENDOP: u32 = 171;
const RS_SRGBWRITEENABLE: u32 = 194;
const RS_SEPARATEALPHABLENDENABLE: u32 = 206;
const RS_SRCBLENDALPHA: u32 = 207;
const RS_DESTBLENDALPHA: u32 = 208;
const RS_BLENDOPALPHA: u32 = 209;

#[derive(Clone, Copy)]
pub(crate) struct Found {
    /// Where shaderapidx9.dll keeps the device pointer; 0 = found through a resource's GetDevice.
    pub slot: usize,
    pub device: usize,
    pub vtable: usize,
    /// A Direct3D 9Ex device (it has PresentEx and the other Ex functions).
    pub ex: bool,
}

static FOUND: Mutex<Option<Found>> = Mutex::new(None);
/// The last failed search: when, and why.
static FAILED: Mutex<Option<(Instant, String)>> = Mutex::new(None);

/// Reads `n` bytes at `addr` from our own process; None instead of a crash if it isn't readable.
fn read(addr: usize, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), addr as *const c_void, buf.as_mut_ptr() as *mut c_void, n, &mut got) };
    (ok != 0 && got == n).then_some(buf)
}

fn read_u64(addr: usize) -> Option<usize> {
    read(addr, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize)
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// A loaded DLL's address range and its writable sections (start, size), from its PE header.
struct Image {
    base: usize,
    size: usize,
    writable: Vec<(usize, usize)>,
}

fn image(name: &str) -> Option<Image> {
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let base = unsafe { GetModuleHandleW(w.as_ptr()) } as usize;
    if base == 0 {
        return None;
    }
    let dos = read(base, 0x40)?;
    let nt = base + le32(&dos, 0x3C) as usize;
    let hdr = read(nt, 24 + 240)?;
    if &hdr[0..4] != b"PE\0\0" {
        return None;
    }
    let sections = u16::from_le_bytes([hdr[6], hdr[7]]) as usize;
    let opt_size = u16::from_le_bytes([hdr[20], hdr[21]]) as usize;
    let size = le32(&hdr, 24 + 56) as usize; // OptionalHeader64.SizeOfImage
    let table = read(nt + 24 + opt_size, sections * 40)?;
    let writable = (0..sections)
        .map(|i| &table[i * 40..i * 40 + 40])
        .filter(|s| le32(s, 36) & 0x8000_0000 != 0) // IMAGE_SCN_MEM_WRITE
        .map(|s| (base + le32(s, 12) as usize, le32(s, 8) as usize))
        .collect();
    Some(Image { base, size, writable })
}

impl Image {
    fn contains(&self, p: usize) -> bool {
        p >= self.base && p < self.base + self.size
    }
}

/// How many of the object's first 119 functions lie in d3d9.dll (None: not readable).
fn funcs_in(d3d9: &Image, vtable: usize) -> Option<usize> {
    let t = read(vtable, DEVICE_FUNCS * 8)?;
    Some((0..DEVICE_FUNCS).filter(|&i| d3d9.contains(u64::from_le_bytes(t[i * 8..i * 8 + 8].try_into().unwrap()) as usize)).count())
}

pub(crate) unsafe fn func(vtable: usize, i: usize) -> usize {
    *((vtable + i * 8) as *const usize)
}

/// Asks the object whether it has the interface `iid` (and gives back the reference it took).
unsafe fn has_interface(obj: usize, vtable: usize, iid: &[u8; 16]) -> bool {
    type Qi = unsafe extern "system" fn(usize, *const u8, *mut usize) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    let qi: Qi = std::mem::transmute(func(vtable, QUERY_INTERFACE));
    let mut out = 0usize;
    if qi(obj, iid.as_ptr(), &mut out) != 0 || out == 0 {
        return false;
    }
    let rel: Rel = std::mem::transmute(func(*(out as *const usize), RELEASE));
    rel(out);
    true
}

/// Asks the object whether it is a Direct3D 9 device.
unsafe fn is_device(obj: usize, vtable: usize) -> bool {
    has_interface(obj, vtable, &IID_DEVICE9)
}

/// Asks a device whether it is a Direct3D 9Ex device.
unsafe fn is_ex(obj: usize, vtable: usize) -> bool {
    has_interface(obj, vtable, &IID_DEVICE9EX)
}

/// Real devices' QueryInterface addresses, (name, address), from throwaway devices: a plain null
/// device, and an Ex device (null if Windows allows it, otherwise a real one on the hidden window).
/// Plus a line saying which could be made.
fn reference_functions(d3d9: &Image) -> (Vec<(&'static str, usize)>, String) {
    let mut refs = Vec::new();
    let mut notes = Vec::new();
    match with_device(d3d9, 4, false, |_, vt| unsafe { func(vt, QUERY_INTERFACE) }) {
        Ok(qi) => refs.push(("plain", qi)),
        Err(e) => notes.push(format!("plain reference: {e}")),
    }
    let ex = with_device(d3d9, 4, true, |_, vt| unsafe { func(vt, QUERY_INTERFACE) })
        .or_else(|e1| with_device(d3d9, 1, true, |_, vt| unsafe { func(vt, QUERY_INTERFACE) })
            .map_err(|e2| format!("{e1}; {e2}")));
    match ex {
        Ok(qi) => refs.push(("Ex", qi)),
        Err(e) => notes.push(format!("Ex reference: {e}")),
    }
    let line = refs.iter().map(|(n, qi)| format!("{n} QueryInterface d3d9+0x{:x}", qi - d3d9.base))
        .chain(notes).collect::<Vec<_>>().join(", ");
    (refs, line)
}

/// Makes a throwaway device of `dev_type` (4 = D3DDEVTYPE_NULLREF: no drawing, no driver work;
/// 1 = HAL, a real one) on a hidden window, gives it (object, function table) to `f`, and
/// releases everything again. `ex`: made through Direct3DCreate9Ex / CreateDeviceEx.
fn with_device<R>(d3d9: &Image, dev_type: u32, ex: bool, f: impl FnOnce(usize, usize) -> R) -> Result<R, String> {
    with_device_sized(d3d9, dev_type, ex, (16, 16, 0), f)
}

/// The same, with the back buffer's (width, height, format; format 0 = the desktop's).
fn with_device_sized<R>(d3d9: &Image, dev_type: u32, ex: bool, (bw, bh, bf): (u32, u32, u32), f: impl FnOnce(usize, usize) -> R) -> Result<R, String> {
    #[repr(C)]
    struct PresentParams {
        width: u32, height: u32, format: u32, count: u32, multisample: u32, quality: u32, swap_effect: u32,
        window: *mut c_void, windowed: i32, auto_depth: i32, depth_format: u32, flags: u32, refresh: u32, interval: u32,
    }
    type Create9 = unsafe extern "system" fn(u32) -> usize;
    type Create9Ex = unsafe extern "system" fn(u32, *mut usize) -> i32;
    type CreateDevice = unsafe extern "system" fn(usize, u32, u32, *mut c_void, u32, *mut PresentParams, *mut usize) -> i32;
    type CreateDeviceEx = unsafe extern "system" fn(usize, u32, u32, *mut c_void, u32, *mut PresentParams, *const c_void, *mut usize) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    unsafe {
        let name: &[u8] = if ex { b"Direct3DCreate9Ex\0" } else { b"Direct3DCreate9\0" };
        let create = GetProcAddress(d3d9.base as *mut c_void, name.as_ptr() as *const std::ffi::c_char);
        if create.is_null() {
            return Err(format!("d3d9.dll has no {}", std::str::from_utf8(&name[..name.len() - 1]).unwrap()));
        }
        let d3d = if ex {
            let create: Create9Ex = std::mem::transmute(create);
            let mut d3d = 0usize;
            let hr = create(32, &mut d3d); // D3D_SDK_VERSION
            if hr != 0 || d3d == 0 {
                return Err(format!("Direct3DCreate9Ex failed (0x{:08x})", hr as u32));
            }
            d3d
        } else {
            let create: Create9 = std::mem::transmute(create);
            let d3d = create(32);
            if d3d == 0 {
                return Err("Direct3DCreate9 failed".into());
            }
            d3d
        };
        let d3d_vt = *(d3d as *const usize);
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 16, 16,
            std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        let mut pp = PresentParams { width: bw, height: bh, format: bf, count: 1, multisample: 0, quality: 0,
            swap_effect: 1, window: hwnd, windowed: 1, auto_depth: 0, depth_format: 0, flags: 0, refresh: 0, interval: 0 };
        let mut dev = 0usize;
        // Software vertex processing, keep this thread's FPU mode, no window changes; plain
        // devices also without driver resource management (not allowed for Ex devices).
        let hr = if ex {
            let create_device: CreateDeviceEx = std::mem::transmute(func(d3d_vt, 20)); // IDirect3D9Ex::CreateDeviceEx
            create_device(d3d, 0, dev_type, hwnd, 0x20 | 0x2 | 0x800, &mut pp, std::ptr::null(), &mut dev)
        } else {
            let create_device: CreateDevice = std::mem::transmute(func(d3d_vt, 16)); // IDirect3D9::CreateDevice
            create_device(d3d, 0, dev_type, hwnd, 0x20 | 0x2 | 0x800 | 0x100, &mut pp, &mut dev)
        };
        let result = if hr == 0 && dev != 0 {
            let vt = *(dev as *const usize);
            let r = f(dev, vt);
            let rel: Rel = std::mem::transmute(func(vt, RELEASE));
            rel(dev);
            Ok(r)
        } else {
            Err(format!("{}device type {dev_type} not created (0x{:08x})", if ex { "Ex " } else { "" }, hr as u32))
        };
        if !hwnd.is_null() {
            DestroyWindow(hwnd);
        }
        let rel: Rel = std::mem::transmute(func(d3d_vt, RELEASE));
        rel(d3d);
        result
    }
}

/// Scans shaderapidx9.dll's data for the device. Returns it and a line for the log.
fn find() -> Result<(Found, String), String> {
    let t0 = Instant::now();
    let shader = image("shaderapidx9.dll").ok_or("shaderapidx9.dll is not loaded")?;
    let d3d9 = image("d3d9.dll").ok_or("d3d9.dll is not loaded")?;
    let (refs, ref_line) = reference_functions(&d3d9);
    if refs.is_empty() {
        return Err(format!("no reference device could be made ({ref_line})"));
    }
    let (mut tried, mut candidates) = (0usize, 0usize);
    let mut seen: Vec<String> = Vec::new();
    // Objects asked for their device: (where in shaderapidx9, their QueryInterface, the answer).
    let mut via: Vec<(usize, usize, Option<usize>)> = Vec::new();
    // Page by page: a page that can't be read is skipped, not the whole section.
    let pages: Vec<(usize, usize)> = shader.writable.iter()
        .flat_map(|&(start, size)| (start..start + size).step_by(4096).map(move |p| (p, (start + size - p).min(4096))))
        .collect();
    for (start, size) in pages {
        let Some(data) = read(start, size) else { continue };
        for off in (0..size.saturating_sub(7)).step_by(8) {
            let p = u64::from_le_bytes(data[off..off + 8].try_into().unwrap()) as usize;
            // A heap object: not null, not inside a DLL we know, a plausible user-mode address.
            if p < 0x10000 || p > 0x7FFF_FFFF_FFFF || p % 8 != 0 || d3d9.contains(p) || shader.contains(p) {
                continue;
            }
            tried += 1;
            let Some(vt) = read_u64(p) else { continue };
            if !d3d9.contains(vt) {
                // After test 40 (unit test finds_the_device_through_a_resource): this d3d9.dll
                // gives each device its own copy of its function table, outside the DLL. Such a
                // table counts if its QueryInterface is a real device's.
                let qi = read_u64(vt).unwrap_or(0);
                if qi != 0 && refs.iter().any(|(_, r)| *r == qi) && unsafe { is_device(p, vt) } {
                    let f = Found { slot: start + off, device: p, vtable: vt, ex: unsafe { is_ex(p, vt) } };
                    let line = format!(
                        "device found at shaderapidx9+0x{:x} in {:.1} ms ({} pointers tried; its own function table {}; Ex device: {}){}; Present = {}; references: {}",
                        f.slot - shader.base, t0.elapsed().as_secs_f64() * 1000.0, tried, module_of(vt),
                        if f.ex { "yes" } else { "no" }, unsafe { creation_flags(f) }, module_of(unsafe { func(vt, 17) }), ref_line);
                    return Ok((f, line));
                }
                continue;
            }
            let n = funcs_in(&d3d9, vt).unwrap_or(0);
            if n < 100 {
                continue;
            }
            candidates += 1;
            // Only called into if its QueryInterface is a real device's.
            let qi = read_u64(vt + QUERY_INTERFACE * 8).unwrap_or(0);
            let matched = refs.iter().find(|(_, r)| *r == qi).map(|(name, _)| *name);
            if seen.len() < 16 {
                seen.push(format!("[shaderapidx9+0x{:x}: table d3d9+0x{:x}, {n} functions in d3d9, QueryInterface d3d9+0x{:x}{}]",
                    start + off - shader.base, vt - d3d9.base, qi.wrapping_sub(d3d9.base),
                    matched.map(|m| format!(" = {m}")).unwrap_or_default()));
            }
            if matched.is_some() && unsafe { is_device(p, vt) } {
                let f = Found { slot: start + off, device: p, vtable: vt, ex: unsafe { is_ex(p, vt) } };
                let line = format!(
                    "device found at shaderapidx9+0x{:x} in {:.1} ms ({} pointers tried, {} with a function table in d3d9.dll; {} of {} functions in d3d9.dll; QueryInterface = {} reference; Ex device: {}){}; references: {}",
                    f.slot - shader.base, t0.elapsed().as_secs_f64() * 1000.0, tried, candidates, n, DEVICE_FUNCS,
                    matched.unwrap(), if f.ex { "yes" } else { "no" }, unsafe { creation_flags(f) }, ref_line);
                return Ok((f, line));
            }
            // Test 40: no direct pointer to the device. A texture or surface Source keeps knows
            // the device that made it (GetDevice).
            if via.len() < 16 {
                let got = unsafe { device_of_resource(p, vt) };
                via.push((start + off - shader.base, qi.wrapping_sub(d3d9.base), got));
            }
        }
    }
    let answers = via.iter().map(|(at, qi, got)| format!("[shaderapidx9+0x{at:x} (QueryInterface d3d9+0x{qi:x}): {}]",
        match got { Some(d) => format!("device 0x{d:x}"), None => "not a resource".into() })).collect::<Vec<_>>().join(" ");
    let devices: Vec<usize> = via.iter().filter_map(|v| v.2).collect();
    if let Some(&dev) = devices.first() {
        let vt = read_u64(dev).unwrap_or(0);
        // A pointer Direct3D itself handed out as a device: asking it is safe.
        if vt != 0 && unsafe { is_device(dev, vt) } {
            let f = Found { slot: 0, device: dev, vtable: vt, ex: unsafe { is_ex(dev, vt) } };
            let others = devices.iter().filter(|&&d| d != dev).count();
            let line = format!(
                "device found through a resource's GetDevice in {:.1} ms: 0x{dev:x}, function table {} ({} resources named it, {others} named another device; Ex device: {}){}; Present = {}; objects asked: {answers}",
                t0.elapsed().as_secs_f64() * 1000.0, module_of(vt), devices.len() - others, if f.ex { "yes" } else { "no" },
                unsafe { creation_flags(f) }, module_of(unsafe { func(vt, 17) }));
            return Ok((f, line));
        }
    }
    Err(format!("no device found in shaderapidx9.dll ({} pointers tried, {} with a function table in d3d9.dll, none answered as a device; {:.1} ms); references: {}; candidates: {}; asked for their device: {}",
        tried, candidates, t0.elapsed().as_secs_f64() * 1000.0, ref_line, seen.join(" "), answers))
}

/// IID_IDirect3DResource9 {05EEC05D-8F7D-4362-B999-D1BAF357C704}: textures, surfaces, buffers.
const IID_RESOURCE9: [u8; 16] = [
    0x5D, 0xC0, 0xEE, 0x05, 0x7D, 0x8F, 0x62, 0x43, 0xB9, 0x99, 0xD1, 0xBA, 0xF3, 0x57, 0xC7, 0x04,
];
/// IDirect3DResource9::GetDevice
const RESOURCE_GET_DEVICE: usize = 3;

/// If the object is a Direct3D resource (texture, surface, buffer), the device that made it.
/// Calls the object's QueryInterface (slot 0 of every COM object; every public d3d9.dll object is
/// one), then GetDevice on the resource interface it hands back. References are given back.
unsafe fn device_of_resource(obj: usize, vtable: usize) -> Option<usize> {
    type Qi = unsafe extern "system" fn(usize, *const u8, *mut usize) -> i32;
    type GetDevice = unsafe extern "system" fn(usize, *mut usize) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    let qi: Qi = std::mem::transmute(func(vtable, QUERY_INTERFACE));
    let mut res = 0usize;
    if qi(obj, IID_RESOURCE9.as_ptr(), &mut res) != 0 || res == 0 {
        return None;
    }
    let rvt = *(res as *const usize);
    let get_device: GetDevice = std::mem::transmute(func(rvt, RESOURCE_GET_DEVICE));
    let mut dev = 0usize;
    let ok = get_device(res, &mut dev) == 0 && dev != 0;
    let rel: Rel = std::mem::transmute(func(rvt, RELEASE));
    rel(res);
    if !ok {
        return None;
    }
    // GetDevice took a reference; the game keeps its own, so give ours back.
    let rel: Rel = std::mem::transmute(func(*(dev as *const usize), RELEASE));
    rel(dev);
    Some(dev)
}

/// "name+0xoffset" of the loaded DLL holding an address, for the log.
fn module_of(addr: usize) -> String {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleExW(flags: u32, addr: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, size: u32) -> u32;
    }
    let mut m = std::ptr::null_mut();
    // FROM_ADDRESS | UNCHANGED_REFCOUNT
    if unsafe { GetModuleHandleExW(0x4 | 0x2, addr as *const c_void, &mut m) } == 0 {
        return format!("0x{addr:x} (in no DLL)");
    }
    let mut buf = [0u16; 260];
    let n = unsafe { GetModuleFileNameW(m, buf.as_mut_ptr(), 260) } as usize;
    let path = String::from_utf16_lossy(&buf[..n]);
    let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
    format!("{name}+0x{:x}", addr - m as usize)
}
/// Garry's Mod's device: the one found before if it is still in place, otherwise searched again
/// (not more than every RETRY_AFTER_FAIL_S after a failure). The string is a note for the log
/// when something happened (found, changed).
pub(crate) fn device() -> Result<(Found, String), String> {
    let mut found = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    let mut note = String::new();
    // Still the same device in the same place?
    if let Some(f) = *found {
        // (slot 0: found through a resource, so only the device itself can be checked.)
        if (f.slot == 0 || read_u64(f.slot) == Some(f.device)) && read_u64(f.device) == Some(f.vtable) {
            return Ok((f, note));
        }
        *found = None;
        note = "device changed, searched again; ".into();
    }
    let mut failed = FAILED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, e)) = failed.as_ref() {
        if at.elapsed().as_secs_f64() < RETRY_AFTER_FAIL_S {
            return Err(format!("{e} (searched {:.0} s ago; not again before {RETRY_AFTER_FAIL_S:.0} s)", at.elapsed().as_secs_f64()));
        }
    }
    match find() {
        Ok((f, line)) => {
            *found = Some(f);
            *failed = None;
            note += &line;
            Ok((f, note))
        }
        Err(e) => {
            // Without the game's graphics DLLs nothing is searched, so nothing to wait for.
            if e.contains("is not loaded") {
                return Err(e);
            }
            *failed = Some((Instant::now(), format!("{note}{e}")));
            Err(format!("{note}{e}"))
        }
    }
}

/// The device's creation flags: a "pure" device can't be read; "multithreaded" is thread-safe.
unsafe fn creation_flags(f: Found) -> String {
    #[repr(C)]
    struct Params { adapter: u32, dev_type: u32, focus: usize, flags: u32 }
    type Gcp = unsafe extern "system" fn(usize, *mut Params) -> i32;
    let gcp: Gcp = std::mem::transmute(func(f.vtable, GET_CREATION_PARAMETERS));
    let mut p = Params { adapter: 0, dev_type: 0, focus: 0, flags: 0 };
    if gcp(f.device, &mut p) != 0 {
        return ", creation parameters unreadable".into();
    }
    format!(", creation flags 0x{:x} (pure {}, multithreaded {})", p.flags,
        if p.flags & 0x10 != 0 { "YES: settings unreadable" } else { "no" },
        if p.flags & 0x4 != 0 { "yes" } else { "no" })
}

fn blend_name(v: u32) -> String {
    match v {
        1 => "ZERO", 2 => "ONE", 3 => "SRCCOLOR", 4 => "INVSRCCOLOR", 5 => "SRCALPHA", 6 => "INVSRCALPHA",
        7 => "DESTALPHA", 8 => "INVDESTALPHA", 9 => "DESTCOLOR", 10 => "INVDESTCOLOR", 11 => "SRCALPHASAT",
        _ => return v.to_string(),
    }.into()
}

fn op_name(v: u32) -> String {
    match v { 1 => "ADD", 2 => "SUBTRACT", 3 => "REVSUBTRACT", 4 => "MIN", 5 => "MAX", _ => return v.to_string() }.into()
}

pub(crate) fn format_name(v: u32) -> String {
    match v {
        21 => "A8R8G8B8", 22 => "X8R8G8B8", 32 => "A8B8G8R8", 33 => "X8B8G8R8", 36 => "A16B16G16R16",
        113 => "A16B16G16R16F", 116 => "A32B32G32R32F", 23 => "R5G6B5", 31 => "A2R10G10B10", 35 => "A2B10G10R10",
        _ => return v.to_string(),
    }.into()
}

/// The blending settings now, one line.
unsafe fn state_line(f: Found) -> String {
    type Grs = unsafe extern "system" fn(usize, u32, *mut u32) -> i32;
    let grs: Grs = std::mem::transmute(func(f.vtable, GET_RENDER_STATE));
    let rs = |s: u32| {
        let mut v = 0u32;
        if grs(f.device, s, &mut v) == 0 { Some(v) } else { None }
    };
    let Some(cw) = rs(RS_COLORWRITEENABLE) else { return "GetRenderState failed".into() };
    let mask: String = [(1, 'R'), (2, 'G'), (4, 'B'), (8, 'A')]
        .iter().map(|&(b, c)| if cw & b != 0 { c } else { '-' }).collect();
    let g = |s: u32| rs(s).unwrap_or(u32::MAX);
    let on = |s: u32| if g(s) != 0 { "on" } else { "off" };
    format!(
        "colour write {mask}, blending {} ({}/{} {}), separate alpha blending {} ({}/{} {}), alpha test {}, sRGB write {}, {}",
        on(RS_ALPHABLENDENABLE), blend_name(g(RS_SRCBLEND)), blend_name(g(RS_DESTBLEND)), op_name(g(RS_BLENDOP)),
        on(RS_SEPARATEALPHABLENDENABLE), blend_name(g(RS_SRCBLENDALPHA)), blend_name(g(RS_DESTBLENDALPHA)),
        op_name(g(RS_BLENDOPALPHA)), on(RS_ALPHATESTENABLE), on(RS_SRGBWRITEENABLE), target_line(f))
}

/// The current render target 0 (the picture being drawn into): format and size.
unsafe fn target_line(f: Found) -> String {
    type Grt = unsafe extern "system" fn(usize, u32, *mut usize) -> i32;
    type Desc = unsafe extern "system" fn(usize, *mut [u32; 8]) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    let grt: Grt = std::mem::transmute(func(f.vtable, GET_RENDER_TARGET));
    let mut s = 0usize;
    if grt(f.device, 0, &mut s) != 0 || s == 0 {
        return "render target unreadable".into();
    }
    let svt = *(s as *const usize);
    let desc: Desc = std::mem::transmute(func(svt, SURFACE_GET_DESC));
    let mut d = [0u32; 8];
    let ok = desc(s, &mut d) == 0;
    let rel: Rel = std::mem::transmute(func(svt, RELEASE));
    rel(s);
    if !ok {
        return "render target description unreadable".into();
    }
    format!("render target {} {}x{}", format_name(d[0]), d[6], d[7])
}

/// One line for gmsky.D3DState: the settings now, plus how the device was found the first time.
pub fn state() -> String {
    match device() {
        Ok((f, note)) if note.is_empty() => unsafe { state_line(f) },
        Ok((f, note)) => format!("{note}; {}", unsafe { state_line(f) }),
        Err(e) => format!("not found: {e}"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
    }

    /// A real (hardware) 16 x 16 device on a hidden window, plain or Ex, for other modules' tests.
    pub(crate) fn with_test_device<R>(ex: bool, f: impl FnOnce(usize, usize) -> R) -> R {
        let name: Vec<u16> = "d3d9.dll\0".encode_utf16().collect();
        assert!(!unsafe { LoadLibraryW(name.as_ptr()) }.is_null());
        let d3d9 = image("d3d9.dll").expect("d3d9.dll image");
        with_device(&d3d9, 1, ex, f).expect("test device")
    }

    /// A real hidden device with a w x h back buffer of `format` (21 = A8R8G8B8, like Garry's Mod's).
    pub(crate) fn with_test_device_sized<R>(ex: bool, w: u32, h: u32, format: u32, f: impl FnOnce(usize, usize) -> R) -> R {
        let name: Vec<u16> = "d3d9.dll\0".encode_utf16().collect();
        assert!(!unsafe { LoadLibraryW(name.as_ptr()) }.is_null());
        let d3d9 = image("d3d9.dll").expect("d3d9.dll image");
        with_device_sized(&d3d9, 1, ex, (w, h, format), f).expect("test device")
    }

    /// Tests that use the module's global hook / capture state take this, so they don't run at once.
    pub(crate) static GLOBAL_STATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The function numbers and structures used on Garry's Mod's device, tried on a null device
    /// on this PC (without the game, so not the scan itself).
    #[test]
    fn reads_a_null_device() {
        let name: Vec<u16> = "d3d9.dll\0".encode_utf16().collect();
        assert!(!unsafe { LoadLibraryW(name.as_ptr()) }.is_null());
        let d3d9 = image("d3d9.dll").expect("d3d9.dll image");
        assert!(!d3d9.writable.is_empty());
        let (refs, ref_line) = reference_functions(&d3d9);
        println!("references: {ref_line}");
        let qi = refs.iter().find(|(n, _)| *n == "plain").expect("plain null device").1;
        assert!(d3d9.contains(qi));
        let (_, grs) = with_device(&d3d9, 4, false, |_, vt| unsafe { (func(vt, QUERY_INTERFACE), func(vt, GET_RENDER_STATE)) }).unwrap();
        let line = with_device(&d3d9, 4, false, |dev, vt| unsafe {
            let f = Found { slot: 0, device: dev, vtable: vt, ex: false };
            assert!(is_device(dev, vt));
            assert!(!is_ex(dev, vt));
            assert!(funcs_in(&d3d9, vt).unwrap() >= 100);
            format!("{} | {}", creation_flags(f), state_line(f))
        }).unwrap();
        println!("{line}");
        // Direct3D 9's defaults: all colours written, blending off, ONE/ZERO.
        assert!(line.contains("colour write RGBA, blending off (ONE/ZERO ADD)"), "{line}");
        assert!(line.contains("pure no"), "{line}");
        // A real (hardware) device, 16 x 16, hidden: the render target can be read there, and a
        // real device has the null device's functions (what the scan relies on).
        let line = with_device(&d3d9, 1, false, |dev, vt| unsafe {
            assert_eq!((func(vt, QUERY_INTERFACE), func(vt, GET_RENDER_STATE)), (qi, grs));
            state_line(Found { slot: 0, device: dev, vtable: vt, ex: false })
        }).unwrap();
        println!("{line}");
        assert!(line.contains("render target") && line.contains(" 16x16"), "{line}");
    }

    /// After test 40: a texture names the device that made it (plain and Ex), and an object that
    /// isn't a resource (the IDirect3D9 factory) answers "no" without harm.
    #[test]
    fn finds_the_device_through_a_resource() {
        type CreateTexture = unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32, *mut usize, *mut c_void) -> i32;
        type GetDirect3D = unsafe extern "system" fn(usize, *mut usize) -> i32;
        type Rel = unsafe extern "system" fn(usize) -> u32;
        for ex in [false, true] {
            with_test_device(ex, |dev, vt| unsafe {
                let create: CreateTexture = std::mem::transmute(func(vt, 23));
                let mut tex = 0usize;
                // 16 x 16, 1 level, render target usage, A8R8G8B8, default pool.
                assert_eq!(create(dev, 16, 16, 1, 1, 21, 0, &mut tex, std::ptr::null_mut()), 0);
                let tvt = *(tex as *const usize);
                assert_eq!(device_of_resource(tex, tvt), Some(dev));
                let gd3d: GetDirect3D = std::mem::transmute(func(vt, 6));
                let mut d3d = 0usize;
                assert_eq!(gd3d(dev, &mut d3d), 0);
                assert_eq!(device_of_resource(d3d, *(d3d as *const usize)), None);
                println!("ex={ex}: texture -> device ok; device table {}, Present {}", module_of(vt), module_of(func(vt, 17)));
                let rel: Rel = std::mem::transmute(func(*(d3d as *const usize), RELEASE));
                rel(d3d);
                let rel: Rel = std::mem::transmute(func(tvt, RELEASE));
                rel(tex);
            });
        }
    }

    /// Step 2b-1: an Ex device (what Garry's Mod probably has) answers as a device and as Ex,
    /// and the search's Ex reference is its QueryInterface. Prints whether it differs from a
    /// plain device's (test 32's suspicion).
    #[test]
    fn knows_an_ex_device() {
        let name: Vec<u16> = "d3d9.dll\0".encode_utf16().collect();
        assert!(!unsafe { LoadLibraryW(name.as_ptr()) }.is_null());
        let d3d9 = image("d3d9.dll").expect("d3d9.dll image");
        let (refs, ref_line) = reference_functions(&d3d9);
        println!("references: {ref_line}");
        let ex_qi = refs.iter().find(|(n, _)| *n == "Ex").expect("an Ex reference device").1;
        let plain_qi = refs.iter().find(|(n, _)| *n == "plain").expect("plain reference").1;
        println!("Ex QueryInterface {} the plain one", if ex_qi == plain_qi { "is the same as" } else { "DIFFERS from" });
        let line = with_device(&d3d9, 1, true, |dev, vt| unsafe {
            assert_eq!(func(vt, QUERY_INTERFACE), ex_qi);
            assert!(is_device(dev, vt) && is_ex(dev, vt));
            let f = Found { slot: 0, device: dev, vtable: vt, ex: true };
            format!("{} | {}", creation_flags(f), state_line(f))
        }).unwrap();
        println!("{line}");
        assert!(line.contains(" 16x16"), "{line}");
    }
}
