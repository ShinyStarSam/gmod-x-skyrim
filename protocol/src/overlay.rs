//! Rust copy of the overlay picture block in `protocol/gmsky_protocol.h` ("Overlay picture block";
//! the header is the source of truth, the layout checks below assert the same numbers).
//!
//! Garry's Mod's finished picture goes into one of three slots (a triple buffer, adapted from
//! SkyCraft, MIT). The whole arrangement is one 32-bit word changed only by compare-and-swap, so
//! the three slot numbers stay a permutation of 0, 1, 2 even when a game restarts or crashes:
//! bits 0-1 middle slot, bit 2 "new picture", bits 4-5 / 6 writer's slot / attached,
//! bits 8-9 / 10 reader's slot / attached. Garry's Mod writes; Skyrim reads (the unit tests and
//! tools/overlay_read.py play Skyrim).

use crate::shm::{map_named, unmap};
use crate::{MAGIC_INIT};
use std::mem::{offset_of, size_of};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::*};

pub const SHM_NAME: &str = r"Local\GmodSkyrim_overlay_v1";
pub const MAGIC: u32 = 0x564F_534D; // "MSOV"
pub const VERSION: u32 = 1;
pub const MAX_W: u32 = 3840;
pub const MAX_H: u32 = 2160;
pub const MASK_MAX_W: u32 = MAX_W / 4;
pub const MASK_MAX_H: u32 = MAX_H / 4;
pub const PIXELS_OFF: usize = 0x1000;
pub const MASK_OFF: usize = (MAX_W * MAX_H * 4) as usize;
pub const SLOT_BYTES: usize = (MASK_OFF + (MASK_MAX_W * MASK_MAX_H) as usize + 0xFFFF) & !0xFFFF;
pub const MAP_SIZE: usize = PIXELS_OFF + 3 * SLOT_BYTES;
pub const SLOT_HDR_OFF: usize = 0x40;

pub const S_MIDDLE_MASK: u32 = 0x3;
pub const S_NEW: u32 = 0x4;
pub const S_WRITER_SHIFT: u32 = 4;
pub const S_WRITER_ON: u32 = 0x40;
pub const S_READER_SHIFT: u32 = 8;
pub const S_READER_ON: u32 = 0x400;

pub const FMT_BGRA8: u32 = 1;
pub const F_PREMULTIPLIED: u32 = 0x1;
pub const F_SRGB: u32 = 0x2;
/// Skyrim blurs its picture behind see-through parts (gmsky_overlay_blur).
pub const F_BLUR: u32 = 0x4;

