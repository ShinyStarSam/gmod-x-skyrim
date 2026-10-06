//! Opening the shared memory block (Windows named file mapping).
//! Hand-written bindings to the few kernel32 functions we need, to avoid a large dependency.

use crate::{Shared, MAGIC, MAGIC_INIT, MAP_SIZE, VERSION};
use std::ffi::c_void;
use std::sync::atomic::Ordering;

type Handle = isize;

const INVALID_HANDLE_VALUE: Handle = -1;
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileMappingW(file: Handle, attrs: *const c_void, protect: u32, size_high: u32,
                          size_low: u32, name: *const u16) -> Handle;
    fn MapViewOfFile(mapping: Handle, access: u32, off_high: u32, off_low: u32, bytes: usize) -> *mut c_void;
    fn UnmapViewOfFile(addr: *const c_void) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount64() -> u64;
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
}

/// The PC's precise clock (QueryPerformanceCounter ticks). The same clock in every process.
pub fn now_qpc() -> i64 {
    let mut c = 0i64;
    unsafe { QueryPerformanceCounter(&mut c) };
    c
}

/// Milliseconds since Windows started. Both games run on one PC, so heartbeats compare directly.
pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// Creates the named block if it doesn't exist yet (zero-filled by Windows), otherwise opens it,
/// and maps all of it. For the overlay picture block (overlay.rs).
pub(crate) fn map_named(name: &str, size: usize) -> Result<(isize, *mut u8), String> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_READWRITE, (size as u64 >> 32) as u32,
                           size as u32, wide.as_ptr())
    };
    if handle == 0 {
        return Err(format!("CreateFileMappingW({name}) failed, error {}", unsafe { GetLastError() }));
    }
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) } as *mut u8;
    if view.is_null() {
        let err = unsafe { GetLastError() };
        unsafe { CloseHandle(handle) };
        return Err(format!("MapViewOfFile({name}) failed, error {err}"));
    }
    Ok((handle, view))
}

