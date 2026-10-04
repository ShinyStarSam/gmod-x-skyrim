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
