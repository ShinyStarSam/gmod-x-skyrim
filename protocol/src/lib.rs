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
pub const VERSION: u32 = 15; // 2: Input; 3: applied input in PlayerState; 4: tick stamps; 5: Terrain; 6: noclip_presses; 7: anchor-relative Terrain; 8: collision ring; 9: view punch; 10: Actors; 11: Hitboxes; 12: hit test; 13: hit events; 14: hurt events; 15: water_z
pub const HEARTBEAT_TIMEOUT_MS: u64 = 2000;

pub const PF_ON_GROUND: u32 = 0x1;
pub const PF_CROUCHING: u32 = 0x2;
pub const PF_ALIVE: u32 = 0x4;
pub const PF_NOCLIP: u32 = 0x8;
pub const PF_INPUT_APPLIED: u32 = 0x10; // input_frame / mouse_*_applied are valid
pub const PF_DRIVE_CAMERA: u32 = 0x20; // Skyrim may turn its camera directly (gmsky_camera 1)
pub const PF_SMOOTH: u32 = 0x40; // Skyrim may blend between ticks (gmsky_smooth 1)
pub const PF_SHAPES: u32 = 0x80; // Garry's Mod wants Skyrim's collision shapes (the ring), not the ray grid
pub const PF_CURSOR: u32 = 0x100; // a Garry's Mod menu cursor is up: Skyrim must not predict the view (step 3a)

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
pub const IN_MENU: u32 = 0x800;
pub const IN_CONSOLE: u32 = 0x1000;
pub const IN_SLOT_SHIFT: u32 = 16;
pub const IN_SLOT_MASK: u32 = 0xF0000;

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
    /// Water surface where the player is, Garry's Mod units relative to the anchor (if SF_WATER).
    pub water_z: f32,
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

