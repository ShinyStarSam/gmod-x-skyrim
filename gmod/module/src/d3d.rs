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
/// IDirect3DDevice9 has 119 functions; slots used here (d3d9.h order).
const DEVICE_FUNCS: usize = 119;
const QUERY_INTERFACE: usize = 0;
const RELEASE: usize = 2;
const GET_CREATION_PARAMETERS: usize = 9;
const GET_RENDER_TARGET: usize = 38;
const GET_RENDER_STATE: usize = 58;
/// IDirect3DSurface9::GetDesc
const SURFACE_GET_DESC: usize = 12;

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
struct Found {
    /// Where shaderapidx9.dll keeps the device pointer.
    slot: usize,
    device: usize,
    vtable: usize,
}

static FOUND: Mutex<Option<Found>> = Mutex::new(None);

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

unsafe fn func(vtable: usize, i: usize) -> usize {
    *((vtable + i * 8) as *const usize)
}

/// Asks the object whether it is a Direct3D 9 device (and gives back the reference it took).
unsafe fn is_device(obj: usize, vtable: usize) -> bool {
    type Qi = unsafe extern "system" fn(usize, *const u8, *mut usize) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    let qi: Qi = std::mem::transmute(func(vtable, QUERY_INTERFACE));
    let mut out = 0usize;
    if qi(obj, IID_DEVICE9.as_ptr(), &mut out) != 0 || out == 0 {
        return false;
    }
    let rel: Rel = std::mem::transmute(func(*(out as *const usize), RELEASE));
    rel(out);
    true
}

/// A real device's QueryInterface and GetRenderState addresses, from a throwaway null device.
fn reference_functions(d3d9: &Image) -> Result<(usize, usize), String> {
    with_device(d3d9, 4, |_, vt| unsafe { (func(vt, QUERY_INTERFACE), func(vt, GET_RENDER_STATE)) })
}

/// Makes a throwaway device of `dev_type` (4 = D3DDEVTYPE_NULLREF: no drawing, no driver work;
/// the unit test also uses 1 = HAL, a real one) on a hidden window, gives it (object, function
/// table) to `f`, and releases everything again.
fn with_device<R>(d3d9: &Image, dev_type: u32, f: impl FnOnce(usize, usize) -> R) -> Result<R, String> {
    #[repr(C)]
    struct PresentParams {
        width: u32, height: u32, format: u32, count: u32, multisample: u32, quality: u32, swap_effect: u32,
        window: *mut c_void, windowed: i32, auto_depth: i32, depth_format: u32, flags: u32, refresh: u32, interval: u32,
    }
    type Create9 = unsafe extern "system" fn(u32) -> usize;
    type CreateDevice = unsafe extern "system" fn(usize, u32, u32, *mut c_void, u32, *mut PresentParams, *mut usize) -> i32;
    type Rel = unsafe extern "system" fn(usize) -> u32;
    unsafe {
        let create9 = GetProcAddress(d3d9.base as *mut c_void, b"Direct3DCreate9\0".as_ptr() as *const std::ffi::c_char);
        if create9.is_null() {
            return Err("d3d9.dll has no Direct3DCreate9".into());
        }
        let create9: Create9 = std::mem::transmute(create9);
        let d3d = create9(32); // D3D_SDK_VERSION
        if d3d == 0 {
            return Err("Direct3DCreate9 failed".into());
        }
        let d3d_vt = *(d3d as *const usize);
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 16, 16,
            std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        let mut pp = PresentParams { width: 16, height: 16, format: 0, count: 1, multisample: 0, quality: 0,
            swap_effect: 1, window: hwnd, windowed: 1, auto_depth: 0, depth_format: 0, flags: 0, refresh: 0, interval: 0 };
        let create_device: CreateDevice = std::mem::transmute(func(d3d_vt, 16)); // IDirect3D9::CreateDevice
        let mut dev = 0usize;
        // NULLREF device; software vertex processing, keep this thread's FPU mode, no window
        // changes, no driver resource management.
        let hr = create_device(d3d, 0, dev_type, hwnd, 0x20 | 0x2 | 0x800 | 0x100, &mut pp, &mut dev);
        let result = if hr == 0 && dev != 0 {
            let vt = *(dev as *const usize);
            let r = f(dev, vt);
            let rel: Rel = std::mem::transmute(func(vt, RELEASE));
            rel(dev);
            Ok(r)
        } else {
            Err(format!("device type {dev_type} not created (0x{:08x})", hr as u32))
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
    let (ref_qi, ref_grs) = reference_functions(&d3d9)?;
    let (mut tried, mut candidates) = (0usize, 0usize);
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
                continue;
            }
            let n = funcs_in(&d3d9, vt).unwrap_or(0);
            if n < 100 {
                continue;
            }
            candidates += 1;
            // Only called into if it has a real device's functions.
            let same = read_u64(vt + QUERY_INTERFACE * 8) == Some(ref_qi) && read_u64(vt + GET_RENDER_STATE * 8) == Some(ref_grs);
            if same && unsafe { is_device(p, vt) } {
                let f = Found { slot: start + off, device: p, vtable: vt };
                let line = format!(
                    "device found at shaderapidx9+0x{:x} in {:.1} ms ({} pointers tried, {} with a function table in d3d9.dll; {} of {} functions in d3d9.dll){}",
                    f.slot - shader.base, t0.elapsed().as_secs_f64() * 1000.0, tried, candidates, n, DEVICE_FUNCS,
                    unsafe { creation_flags(f) });
                return Ok((f, line));
            }
        }
    }
    Err(format!("no device found in shaderapidx9.dll ({} pointers tried, {} with a function table in d3d9.dll, none with the null device's functions; {:.1} ms)",
        tried, candidates, t0.elapsed().as_secs_f64() * 1000.0))
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