#[repr(C)]
pub struct Ctl {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    pub map_size: AtomicU64,
    pub state: AtomicU32,
    /// Garry's Mod adds 1 (gmsky_overlay_shot): Skyrim saves its next drawn frame (step 2c-1).
    pub shot_request: AtomicU32,
    pub frames_published: AtomicU64,
    pub frames_taken: AtomicU64,
    pub skyrim_width: AtomicU32,
    pub skyrim_height: AtomicU32,
    pub skyrim_size_qpc: AtomicU64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotHdr {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub flags: u32,
    pub frame: u64,
    pub capture_qpc: i64,
    pub row_bytes: u32,
    pub mask_width: u32,
    pub mask_height: u32,
    pub reserved0: u32,
    pub reserved: [u8; 16],
}

const _: () = {
    assert!(size_of::<Ctl>() == 56);
    assert!(offset_of!(Ctl, state) == 16);
    assert!(offset_of!(Ctl, frames_published) == 24);
    assert!(offset_of!(Ctl, skyrim_width) == 40);
    assert!(offset_of!(Ctl, skyrim_size_qpc) == 48);
    assert!(size_of::<SlotHdr>() == 64);
    assert!(offset_of!(SlotHdr, frame) == 16);
    assert!(offset_of!(SlotHdr, capture_qpc) == 24);
    assert!(offset_of!(SlotHdr, row_bytes) == 32);
    assert!(size_of::<Ctl>() <= SLOT_HDR_OFF);
    assert!(SLOT_HDR_OFF + 3 * size_of::<SlotHdr>() <= PIXELS_OFF);
    assert!(SLOT_BYTES == 33_751_040);
    assert!(MAP_SIZE == 101_257_216);
};

// ---- The rules, as pure functions on the state word (the same as gmsky::ov in the header) ----

/// Attaches the writer (`writer`) or the reader; returns (new state, the caller's slot).
pub fn attach(s: u32, writer: bool) -> (u32, u32) {
    let (shift, on, oshift, oon) = if writer {
        (S_WRITER_SHIFT, S_WRITER_ON, S_READER_SHIFT, S_READER_ON)
    } else {
        (S_READER_SHIFT, S_READER_ON, S_WRITER_SHIFT, S_WRITER_ON)
    };
    let m = s & S_MIDDLE_MASK;
    let own = if s & on != 0 {
        (s >> shift) & 3 // a restarted side takes back the slot it had
    } else if s & oon != 0 {
        3 - m - ((s >> oshift) & 3) // the one neither the middle nor the other side has
    } else {
        (m + if writer { 1 } else { 2 }) % 3 // nobody else: writer and reader pick different ones
    };
    ((s & !(3 << shift)) | (own << shift) | on, own)
}

/// Writer: its slot becomes the middle (with "new picture"); returns (new state, its new slot).
pub fn publish(s: u32) -> (u32, u32) {
    let (w, m) = ((s >> S_WRITER_SHIFT) & 3, s & S_MIDDLE_MASK);
    ((s & !(S_MIDDLE_MASK | (3 << S_WRITER_SHIFT))) | w | (m << S_WRITER_SHIFT) | S_NEW, m)
}

/// Reader: if there is a new picture, the middle becomes its slot; returns (new state, that slot).
pub fn take(s: u32) -> Option<(u32, u32)> {
    if s & S_NEW == 0 {
        return None;
    }
    let (r, m) = ((s >> S_READER_SHIFT) & 3, s & S_MIDDLE_MASK);
    Some(((s & !(S_MIDDLE_MASK | S_NEW | (3 << S_READER_SHIFT))) | r | (m << S_READER_SHIFT), m))
}

pub fn detach(s: u32, writer: bool) -> u32 {
    s & !if writer { S_WRITER_ON } else { S_READER_ON }
}

/// The three slot numbers are 0, 1, 2 in some order.
pub fn is_permutation(s: u32) -> bool {
    let (m, w, r) = (s & 3, (s >> S_WRITER_SHIFT) & 3, (s >> S_READER_SHIFT) & 3);
    m < 3 && w < 3 && r < 3 && m != w && m != r && w != r
}

/// The picture block, mapped.
pub struct OverlayMapping {
    handle: isize,
    base: *mut u8,
    writer: Option<bool>,
}

// Safety: the view stays mapped for the lifetime of OverlayMapping; shared fields are atomics, and
// pixels are only written by the side that owns the slot (the state word decides).
unsafe impl Send for OverlayMapping {}
unsafe impl Sync for OverlayMapping {}

impl OverlayMapping {
    pub fn open() -> Result<OverlayMapping, String> {
        Self::open_named(SHM_NAME)
    }

    /// Creates the block if it doesn't exist yet, otherwise opens it; the first side to arrive
    /// fills in the header (as the link block does).
    pub fn open_named(name: &str) -> Result<OverlayMapping, String> {
        let (handle, base) = map_named(name, MAP_SIZE)?;
        let m = OverlayMapping { handle, base, writer: None };
        let c = m.ctl();
        if c.magic.compare_exchange(0, MAGIC_INIT, AcqRel, Acquire).is_ok() {
            c.version.store(VERSION, Relaxed);
            c.map_size.store(MAP_SIZE as u64, Relaxed);
            c.magic.store(MAGIC, Release);
        } else {
            let start = crate::shm::now_ms();
            while c.magic.load(Acquire) != MAGIC {
                if crate::shm::now_ms() - start > 1000 {
                    return Err(format!("overlay block never initialized (magic {:#x})", c.magic.load(Acquire)));
                }
                std::thread::yield_now();
            }
        }
        let (v, size) = (c.version.load(Acquire), c.map_size.load(Acquire));
        if v != VERSION || size != MAP_SIZE as u64 {
            return Err(format!("overlay block mismatch: version {v} size {size}, this build {VERSION} / {MAP_SIZE}"));
        }
        Ok(m)
    }

    pub fn ctl(&self) -> &Ctl {
        unsafe { &*(self.base as *const Ctl) }
    }

    pub fn state(&self) -> u32 {
        self.ctl().state.load(Acquire)
    }

    /// Applies a rule to the shared word with compare-and-swap; returns what the rule returned.
    fn update<R>(&self, mut rule: impl FnMut(u32) -> Option<(u32, R)>) -> Option<R> {
        let st = &self.ctl().state;
        let mut s = st.load(Acquire);
        loop {
            let (next, r) = rule(s)?;
            match st.compare_exchange_weak(s, next, AcqRel, Acquire) {
                Ok(_) => return Some(r),
                Err(cur) => s = cur,
            }
        }
    }

