//! S2k-K1 (DESIGN.md "S2k"): Skyrim's loose objects (clutter, dropped weapons, moveable statics)
//! near the player, as Skyrim sends them through the collision ring (GMSKY_COL_LOOSE /
//! GMSKY_COL_LOOSE_GONE). Lives inside shapes::Store, so it is cleared with the regions and both
//! Garry's Mod sides (one module per process) read the same objects.
//!
//! Each Skyrim body gets a small number `n` (Lua numbers can't hold its 64-bit id exactly). Every
//! change bumps a counter, so each side can ask "what changed since I last looked".
//! Coordinates: Garry's Mod units, positions relative to the anchor, points in the body's frame.

use gmsky_protocol::{ColLoose, ColLooseGone, LOOSE_SHAPE};
use std::collections::{HashMap, VecDeque};

/// Removals remembered for the "since" question; a side further behind starts over (resync).
const GONE_KEPT: usize = 65536;

pub struct Obj {
    pub n: u32,
    pub form_id: u32,
    pub flags: u32,
    pub layer: u16,
    pub motion: u8,
    pub pos: [f64; 3],
    pub rot: [f64; 9],
    pub vel: [f64; 3],
    pub mass: f64,
    /// Each piece: points (convex hull), body frame.
    pub pieces: Vec<Vec<[f32; 3]>>,
    pub points: usize,
    /// The change counter at its last change.
    pub ver: u64,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub added: u64,
    pub moved: u64,
    pub gone: u64,
    /// Pose messages for a body whose shape never arrived (its first message was dropped).
    pub unknown: u64,
    pub bad: u64,
    pub ignored: u64,
}

#[derive(Default)]
pub struct Store {
    by_id: HashMap<u64, u32>,
    objs: HashMap<u32, Obj>,
    /// (change counter, n) of removals, oldest first.
    gone: VecDeque<(u64, u32)>,
    next_n: u32,
    ver: u64,
    /// The change counter of the newest removal no longer remembered.
    gone_dropped: u64,
    /// +1 on every clear: a side that saw another generation starts over.
    pub generation: u32,
    pub stats: Stats,
}

fn read<T: Copy>(p: &[u8], off: usize) -> Option<T> {
    if off + std::mem::size_of::<T>() > p.len() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(p.as_ptr().add(off) as *const T) })
}

/// What changed since a side last looked.
pub struct Changes<'a> {
    pub generation: u32,
    pub ver: u64,
    /// The side must drop everything and take `changed` as the whole list.
    pub resync: bool,
    pub changed: Vec<&'a Obj>,
    pub gone: Vec<u32>,
}

impl Store {
    pub fn len(&self) -> usize {
        self.objs.len()
    }

    pub fn clear(&mut self) {
        self.by_id.clear();
        self.objs.clear();
        self.gone.clear();
        self.generation += 1;
    }

    /// Pieces and points of all objects together.
    pub fn totals(&self) -> (usize, usize) {
        self.objs.values().fold((0, 0), |(p, q), o| (p + o.pieces.len(), q + o.points))
    }

    pub fn get(&self, n: u32) -> Option<&Obj> {
        self.objs.get(&n)
    }

    fn remove(&mut self, id: u64) {
        if let Some(n) = self.by_id.remove(&id) {
            self.objs.remove(&n);
            self.ver += 1;
            self.gone.push_back((self.ver, n));
            if self.gone.len() > GONE_KEPT {
                if let Some((v, _)) = self.gone.pop_front() {
                    self.gone_dropped = v;
                }
            }
            self.stats.gone += 1;
        }
    }

    /// GMSKY_COL_LOOSE. `col_epoch`: the shapes store's current epoch (older messages are ignored).
    pub fn apply_loose(&mut self, p: &[u8], col_epoch: u32) -> Option<String> {
        let Some(h) = read::<ColLoose>(p, 0) else {
            self.stats.bad += 1;
            return Some("loose: message too short".into());
        };
        if h.col_epoch != col_epoch {
            self.stats.ignored += 1;
            return None;
        }
        let f = |v: f32| v as f64;
        let pos = [f(h.pos[0]), f(h.pos[1]), f(h.pos[2])];
        let rot = h.rot.map(f);
        let vel = [f(h.vel[0]), f(h.vel[1]), f(h.vel[2])];
        if h.flags & LOOSE_SHAPE == 0 {
            let Some(&n) = self.by_id.get(&h.id) else {
                self.stats.unknown += 1;
                return None;
            };
            self.ver += 1;
            let o = self.objs.get_mut(&n)?;
            o.pos = pos;
            o.rot = rot;
            o.vel = vel;
            o.flags = (o.flags & LOOSE_SHAPE) | h.flags;
            o.ver = self.ver;
            self.stats.moved += 1;
            return None;
        }
        let head = std::mem::size_of::<ColLoose>();
        let need = head + h.pieces as usize * 4 + h.points as usize * 12;
        if p.len() < need || h.pieces > 4096 || h.points > 1 << 20 {
            self.stats.bad += 1;
            return Some(format!("loose: body {:#x} message has {} bytes, needs {}", h.id, p.len(), need));
        }
        let mut pieces = Vec::with_capacity(h.pieces as usize);
        let mut at = head + h.pieces as usize * 4;
        let mut total = 0usize;
        for i in 0..h.pieces as usize {
            let count = read::<u32>(p, head + i * 4)? as usize;
            total += count;
            if total > h.points as usize {
                self.stats.bad += 1;
                return Some(format!("loose: body {:#x}: piece counts add up past {} points", h.id, h.points));
            }
            let mut piece = Vec::with_capacity(count);
            for _ in 0..count {
                piece.push([read::<f32>(p, at)?, read::<f32>(p, at + 4)?, read::<f32>(p, at + 8)?]);
                at += 12;
            }
            pieces.push(piece);
        }
        // A body sent with its shape again (Skyrim restarted its list) replaces the old one.
        self.remove(h.id);
        self.next_n += 1;
        self.ver += 1;
        let n = self.next_n;
        self.by_id.insert(h.id, n);
        self.objs.insert(n, Obj { n, form_id: h.form_id, flags: h.flags, layer: h.layer, motion: h.motion, pos, rot, vel,
                                  mass: h.mass as f64, pieces, points: total, ver: self.ver });
        self.stats.added += 1;
        None
    }