fn format_name(v: u32) -> String {
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
    let mut found = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    let mut note = String::new();
    // Still the same device in the same place?
    if let Some(f) = *found {
        if read_u64(f.slot) != Some(f.device) || read_u64(f.device) != Some(f.vtable) {
            *found = None;
            note = "device changed, searched again; ".into();
        }
    }
    if found.is_none() {
        match find() {
            Ok((f, line)) => {
                *found = Some(f);
                note += &line;
                note += "; ";
            }
            Err(e) => return format!("{note}not found: {e}"),
        }
    }
    let f = found.unwrap();
    format!("{note}{}", unsafe { state_line(f) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
    }

    /// The function numbers and structures used on Garry's Mod's device, tried on a null device
    /// on this PC (without the game, so not the scan itself).
    #[test]
    fn reads_a_null_device() {
        let name: Vec<u16> = "d3d9.dll\0".encode_utf16().collect();
        assert!(!unsafe { LoadLibraryW(name.as_ptr()) }.is_null());
        let d3d9 = image("d3d9.dll").expect("d3d9.dll image");
        assert!(!d3d9.writable.is_empty());
        let (qi, grs) = reference_functions(&d3d9).expect("null device");
        assert!(d3d9.contains(qi) && d3d9.contains(grs));
        let line = with_device(&d3d9, 4, |dev, vt| unsafe {
            let f = Found { slot: 0, device: dev, vtable: vt };
            assert!(is_device(dev, vt));
            assert!(funcs_in(&d3d9, vt).unwrap() >= 100);
            format!("{} | {}", creation_flags(f), state_line(f))
        }).unwrap();
        println!("{line}");
        // Direct3D 9's defaults: all colours written, blending off, ONE/ZERO.
        assert!(line.contains("colour write RGBA, blending off (ONE/ZERO ADD)"), "{line}");
        assert!(line.contains("pure no"), "{line}");
        // A real (hardware) device, 16 x 16, hidden: the render target can be read there, and a
        // real device has the null device's functions (what the scan relies on).
        let line = with_device(&d3d9, 1, |dev, vt| unsafe {
            assert_eq!((func(vt, QUERY_INTERFACE), func(vt, GET_RENDER_STATE)), (qi, grs));
            state_line(Found { slot: 0, device: dev, vtable: vt })
        }).unwrap();
        println!("{line}");
        assert!(line.contains("render target") && line.contains(" 16x16"), "{line}");
    }
}