    /// Becomes the writer (Garry's Mod) or the reader (Skyrim). Returns this side's slot.
    pub fn attach(&mut self, writer: bool) -> u32 {
        self.writer = Some(writer);
        self.update(|s| Some(attach(s, writer))).unwrap()
    }

    /// This side's slot now (each side's slot changes only through its own calls).
    pub fn own_slot(&self) -> u32 {
        let s = self.state();
        match self.writer {
            Some(true) => (s >> S_WRITER_SHIFT) & 3,
            _ => (s >> S_READER_SHIFT) & 3,
        }
    }

    pub fn slot_hdr(&self, slot: u32) -> SlotHdr {
        unsafe { std::ptr::read_volatile(self.base.add(SLOT_HDR_OFF + slot as usize * size_of::<SlotHdr>()) as *const SlotHdr) }
    }

    /// The pixels of a slot (MAX_W * MAX_H * 4 bytes; the plate mask follows at MASK_OFF).
    /// Safety: write only into the writer's own slot; read only the reader's own slot.
    pub unsafe fn slot_ptr(&self, slot: u32) -> *mut u8 {
        self.base.add(PIXELS_OFF + slot as usize * SLOT_BYTES)
    }

    /// Writer: the picture in its slot is finished; `hdr` describes it. Makes it the newest
    /// picture and returns the writer's next slot.
    pub fn publish(&self, hdr: &SlotHdr) -> u32 {
        debug_assert_eq!(self.writer, Some(true));
        let own = self.own_slot();
        unsafe {
            std::ptr::write_volatile(self.base.add(SLOT_HDR_OFF + own as usize * size_of::<SlotHdr>()) as *mut SlotHdr, *hdr);
        }
        let next = self.update(|s| Some(publish(s))).unwrap();
        self.ctl().frames_published.fetch_add(1, Relaxed);
        next
    }

    /// Reader: if Garry's Mod published a picture since the last take, it becomes the reader's
    /// slot (returned) until the next take.
    pub fn take(&self) -> Option<u32> {
        debug_assert_eq!(self.writer, Some(false));
        let slot = self.update(take)?;
        self.ctl().frames_taken.fetch_add(1, Relaxed);
        Some(slot)
    }

    /// Reader (Skyrim, step 2b-4): its back buffer size.
    pub fn set_skyrim_size(&self, w: u32, h: u32) {
        let c = self.ctl();
        c.skyrim_width.store(w, Relaxed);
        c.skyrim_height.store(h, Relaxed);
        c.skyrim_size_qpc.store(crate::shm::now_qpc() as u64, Release);
    }