    /// GMSKY_COL_LOOSE_GONE.
    pub fn apply_gone(&mut self, p: &[u8], col_epoch: u32) -> Option<String> {
        let Some(g) = read::<ColLooseGone>(p, 0) else {
            self.stats.bad += 1;
            return Some("loose: gone message too short".into());
        };
        if g.col_epoch != col_epoch {
            self.stats.ignored += 1;
            return None;
        }
        self.remove(g.id);
        None
    }

    /// Changes after counter `since` of generation `generation`.
    pub fn changes(&self, generation: u32, since: u64) -> Changes<'_> {
        let resync = generation != self.generation || since < self.gone_dropped;
        let mut changed: Vec<&Obj> = self.objs.values().filter(|o| resync || o.ver > since).collect();
        changed.sort_by_key(|o| o.n);
        let gone = if resync { Vec::new() } else { self.gone.iter().filter(|&&(v, _)| v > since).map(|&(_, n)| n).collect() };
        Changes { generation: self.generation, ver: self.ver, resync, changed, gone }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gmsky_protocol::LOOSE_KEYFRAMED;

    fn msg(id: u64, epoch: u32, pos: [f32; 3], pieces: &[Vec<[f32; 3]>]) -> Vec<u8> {
        let points: usize = pieces.iter().map(|p| p.len()).sum();
        let h = ColLoose { id, col_epoch: epoch, form_id: 0x1234, flags: if pieces.is_empty() { 0 } else { LOOSE_SHAPE },
                           pieces: pieces.len() as u32, points: points as u32, pos, rot: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                           mass: 2.0, ..Default::default() };
        let mut v = unsafe { std::slice::from_raw_parts(&h as *const _ as *const u8, std::mem::size_of::<ColLoose>()) }.to_vec();
        for p in pieces {
            v.extend((p.len() as u32).to_le_bytes());
        }
        for p in pieces {
            for q in p {
                for c in q {
                    v.extend(c.to_le_bytes());
                }
            }
        }
        v
    }

    fn gone(id: u64, epoch: u32) -> Vec<u8> {
        let g = ColLooseGone { id, col_epoch: epoch, reason: 1 };
        unsafe { std::slice::from_raw_parts(&g as *const _ as *const u8, 16) }.to_vec()
    }

    #[test]
    fn add_move_remove() {
        let mut s = Store::default();
        let cube: Vec<[f32; 3]> = (0..8).map(|i| [(i & 1) as f32, ((i >> 1) & 1) as f32, (i >> 2) as f32]).collect();
        assert!(s.apply_loose(&msg(0xABC, 1, [1.0, 2.0, 3.0], &[cube.clone(), cube[..4].to_vec()]), 1).is_none());
        assert_eq!(s.len(), 1);
        let c = s.changes(s.generation, 0);
        assert_eq!(c.changed.len(), 1);
        let o = c.changed[0];
        assert_eq!((o.pieces.len(), o.points, o.pos), (2, 12, [1.0, 2.0, 3.0]));
        let (gen, seen) = (c.generation, c.ver);
        // Nothing new.
        assert!(s.changes(gen, seen).changed.is_empty());
        // A move without the shape keeps the pieces.
        let mut m = msg(0xABC, 1, [5.0, 2.0, 3.0], &[]);
        m[16..20].copy_from_slice(&LOOSE_KEYFRAMED.to_le_bytes());
        s.apply_loose(&m, 1);
        let c = s.changes(gen, seen);
        assert_eq!(c.changed.len(), 1);
        assert_eq!(c.changed[0].pos[0], 5.0);
        assert_eq!(c.changed[0].pieces.len(), 2);
        assert_eq!(c.changed[0].flags, LOOSE_SHAPE | LOOSE_KEYFRAMED);
        let seen = c.ver;
        // A move for a body never seen is counted, not stored.
        s.apply_loose(&msg(0xDEF, 1, [0.0; 3], &[]), 1);
        assert_eq!((s.len(), s.stats.unknown), (1, 1));
        // Gone.
        s.apply_gone(&gone(0xABC, 1), 1);
        let c = s.changes(gen, seen);
        assert_eq!((c.changed.len(), c.gone.clone()), (0, vec![1]));
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn old_epoch_and_clear() {
        let mut s = Store::default();
        let tri = vec![[0.0f32; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        s.apply_loose(&msg(1, 1, [0.0; 3], &[tri.clone()]), 2);
        assert_eq!((s.len(), s.stats.ignored), (0, 1));
        s.apply_loose(&msg(1, 2, [0.0; 3], &[tri.clone()]), 2);
        let gen = s.generation;
        s.clear();
        let c = s.changes(gen, 0);
        assert!(c.resync && c.changed.is_empty());
        // A short message is reported, not a panic.
        assert!(s.apply_loose(&msg(1, 2, [0.0; 3], &[tri])[..120], 2).is_some());
    }
}
