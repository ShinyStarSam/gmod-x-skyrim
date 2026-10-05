//! Rust copy of `protocol/gmsky_protocol.h`. The header is the source of truth.
//! The layout checks at the bottom assert the same numbers as the header's static_asserts.
//! Coordinate conversion lives only in the header (Skyrim side); Garry's Mod uses its own units.

use std::cell::UnsafeCell;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

pub mod overlay;
pub mod shm;

/// v2: the block grew (the collision ring), and an old process must not open the bigger block.
pub const SHM_NAME: &str = r"Local\GmodSkyrim_v2";
pub const SLOTS_SIZE: usize = 65536; // the latest-value slots (Shared) live here
pub const RING_OFFSET: usize = SLOTS_SIZE; // then the collision ring
pub const RING_BYTES: usize = 16 << 20;
pub const MAP_SIZE: usize = RING_OFFSET + RING_BYTES;
pub const MAGIC: u32 = 0x594B_534D;
pub const MAGIC_INIT: u32 = 0x5449_4E49;
pub const VERSION: u32 = 9; // 2: Input; 3: applied input in PlayerState; 4: tick stamps; 5: Terrain; 6: noclip_presses; 7: anchor-relative Terrain; 8: collision ring; 9: view punch
pub const HEARTBEAT_TIMEOUT_MS: u64 = 2000;

pub const PF_ON_GROUND: u32 = 0x1;
pub const PF_CROUCHING: u32 = 0x2;
pub const PF_ALIVE: u32 = 0x4;
pub const PF_NOCLIP: u32 = 0x8;
pub const PF_INPUT_APPLIED: u32 = 0x10; // input_frame / mouse_*_applied are valid
pub const PF_DRIVE_CAMERA: u32 = 0x20; // Skyrim may turn its camera directly (gmsky_camera 1)
pub const PF_SMOOTH: u32 = 0x40; // Skyrim may blend between ticks (gmsky_smooth 1)
pub const PF_SHAPES: u32 = 0x80; // Garry's Mod wants Skyrim's collision shapes (the ring), not the ray grid