/// Skyrim's actors near the player (Phase 3 step 3b), nearest first. See gmsky_protocol.h.
pub const MAX_ACTORS: usize = 64;
pub const ACT_HOSTILE: u32 = 0x1;
pub const ACT_DEAD: u32 = 0x2;
pub const ACT_ESSENTIAL: u32 = 0x4;
pub const ACT_IN_COMBAT: u32 = 0x8;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Actor {
    pub form_id: u32,
    pub flags: u32,
    /// Feet, Garry's Mod units relative to the anchor.
    pub pos: [f32; 3],
    pub yaw_deg: f32,
    pub radius: f32,
    pub height: f32,
    pub health_frac: f32,
    pub level: u16,
    pub reserved0: u16,
    /// UTF-8, NUL-terminated.
    pub name: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Actors {
    pub count: u32,
    pub origin_epoch: u32,
    pub skyrim_frame: u64,
    pub actors: [Actor; MAX_ACTORS],
}

impl Default for Actors {
    fn default() -> Self {
        Actors { count: 0, origin_epoch: 0, skyrim_frame: 0, actors: [Actor::default(); MAX_ACTORS] }
    }
}

/// The body shapes of the actors near the player (Phase 3 step 3c-2). See gmsky_protocol.h.
pub const HB_MAX_ACTORS: usize = 24;
pub const HB_MAX_PARTS: usize = 512;
pub const HBK_CAPSULE: u8 = 1;
pub const HBK_BOX: u8 = 2;
pub const HBF_APPROX: u8 = 0x1;
pub const HBF_GEAR: u8 = 0x2;
pub const HBG_GENERIC: u8 = 0;
pub const HBG_HEAD: u8 = 1;
pub const HBG_CHEST: u8 = 2;
pub const HBG_STOMACH: u8 = 3;
pub const HBG_LEFTARM: u8 = 4;
pub const HBG_RIGHTARM: u8 = 5;
pub const HBG_LEFTLEG: u8 = 6;
pub const HBG_RIGHTLEG: u8 = 7;
pub const HBG_GEAR: u8 = 10;
pub const HBA_DEAD: u32 = 0x1;
pub const HBA_IN_WORLD: u32 = 0x2;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HitPart {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub r: f32,
    pub kind: u8,
    pub group: u8,
    pub flags: u8,
    pub bone: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HitActor {
    pub form_id: u32,
    pub first_part: u16,
    pub part_count: u16,
    pub flags: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hitboxes {
    pub actor_count: u32,
    pub part_count: u32,
    pub origin_epoch: u32,
    pub reserved0: u32,
    pub skyrim_frame: u64,
    pub qpc: i64,
    pub actors: [HitActor; HB_MAX_ACTORS],
    pub parts: [HitPart; HB_MAX_PARTS],
}

impl Default for Hitboxes {
    fn default() -> Self {
        Hitboxes { actor_count: 0, part_count: 0, origin_epoch: 0, reserved0: 0, skyrim_frame: 0, qpc: 0,
                   actors: [HitActor::default(); HB_MAX_ACTORS], parts: [HitPart::default(); HB_MAX_PARTS] }
    }
}

/// Hitbox test (Phase 3 step 3c-3). See gmsky_protocol.h.
pub const HT_MAX_RAYS: usize = 441;
pub const HTH_NOTHING: u8 = 0;
pub const HTH_THIS_NPC: u8 = 1;
pub const HTH_OTHER: u8 = 2;
pub const HTH_FAILED: u8 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HitTestRay {
    pub o: [f32; 3],
    pub d: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitTestRequest {
    pub id: u32,
    pub form_id: u32,
    pub count: u32,
    pub origin_epoch: u32,
    pub rays: [HitTestRay; HT_MAX_RAYS],
}

impl Default for HitTestRequest {
    fn default() -> Self {
        HitTestRequest { id: 0, form_id: 0, count: 0, origin_epoch: 0, rays: [HitTestRay::default(); HT_MAX_RAYS] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HitTestAnswer {
    pub now_t: f32,
    pub havok_t: f32,
    pub now_group: u8,
    pub havok: u8,
    pub havok_layer: u8,
    pub reserved0: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitTestResult {
    pub id: u32,
    pub form_id: u32,
    pub count: u32,
    pub flags: u32,
    pub skyrim_frame: u64,
    pub parts: u32,
    pub reserved0: u32,
    pub answers: [HitTestAnswer; HT_MAX_RAYS],
}

impl Default for HitTestResult {
    fn default() -> Self {
        HitTestResult { id: 0, form_id: 0, count: 0, flags: 0, skyrim_frame: 0, parts: 0, reserved0: 0,
                        answers: [HitTestAnswer::default(); HT_MAX_RAYS] }
    }
}

/// Hits on Skyrim's NPCs (Phase 3 step 3d): a ring of events, see gmsky_protocol.h.
pub const EV_MAX: usize = 64;
pub const HITK_OTHER: u8 = 0;
pub const HITK_BULLET: u8 = 1;
pub const HITK_MELEE: u8 = 2;
pub const HITK_EXPLOSION: u8 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HitEvent {
    pub form_id: u32,
    pub damage: f32,
    pub gmod_damage: f32,
    pub pos: [f32; 3],
    pub dir: [f32; 3],
    pub force: f32,
    pub dmg_type: u32,
    pub origin_epoch: u32,
    pub group: u8,
    pub bone: u8,
    pub kind: u8,
    pub flags: u8,
}

#[repr(C)]
pub struct HitEvents {
    pub head: AtomicU32,
    pub session: AtomicU32,
    ev: UnsafeCell<[HitEvent; EV_MAX]>,
}

// Safety: one writer (Garry's Mod) by protocol; readers only read slots behind `head`.
unsafe impl Sync for HitEvents {}

impl HitEvents {
    /// Writer (Garry's Mod): appends an event; returns the new head.
    pub fn push(&self, e: &HitEvent) -> u32 {
        self.session.store(std::process::id(), Ordering::Relaxed);
        let h = self.head.load(Ordering::Relaxed);
        unsafe { std::ptr::write_volatile((self.ev.get() as *mut HitEvent).add(h as usize % EV_MAX), *e) };
        fence(Ordering::Release);
        self.head.store(h.wrapping_add(1), Ordering::Release);
        h.wrapping_add(1)
    }
    /// Reader (tests; Skyrim has its own): event number i (0-based).
    pub fn get(&self, i: u32) -> HitEvent {
        unsafe { std::ptr::read_volatile((self.ev.get() as *const HitEvent).add(i as usize % EV_MAX)) }
    }
}

/// SkyrimState.flags: someone is fighting the player (Phase 3 step 3f).
pub const SF_IN_COMBAT: u32 = 0x1;
/// SkyrimState.flags: water_z is valid (Phase 4 step 4b).
pub const SF_WATER: u32 = 0x2;

/// Skyrim's NPCs hurting the player (Phase 3 step 3e): a ring of events, see gmsky_protocol.h.
pub const HURT_MAX: usize = 32;
pub const HURTK_MELEE: u8 = 0;
pub const HURTK_PROJECTILE: u8 = 1;
pub const HURTK_MAGIC: u8 = 2;
pub const HURTK_OTHER: u8 = 3;
pub const HURTF_BLOCKED: u8 = 0x1;
pub const HURTF_POWER: u8 = 0x2;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HurtEvent {
    pub attacker: u32,
    pub damage: f32,
    pub source: u32,
    pub kind: u8,
    pub flags: u8,
    pub reserved: u16,
}

#[repr(C)]
pub struct HurtEvents {
    pub head: AtomicU32,
    pub session: AtomicU32,
    ev: UnsafeCell<[HurtEvent; HURT_MAX]>,
}

// Safety: one writer (Skyrim) by protocol; readers only read slots behind `head`.
unsafe impl Sync for HurtEvents {}

impl HurtEvents {
    /// Writer (Skyrim's side; here for tests): appends an event; returns the new head.
    pub fn push(&self, e: &HurtEvent) -> u32 {
        self.session.store(std::process::id(), Ordering::Relaxed);
        let h = self.head.load(Ordering::Relaxed);
        unsafe { std::ptr::write_volatile((self.ev.get() as *mut HurtEvent).add(h as usize % HURT_MAX), *e) };
        fence(Ordering::Release);
        self.head.store(h.wrapping_add(1), Ordering::Release);
        h.wrapping_add(1)
    }
    /// Reader (Garry's Mod): event number i (0-based); read only behind `head`.
    pub fn get(&self, i: u32) -> HurtEvent {
        unsafe { std::ptr::read_volatile((self.ev.get() as *const HurtEvent).add(i as usize % HURT_MAX)) }
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
    pub actors: Slot<Actors>,
    pub hitboxes: Slot<Hitboxes>,
    pub hit_test_request: Slot<HitTestRequest>,
    pub hit_test_result: Slot<HitTestResult>,
    pub hit_events: HitEvents,
    pub hurt_events: HurtEvents,
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
    assert!(size_of::<Actor>() == 72);
    assert!(offset_of!(Actor, yaw_deg) == 20);
    assert!(offset_of!(Actor, level) == 36);
    assert!(offset_of!(Actor, name) == 40);
    assert!(size_of::<Actors>() == 16 + 72 * MAX_ACTORS);
    assert!(offset_of!(Shared, actors) == 17320);
    assert!(size_of::<HitPart>() == 32);
    assert!(offset_of!(HitPart, kind) == 28);
    assert!(size_of::<HitActor>() == 16);
    assert!(offset_of!(Hitboxes, actors) == 32);
    assert!(size_of::<Hitboxes>() == 32 + 16 * HB_MAX_ACTORS + 32 * HB_MAX_PARTS);
    assert!(offset_of!(Shared, hitboxes) == 21952);
    assert!(size_of::<HitTestRay>() == 24);
    assert!(size_of::<HitTestRequest>() == 16 + 24 * HT_MAX_RAYS);
    assert!(size_of::<HitTestAnswer>() == 12);
    assert!(offset_of!(HitTestResult, answers) == 32);
    assert!(size_of::<HitTestResult>() == 32 + 12 * HT_MAX_RAYS + 4);
    assert!(offset_of!(Shared, hit_test_request) == 38760);
    assert!(offset_of!(Shared, hit_test_result) == 38760 + 8 + 10600);
    assert!(size_of::<HitEvent>() == 52);
    assert!(offset_of!(HitEvent, dmg_type) == 40);
    assert!(size_of::<HitEvents>() == 8 + 52 * EV_MAX);
    assert!(offset_of!(Shared, hit_events) == 54704);
    assert!(size_of::<HurtEvent>() == 16);
    assert!(size_of::<HurtEvents>() == 8 + 16 * HURT_MAX);
    assert!(offset_of!(Shared, hurt_events) == 54704 + 3336);
    assert!(size_of::<Shared>() == 54704 + 3336 + 520);
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
