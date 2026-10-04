//! Rust copy of `protocol/gmsky_protocol.h`. The header is the source of truth.
//! The layout checks at the bottom assert the same numbers as the header's static_asserts.
//! Coordinate conversion lives only in the header (Skyrim side); Garry's Mod uses its own units.

use std::cell::UnsafeCell;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

pub mod shm;

pub const SHM_NAME: &str = r"Local\GmodSkyrim_v1";
pub const MAP_SIZE: usize = 65536;
pub const MAGIC: u32 = 0x594B_534D;
pub const MAGIC_INIT: u32 = 0x5449_4E49;
pub const VERSION: u32 = 1;
pub const HEARTBEAT_TIMEOUT_MS: u64 = 2000;

pub const PF_ON_GROUND: u32 = 0x1;
pub const PF_CROUCHING: u32 = 0x2;
pub const PF_ALIVE: u32 = 0x4;
pub const PF_NOCLIP: u32 = 0x8;

#[repr(C)]
pub struct Header {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    pub total_size: AtomicU32,
    pub reserved0: u32,
    pub skyrim_pid: AtomicU32,
    pub gmod_pid: AtomicU32,
    pub skyrim_heartbeat: AtomicU64,
    pub gmod_heartbeat: AtomicU64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Origin {
    pub epoch: u32,
    pub worldspace_form_id: u32,
    pub skyrim_origin: [f64; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct SkyrimState {
    pub frame: u64,
    pub pos: [f64; 3],
    pub heading_rad: f32,
    pub pitch_rad: f32,
    pub height: f32,
    pub cell_form_id: u32,
    pub flags: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct PlayerState {
    pub tick: u64,
    pub origin_epoch: u32,
    pub flags: u32,
    pub anchor: [f64; 3],
    pub pos: [f64; 3],
    pub eye_height: f32,
    pub pitch_deg: f32,
    pub yaw_deg: f32,
    pub roll_deg: f32,
    pub vel: [f32; 3],
    pub fov_deg: f32,
}

/// A latest-value slot guarded by a seqlock. `seq == 0` means never written.
#[repr(C)]
pub struct Slot<T> {
    seq: AtomicU32,
    reserved0: u32,
    data: UnsafeCell<T>,
}

// Safety: access to `data` is coordinated by the seqlock; one writer per slot by protocol.
unsafe impl<T: Copy> Sync for Slot<T> {}

impl<T: Copy> Slot<T> {
    /// Only one side may write a given slot (see the header comments).
    pub fn write(&self, value: &T) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        unsafe { std::ptr::write_volatile(self.data.get(), *value) };
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    /// Returns the value and its sequence number, or None if never written or the writer
    /// kept it busy for every attempt.
    pub fn read(&self) -> Option<(T, u32)> {
        for _ in 0..64 {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 == 0 {
                return None;
            }
            if s1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let v = unsafe { std::ptr::read_volatile(self.data.get()) };
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return Some((v, s1));
            }
        }
        None
    }

    pub fn seq(&self) -> u32 {
        self.seq.load(Ordering::Acquire)
    }
}

#[repr(C)]
pub struct Shared {
    pub header: Header,
    pub origin: Slot<Origin>,
    pub skyrim_state: Slot<SkyrimState>,
    pub player_state: Slot<PlayerState>,
}

// ---- Layout checks: the same numbers as gmsky_protocol.h ----
use std::mem::{offset_of, size_of};
const _: () = {
    assert!(size_of::<Header>() == 40);
    assert!(offset_of!(Header, skyrim_pid) == 16);
    assert!(offset_of!(Header, skyrim_heartbeat) == 24);
    assert!(offset_of!(Header, gmod_heartbeat) == 32);
    assert!(size_of::<Origin>() == 32);
    assert!(offset_of!(Origin, skyrim_origin) == 8);
    assert!(size_of::<SkyrimState>() == 56);
    assert!(offset_of!(SkyrimState, heading_rad) == 32);
    assert!(offset_of!(SkyrimState, cell_form_id) == 44);
    assert!(size_of::<PlayerState>() == 96);
    assert!(offset_of!(PlayerState, anchor) == 16);
    assert!(offset_of!(PlayerState, pos) == 40);
    assert!(offset_of!(PlayerState, eye_height) == 64);
    assert!(offset_of!(PlayerState, vel) == 80);
    assert!(offset_of!(PlayerState, fov_deg) == 92);
    assert!(offset_of!(Slot<Origin>, data) == 8);
    assert!(offset_of!(Shared, origin) == 40);
    assert!(offset_of!(Shared, skyrim_state) == 80);
    assert!(offset_of!(Shared, player_state) == 144);
    assert!(size_of::<Shared>() == 248);
    assert!(size_of::<Shared>() <= MAP_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;
    use shm::{Mapping, Side};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn unique_name(tag: &str) -> String {
        format!(r"Local\GmodSkyrim_test_{}_{}", tag, std::process::id())
    }

    #[test]
    fn never_written_slot_reads_none() {
        let m = Mapping::open_named(&unique_name("none"), Side::Gmod).unwrap();
        assert!(m.shared().player_state.read().is_none());
    }

    #[test]
    fn two_mappings_see_the_same_memory() {
        let name = unique_name("two");
        let a = Mapping::open_named(&name, Side::Gmod).unwrap();
        let b = Mapping::open_named(&name, Side::Skyrim).unwrap();
        let ps = PlayerState { tick: 7, pos: [1.0, 2.0, 3.0], ..Default::default() };
        a.shared().player_state.write(&ps);
        let (got, seq) = b.shared().player_state.read().unwrap();
        assert_eq!(got, ps);
        assert_eq!(seq, 2);
        let h = &b.shared().header;
        assert_eq!(h.magic.load(Ordering::Acquire), MAGIC);
        assert_eq!(h.version.load(Ordering::Acquire), VERSION);
        assert_eq!(h.total_size.load(Ordering::Acquire) as usize, size_of::<Shared>());
        assert_eq!(h.gmod_pid.load(Ordering::Acquire), std::process::id());
        assert_eq!(h.skyrim_pid.load(Ordering::Acquire), std::process::id());
    }

    #[test]
    fn owns_side_and_drop_only_clears_own_pid() {
        let name = unique_name("own");
        let a = Mapping::open_named(&name, Side::Gmod).unwrap();
        assert!(a.owns_side());
        // Another process takes over the Gmod side: we no longer own it, and dropping our
        // mapping must not clear its pid.
        let h = &a.shared().header;
        h.gmod_pid.store(12345, Ordering::Release);
        assert!(!a.owns_side());
        let b = Mapping::open_named(&name, Side::Skyrim).unwrap();
        drop(a);
        assert_eq!(b.shared().header.gmod_pid.load(Ordering::Acquire), 12345);
        // The other process exits (pid 0): a remaining writer may re-claim the side.
        b.shared().header.gmod_pid.store(0, Ordering::Release);
        let c = Mapping::open_named(&name, Side::Gmod).unwrap();
        b.shared().header.gmod_pid.store(0, Ordering::Release);
        assert!(c.owns_side());
        assert_eq!(b.shared().header.gmod_pid.load(Ordering::Acquire), std::process::id());
    }

    /// A reader must never see a half-written value while a writer hammers the slot.
    #[test]
    fn seqlock_never_tears() {
        let name = unique_name("tear");
        let w = Arc::new(Mapping::open_named(&name, Side::Gmod).unwrap());
        let r = Mapping::open_named(&name, Side::Skyrim).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let (w2, stop2) = (w.clone(), stop.clone());
        let writer = std::thread::spawn(move || {
            let mut i = 0u64;
            while !stop2.load(Ordering::Relaxed) {
                i += 1;
                let f = i as f64;
                let ps = PlayerState { tick: i, anchor: [f; 3], pos: [f; 3], ..Default::default() };
                w2.shared().player_state.write(&ps);
            }
            i
        });
        let mut reads = 0u64;
        let mut last_tick = 0u64;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            if let Some((ps, _)) = r.shared().player_state.read() {
                let f = ps.tick as f64;
                assert!(ps.pos == [f; 3] && ps.anchor == [f; 3], "torn read: {ps:?}");
                assert!(ps.tick >= last_tick, "went backwards");
                last_tick = ps.tick;
                reads += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        let writes = writer.join().unwrap();
        println!("seqlock stress: {writes} writes, {reads} consistent reads");
        assert!(writes > 1000 && reads > 1000);
    }
}