// ---- Collision ring (gmsky_protocol.h "Collision ring"; adapted from SkyCraft, MIT) ----
pub const RING_HEAD_OFF: usize = 0x00;
pub const RING_TAIL_OFF: usize = 0x40;
pub const RING_DATA_OFF: usize = 0x80;
pub const RING_DATA_BYTES: usize = RING_BYTES - RING_DATA_OFF;
pub const COL_PAD: u32 = 0;
pub const COL_CLEAR: u32 = 1;
pub const COL_REGION: u32 = 2;
pub const TRI_STAIR_HELPER: u32 = 0x1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColMsgHeader {
    pub kind: u32,
    pub bytes: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColClear {
    pub col_epoch: u32,
    pub origin_epoch: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColRegion {
    pub rx: i32,
    pub ry: i32,
    pub rz: i32,
    pub col_epoch: u32,
    pub count: u32,
    pub flags: u32,
    pub lo: [f32; 3],
    pub size: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColTri {
    pub v: [f32; 9],
    pub flags: u32,
}

pub const IN_FORWARD: u32 = 0x1;
pub const IN_BACK: u32 = 0x2;
pub const IN_LEFT: u32 = 0x4;
pub const IN_RIGHT: u32 = 0x8;
pub const IN_JUMP: u32 = 0x10;
pub const IN_DUCK: u32 = 0x20;
pub const IN_SPEED: u32 = 0x40;
pub const IN_WALK: u32 = 0x80;
pub const IN_ATTACK: u32 = 0x100;
pub const IN_ATTACK2: u32 = 0x200;
pub const IN_RELOAD: u32 = 0x400;

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
    /// Input from Skyrim already turned into this state (valid with PF_INPUT_APPLIED).
    pub input_frame: u64,
    pub mouse_x_applied: i64,
    pub mouse_y_applied: i64,
    /// Degrees per mouse count; 0 = Skyrim must not predict.
    pub deg_per_count: f32,
    pub reserved0: u32,
    /// When this tick was written (QueryPerformanceCounter, same clock in both processes); 0 = unknown.
    pub tick_qpc: i64,
    /// Tick length in ms.
    pub tick_ms: f32,
    pub reserved1: u32,
    /// View punch (pitch, yaw, roll degrees), added to the camera only.
    pub punch_deg: [f32; 3],
    pub reserved2: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Input {
    pub frame: u64,
    pub active: u32,
    pub buttons: u32,
    pub mouse_x_total: i64,
    pub mouse_y_total: i64,
    pub wheel_total: i32,
    /// Noclip key (V) presses since Skyrim started; each new one toggles noclip.
    pub noclip_presses: u32,
}

/// Skyrim's ground around the player, in Garry's Mod units (see the C header).
pub const TERRAIN_N: usize = 65;
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Terrain {
    pub id: u32,
    pub origin_epoch: u32,
    pub n: u32,
    pub misses: u32,
    pub center: [f64; 2],
    pub spacing: f32,
    pub reserved0: f32,
    pub skyrim_frame: u64,
    /// Garry's Mod z relative to the anchor (protocol 7; center too); NaN = no hit. Index i * n + j.
    pub height: [f32; TERRAIN_N * TERRAIN_N],
    pub reserved1: u32,
}

impl Default for Terrain {
    fn default() -> Self {
        Terrain { id: 0, origin_epoch: 0, n: 0, misses: 0, center: [0.0; 2], spacing: 0.0, reserved0: 0.0,
                  skyrim_frame: 0, height: [0.0; TERRAIN_N * TERRAIN_N], reserved1: 0 }
    }
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
    pub input: Slot<Input>,
    pub terrain: Slot<Terrain>,
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
    assert!(size_of::<PlayerState>() == 160);
    assert!(offset_of!(PlayerState, punch_deg) == 144);
    assert!(offset_of!(PlayerState, anchor) == 16);
    assert!(offset_of!(PlayerState, pos) == 40);
    assert!(offset_of!(PlayerState, eye_height) == 64);
    assert!(offset_of!(PlayerState, vel) == 80);
    assert!(offset_of!(PlayerState, fov_deg) == 92);
    assert!(offset_of!(PlayerState, input_frame) == 96);
    assert!(offset_of!(PlayerState, deg_per_count) == 120);
    assert!(offset_of!(PlayerState, tick_qpc) == 128);
    assert!(offset_of!(PlayerState, tick_ms) == 136);
    assert!(size_of::<Input>() == 40);
    assert!(offset_of!(Input, mouse_x_total) == 16);
    assert!(offset_of!(Input, wheel_total) == 32);
    assert!(offset_of!(Input, noclip_presses) == 36);
    assert!(offset_of!(Slot<Origin>, data) == 8);
    assert!(offset_of!(Shared, origin) == 40);
    assert!(offset_of!(Shared, skyrim_state) == 80);
    assert!(offset_of!(Shared, player_state) == 144);
    assert!(offset_of!(Shared, input) == 312);
    assert!(size_of::<Terrain>() == 16952);
    assert!(offset_of!(Terrain, center) == 16);
    assert!(offset_of!(Terrain, skyrim_frame) == 40);
    assert!(offset_of!(Terrain, height) == 48);
    assert!(offset_of!(Shared, terrain) == 360);
    assert!(size_of::<Shared>() == 17320);
    assert!(size_of::<Shared>() <= SLOTS_SIZE);
    assert!(size_of::<ColMsgHeader>() == 8);
    assert!(size_of::<ColClear>() == 8);
    assert!(size_of::<ColRegion>() == 40);
    assert!(offset_of!(ColRegion, lo) == 24);
    assert!(size_of::<ColTri>() == 40);
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

    /// The collision ring: messages come out in order and whole, across the wrap-around (pad),
    /// and a full ring refuses writes instead of overwriting unread data.
    #[test]
    fn collision_ring_wraps_and_fills() {
        let name = unique_name("ring");
        let w = Mapping::open_named(&name, Side::Skyrim).unwrap();
        let r = Mapping::open_named(&name, Side::Gmod).unwrap();
        // ~1 MB messages: 16 fill the ring; read some, write more, so writes wrap around.
        let big = |n: u32| -> Vec<u8> { let mut v = vec![(n % 251) as u8; 1_000_003]; v[0..4].copy_from_slice(&n.to_le_bytes()); v };
        let mut next_w = 0u32;
        let mut next_r = 0u32;
        let mut refused = 0;
        for round in 0..6 {
            loop {
                if w.ring_write(COL_REGION, &big(next_w)) { next_w += 1 } else { refused += 1; break }
            }
            let mut got = 0;
            let max = if round % 2 == 0 { 5_000_000 } else { u64::MAX };
            r.ring_read(max, |kind, p| {
                assert_eq!(kind, COL_REGION);
                assert_eq!(p.len(), 1_000_003);
                let n = u32::from_le_bytes(p[0..4].try_into().unwrap());
                assert_eq!(n, next_r, "messages in order");
                assert!(p[4..].iter().all(|&b| b == (n % 251) as u8), "payload intact");
                next_r += 1;
                got += 1;
            }).unwrap();
            assert!(got > 0);
        }
        r.ring_read(u64::MAX, |_, _| next_r += 1).unwrap();
        assert_eq!(next_r, next_w);
        assert_eq!(r.ring_pending(), 0);
        assert!(refused == 6 && next_w > 40, "ring filled each round ({next_w} written)");
        // Too big for half the ring: refused.
        assert!(!w.ring_write(COL_REGION, &vec![0u8; RING_DATA_BYTES / 2]));
    }
}