    pub fn skyrim_size(&self) -> (u32, u32) {
        let c = self.ctl();
        (c.skyrim_width.load(Acquire), c.skyrim_height.load(Acquire))
    }
}

impl Drop for OverlayMapping {
    fn drop(&mut self) {
        if let Some(writer) = self.writer {
            self.update(|s| Some((detach(s, writer), ())));
        }
        unmap(self.handle, self.base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn unique_name(tag: &str) -> String {
        format!(r"Local\GmodSkyrim_overlay_test_{}_{}", std::process::id(), tag)
    }

    #[test]
    fn rules_keep_a_permutation() {
        // Every order of attach / publish / take / restart from a zero block, exhaustively for a
        // few steps: the three slots never overlap once both sides are attached.
        fn walk(s: u32, depth: u32) {
            let both = s & S_WRITER_ON != 0 && s & S_READER_ON != 0;
            if both {
                assert!(is_permutation(s), "state {s:#x}");
            }
            if depth == 0 {
                return;
            }
            walk(attach(s, true).0, depth - 1);
            walk(attach(s, false).0, depth - 1);
            walk(attach(detach(s, true), true).0, depth - 1);
            walk(attach(detach(s, false), false).0, depth - 1);
            if s & S_WRITER_ON != 0 {
                walk(publish(s).0, depth - 1);
            }
            if s & S_READER_ON != 0 {
                if let Some((n, slot)) = take(s) {
                    assert_eq!(slot, s & 3, "the reader gets the newest picture");
                    walk(n, depth - 1);
                }
            }
        }
        walk(0, 7);
        // The C++ test's sequence, the same numbers.
        let (s, w) = attach(0, true);
        let (s, r) = attach(s, false);
        assert_eq!((w, r), (1, 2));
        let (s, w) = publish(s);
        assert_eq!((s & 3, w), (1, 0));
        assert_eq!(take(s).map(|(_, slot)| slot), Some(1));
    }

    /// Step 2b-2's test: a writer thread and a reader thread hammering the block. Every picture
    /// the reader takes must be whole: every pixel from the frame its header names, frames only
    /// ever newer.
    #[test]
    fn reader_never_gets_a_half_written_picture() {
        let name = unique_name("hammer");
        let mut w = OverlayMapping::open_named(&name).unwrap();
        let mut r = OverlayMapping::open_named(&name).unwrap();
        w.attach(true);
        r.attach(false);
        let stop = Arc::new(AtomicBool::new(false));
        let frames = 20_000u64;
        let writer = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                for frame in 1..=frames {
                    // Sizes change too (a resolution change), up to 64 KB pictures.
                    let (wd, ht) = (64 + (frame % 7) as u32 * 16, 32 + (frame % 5) as u32 * 8);
                    let slot = w.own_slot();
                    let px = unsafe { std::slice::from_raw_parts_mut(w.slot_ptr(slot) as *mut u32, (wd * ht) as usize) };
                    let half = px.len() / 2;
                    for (i, p) in px.iter_mut().enumerate() {
                        // Written slowly in two halves, so a reader inside this slot would see a mix.
                        *p = frame as u32;
                        if i == half { std::thread::yield_now(); }
                    }
                    w.publish(&SlotHdr { width: wd, height: ht, format: FMT_BGRA8, flags: F_PREMULTIPLIED,
                        frame, row_bytes: wd * 4, ..Default::default() });
                }
                stop.store(true, Release);
                w
            })
        };
        let (mut taken, mut last, mut empty) = (0u64, 0u64, 0u64);
        loop {
            let done = stop.load(Acquire);
            match r.take() {
                Some(slot) => {
                    let h = r.slot_hdr(slot);
                    assert!(h.frame > last, "frame {} after {last}", h.frame);
                    let px = unsafe { std::slice::from_raw_parts(r.slot_ptr(slot) as *const u32, (h.width * h.height) as usize) };
                    // Read twice, with a pause: a writer in this slot would change it meanwhile.
                    for pass in 0..2 {
                        if let Some(bad) = px.iter().position(|&p| p != h.frame as u32) {
                            panic!("picture {} not whole (pass {pass}): pixel {bad} is {}", h.frame, px[bad]);
                        }
                        std::thread::yield_now();
                    }
                    last = h.frame;
                    taken += 1;
                }
                None if done => break,
                None => empty += 1,
            }
        }
        let w = writer.join().unwrap();
        assert!(is_permutation(r.state()));
        assert_eq!(last, frames, "the reader ends with the newest picture");
        assert_eq!(w.ctl().frames_published.load(Relaxed), frames);
        assert_eq!(r.ctl().frames_taken.load(Relaxed), taken);
        println!("{frames} pictures written, {taken} taken whole, {empty} empty polls; slots {:#x}", r.state());
    }

    /// A game restarting: the other side keeps working, slots stay apart, the new process
    /// gets the same slot back.
    #[test]
    fn restart_keeps_slots_apart() {
        let name = unique_name("restart");
        let mut r = OverlayMapping::open_named(&name).unwrap();
        let reader_slot = r.attach(false);
        {
            let mut w = OverlayMapping::open_named(&name).unwrap();
            w.attach(true);
            w.publish(&SlotHdr { frame: 1, ..Default::default() });
            assert!(is_permutation(w.state()));
        } // Garry's Mod quits (detaches)
        assert_eq!(r.take().map(|s| r.slot_hdr(s).frame), Some(1));
        let mut w = OverlayMapping::open_named(&name).unwrap();
        w.attach(true);
        assert!(is_permutation(w.state()));
        w.publish(&SlotHdr { frame: 2, ..Default::default() });
        assert_eq!(r.take().map(|s| r.slot_hdr(s).frame), Some(2));
        assert_ne!(reader_slot, 3);
        r.set_skyrim_size(1920, 1080);
        assert_eq!(w.skyrim_size(), (1920, 1080));
        // A block made by another version is refused, not misread.
        w.ctl().version.store(99, Relaxed);
        assert!(OverlayMapping::open_named(&name).is_err());
        w.ctl().version.store(VERSION, Relaxed);
    }
}