pub(crate) fn unmap(handle: isize, view: *mut u8) {
    unsafe {
        UnmapViewOfFile(view as *const c_void);
        CloseHandle(handle);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Skyrim,
    Gmod,
}

pub struct Mapping {
    handle: Handle,
    view: *mut Shared,
    side: Side,
}

// Safety: the view stays mapped for the lifetime of Mapping; Shared only has atomics and seqlocked slots.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    pub fn open(side: Side) -> Result<Mapping, String> {
        Self::open_named(crate::SHM_NAME, side)
    }

    /// Creates the block if it doesn't exist yet, otherwise opens it. The first side to arrive
    /// initializes the header; the other waits for it. Then registers this side's process id.
    pub fn open_named(name: &str, side: Side) -> Result<Mapping, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_READWRITE, 0,
                               MAP_SIZE as u32, wide.as_ptr())
        };
        if handle == 0 {
            return Err(format!("CreateFileMappingW({name}) failed, error {}", unsafe { GetLastError() }));
        }
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, MAP_SIZE) } as *mut Shared;
        if view.is_null() {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            return Err(format!("MapViewOfFile failed, error {err}"));
        }
        let m = Mapping { handle, view, side };
        m.init_header()?;
        // Register this process as the writer for its side. The newest process wins; an older
        // one notices through owns_side() and stops writing.
        m.side_pid().store(unsafe { GetCurrentProcessId() }, Ordering::Release);
        m.heartbeat();
        Ok(m)
    }

    fn init_header(&self) -> Result<(), String> {
        let h = &self.shared().header;
        // A new mapping is zero-filled by Windows. Whoever flips magic 0 -> INIT fills the header.
        if h.magic.compare_exchange(0, MAGIC_INIT, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            h.version.store(VERSION, Ordering::Relaxed);
            h.total_size.store(std::mem::size_of::<Shared>() as u32, Ordering::Relaxed);
            h.magic.store(MAGIC, Ordering::Release);
        } else {
            let start = now_ms();
            while h.magic.load(Ordering::Acquire) != MAGIC {
                if now_ms() - start > 1000 {
                    return Err(format!("shared memory never initialized (magic {:#x})",
                                       h.magic.load(Ordering::Acquire)));
                }
                std::thread::yield_now();
            }
        }
        let v = h.version.load(Ordering::Acquire);
        if v != VERSION {
            return Err(format!("protocol version mismatch: shared memory has {v}, this build is {VERSION}"));
        }
        Ok(())
    }

    pub fn shared(&self) -> &Shared {
        unsafe { &*self.view }
    }

    pub fn side(&self) -> Side {
        self.side
    }

    fn side_pid(&self) -> &std::sync::atomic::AtomicU32 {
        let h = &self.shared().header;
        match self.side {
            Side::Skyrim => &h.skyrim_pid,
            Side::Gmod => &h.gmod_pid,
        }
    }

    /// Each slot has exactly one writer process (the seqlock relies on it). True if this process
    /// is the registered writer for its side, re-claiming the side if its previous owner exited
    /// (pid 0). False if another process of the same side has taken over: stop writing then.
    pub fn owns_side(&self) -> bool {
        let pid = unsafe { GetCurrentProcessId() };
        let cur = self.side_pid().load(Ordering::Acquire);
        cur == pid || (cur == 0 && self.side_pid().compare_exchange(0, pid, Ordering::AcqRel, Ordering::Acquire).is_ok())
    }

    pub fn heartbeat(&self) {
        let h = &self.shared().header;
        match self.side {
            Side::Skyrim => h.skyrim_heartbeat.store(now_ms(), Ordering::Release),
            Side::Gmod => h.gmod_heartbeat.store(now_ms(), Ordering::Release),
        }
    }

    // ---- Collision ring (gmsky_protocol.h "Collision ring"; adapted from SkyCraft's Link.cpp, MIT) ----

    fn ring_base(&self) -> *mut u8 {
        unsafe { (self.view as *mut u8).add(crate::RING_OFFSET) }
    }

    fn ring_counter(&self, off: usize) -> &std::sync::atomic::AtomicU64 {
        unsafe { &*(self.ring_base().add(off) as *const std::sync::atomic::AtomicU64) }
    }

    /// Bytes written but not read yet.
    pub fn ring_pending(&self) -> u64 {
        let head = self.ring_counter(crate::RING_HEAD_OFF).load(Ordering::Acquire);
        let tail = self.ring_counter(crate::RING_TAIL_OFF).load(Ordering::Acquire);
        head.saturating_sub(tail)
    }

    /// Writer side (Skyrim; the harness plays Skyrim with it). False if the ring is too full.
    pub fn ring_write(&self, kind: u32, payload: &[u8]) -> bool {
        use crate::{ColMsgHeader, COL_PAD, RING_DATA_BYTES, RING_DATA_OFF, RING_HEAD_OFF, RING_TAIL_OFF};
        let size = RING_DATA_BYTES as u64;
        let msg = ((8 + payload.len() as u64) + 7) & !7;
        if msg > size / 2 {
            return false;
        }
        let head_ref = self.ring_counter(RING_HEAD_OFF);
        let mut head = head_ref.load(Ordering::Relaxed);
        let tail = self.ring_counter(RING_TAIL_OFF).load(Ordering::Acquire);
        let mut pos = head % size;
        let pad = if pos + msg > size { size - pos } else { 0 };
        if size - (head - tail) < msg + pad {
            return false;
        }
        let data = unsafe { self.ring_base().add(RING_DATA_OFF) };
        unsafe {
            if pad > 0 {
                std::ptr::write_unaligned(data.add(pos as usize) as *mut ColMsgHeader, ColMsgHeader { kind: COL_PAD, bytes: 0 });
                head += pad;
                pos = 0;
            }
            std::ptr::write_unaligned(data.add(pos as usize) as *mut ColMsgHeader,
                                      ColMsgHeader { kind, bytes: payload.len() as u32 });
            std::ptr::copy_nonoverlapping(payload.as_ptr(), data.add(pos as usize + 8), payload.len());
        }
        head_ref.store(head + msg, Ordering::Release);
        true
    }

    /// Reader side (Garry's Mod): calls `f(type, payload)` for each message, oldest first, until
    /// the ring is empty or about `max_bytes` were read. Returns the bytes read, or an error if
    /// the ring holds something that isn't a message (then everything pending is skipped).
    pub fn ring_read(&self, max_bytes: u64, mut f: impl FnMut(u32, &[u8])) -> Result<u64, String> {
        use crate::{ColMsgHeader, COL_PAD, RING_DATA_BYTES, RING_DATA_OFF, RING_HEAD_OFF, RING_TAIL_OFF};
        let size = RING_DATA_BYTES as u64;
        let tail_ref = self.ring_counter(RING_TAIL_OFF);
        let head = self.ring_counter(RING_HEAD_OFF).load(Ordering::Acquire);
        let start = tail_ref.load(Ordering::Relaxed);
        let mut tail = start;
        let data = unsafe { self.ring_base().add(RING_DATA_OFF) };
        while tail < head && tail - start < max_bytes {
            let pos = tail % size;
            if size - pos < 8 {
                tail += size - pos; // (never happens with 8-byte aligned messages; kept as a guard)
                continue;
            }
            let h: ColMsgHeader = unsafe { std::ptr::read_unaligned(data.add(pos as usize) as *const ColMsgHeader) };
            if h.kind == COL_PAD {
                tail += size - pos;
                continue;
            }
            let msg = ((8 + h.bytes as u64) + 7) & !7;
            if pos + msg > size || tail + msg > head {
                tail_ref.store(head, Ordering::Release);
                return Err(format!("bad message in the collision ring (type {}, {} bytes at {}); skipped {} bytes",
                                   h.kind, h.bytes, pos, head - start));
            }
            let payload = unsafe { std::slice::from_raw_parts(data.add(pos as usize + 8), h.bytes as usize) };
            f(h.kind, payload);
            tail += msg;
        }
        tail_ref.store(tail, Ordering::Release);
        Ok(tail - start)
    }

    /// Is the other game alive (heartbeat within the timeout)?
    pub fn peer_alive(&self) -> bool {
        let h = &self.shared().header;
        let last = match self.side {
            Side::Skyrim => h.gmod_heartbeat.load(Ordering::Acquire),
            Side::Gmod => h.skyrim_heartbeat.load(Ordering::Acquire),
        };
        last != 0 && now_ms().saturating_sub(last) < crate::HEARTBEAT_TIMEOUT_MS
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // Only clear the pid if it is ours: a Mapping that failed to open, or that another process
        // took over from, must not wipe the registration of the process that is really connected.
        let pid = unsafe { GetCurrentProcessId() };
        let _ = self.side_pid().compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Acquire);
        unsafe {
            UnmapViewOfFile(self.view as *const c_void);
            CloseHandle(self.handle);
        }
    }
}

/// Reads the origin Skyrim published last, without registering as either side (for tools such as
/// fake_render). None if Skyrim hasn't published one; an error if the block isn't usable.
pub fn peek_origin() -> Result<Option<crate::Origin>, String> {
    let (handle, base) = map_named(crate::SHM_NAME, MAP_SIZE)?;
    let s = unsafe { &*(base as *const Shared) };
    let (magic, version) = (s.header.magic.load(Ordering::Acquire), s.header.version.load(Ordering::Acquire));
    let r = if magic == MAGIC && version == VERSION {
        Ok(s.origin.read().map(|(o, _)| o))
    } else {
        Err(format!("link block not ready (magic {magic:#x}, version {version})"))
    };
    unmap(handle, base);
    r
}
