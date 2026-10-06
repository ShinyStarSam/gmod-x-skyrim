//! Skyrim's collision triangles (DESIGN.md re-plan step D) and the swept test against them.
//!
//! Skyrim streams its collision around the player region by region through the collision ring
//! (gmsky_protocol.h; the streaming is adapted from SkyCraft, MIT). This store keeps the regions
//! and answers "where does this moving box first touch them", for `ENT:TestCollision` on both
//! Garry's Mod sides (server and client share this module in one process). Triangles never go
//! into Lua: a building can have thousands.
//!
//! The swept test is gmod/lua/gmsky/field.lua's, ported line by line (same rules: stop 1/32 short
//! of the surface; a box already overlapping a triangle may move out of it or along it, not
//! further in; long moves are tested in 128-unit sections in order). The harness checks both
//! give the same answers.
//!
//! All coordinates are Garry's Mod units relative to the anchor (like the ground grids).

use gmsky_protocol::{ColClear, ColRegion, ColTri, COL_CLEAR, COL_REGION};
use std::collections::HashMap;

const CELL: f64 = 64.0; // lookup cell inside a region (x, y), Garry's Mod units
const EPS: f64 = 0.03125; // Source's DIST_EPSILON: stop this far before the surface
const SECTION: f64 = 128.0; // long moves are tested in sections this long
/// Regions further than this (in regions, along any axis) from the player are dropped. Skyrim
/// forgets having sent a region at 6, so anything it believes we have, we still have.
pub const EVICT_REGIONS: i32 = 8;

#[derive(Clone, Copy)]
struct Tri {
    v: [f64; 9],
    n: [f64; 3],
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Tri {
    fn new(v: [f64; 9]) -> Option<Tri> {
        let e1 = [v[3] - v[0], v[4] - v[1], v[5] - v[2]];
        let e2 = [v[6] - v[0], v[7] - v[1], v[8] - v[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if !(len >= 1e-6) {
            return None; // degenerate (or NaN)
        }
        let mut lo = [0.0; 3];
        let mut hi = [0.0; 3];
        for k in 0..3 {
            lo[k] = v[k].min(v[3 + k]).min(v[6 + k]);
            hi[k] = v[k].max(v[3 + k]).max(v[6 + k]);
        }
        Some(Tri { v, n: [n[0] / len, n[1] / len, n[2] / len], lo, hi })
    }
}

struct Region {
    tris: Vec<Tri>,
    stamp: Vec<u32>,       // per triangle: the last section that tested it (a triangle is in several cells)
    cells: Vec<Vec<u32>>,  // cn x cn lookup cells over the region's x, y (plus one cell of margin each side)
    cn: i64,
    lo: [f64; 3],
    size: f64,
    stair_helpers: u32,
}

impl Region {
    fn cell_index(&self, x: f64, y: f64) -> (i64, i64) {
        // Cell 0 starts one CELL before the region (triangles stick out a little).
        (((x - self.lo[0]) / CELL).floor() as i64 + 1, ((y - self.lo[1]) / CELL).floor() as i64 + 1)
    }
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub messages: u64,
    pub bytes: u64,
    pub regions_in: u64,
    pub tris_in: u64,
    pub ignored: u64, // region messages of another epoch (or before the first "clear")
    pub evicted: u64,
    pub sweeps: u64,
    pub hits: u64,
    pub tested: u64,
    pub us: f64,
    pub worst_us: f64,
}

#[derive(Default)]
pub struct Store {
    pub col_epoch: u32,
    pub origin_epoch: u32, // 0 = no "clear" seen yet: nothing is used
    regions: HashMap<(i32, i32, i32), Region>,
    /// The region lattice: where region (0, 0, 0) starts, and the region size (Garry's Mod units).
    lattice: Option<([f64; 3], f64)>,
    tris: usize,
    stamp: u32,
    pub stats: Stats,
}

fn read<T: Copy>(p: &[u8], off: usize) -> Option<T> {
    if off + std::mem::size_of::<T>() > p.len() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(p.as_ptr().add(off) as *const T) })
}

impl Store {
    pub fn regions(&self) -> usize {
        self.regions.len()
    }

    pub fn tris(&self) -> usize {
        self.tris
    }

    /// One message from the ring. Returns a line to log for notable events (clear, bad message).
    pub fn apply(&mut self, kind: u32, p: &[u8]) -> Option<String> {
        self.stats.messages += 1;
        self.stats.bytes += p.len() as u64 + 8;
        match kind {
            COL_CLEAR => {
                let c: ColClear = read(p, 0)?;
                let dropped = self.regions.len();
                self.regions.clear();
                self.lattice = None;
                self.tris = 0;
                self.col_epoch = c.col_epoch;
                self.origin_epoch = c.origin_epoch;
                Some(format!("shapes: clear (collision epoch {}, origin epoch {}): {} regions dropped",
                             c.col_epoch, c.origin_epoch, dropped))
            }
            COL_REGION => {
                let Some(h) = read::<ColRegion>(p, 0) else { return Some("shapes: region message too short".into()) };
                let need = std::mem::size_of::<ColRegion>() + h.count as usize * std::mem::size_of::<ColTri>();
                if p.len() < need {
                    return Some(format!("shapes: region ({}, {}, {}) message has {} bytes, needs {}", h.rx, h.ry, h.rz, p.len(), need));
                }
                if self.origin_epoch == 0 || h.col_epoch != self.col_epoch {
                    self.stats.ignored += 1;
                    return None;
                }
                let lo = [h.lo[0] as f64, h.lo[1] as f64, h.lo[2] as f64];
                let size = h.size as f64;
                if self.lattice.is_none() && size > 0.0 {
                    self.lattice = Some(([lo[0] - h.rx as f64 * size, lo[1] - h.ry as f64 * size, lo[2] - h.rz as f64 * size], size));
                }
                let cn = (size / CELL).ceil() as i64 + 2;
                let mut r = Region { tris: Vec::with_capacity(h.count as usize), stamp: Vec::new(), cells: vec![Vec::new(); (cn * cn) as usize],
                                     cn, lo, size, stair_helpers: 0 };
                for i in 0..h.count as usize {
                    let t: ColTri = read(p, std::mem::size_of::<ColRegion>() + i * std::mem::size_of::<ColTri>())?;
                    let mut v = [0.0; 9];
                    for k in 0..9 {
                        v[k] = t.v[k] as f64;
                    }
                    if let Some(tri) = Tri::new(v) {
                        let idx = r.tris.len() as u32;
                        let (x0, y0) = r.cell_index(tri.lo[0], tri.lo[1]);
                        let (x1, y1) = r.cell_index(tri.hi[0], tri.hi[1]);
                        for cx in x0.max(0)..=x1.min(cn - 1) {
                            for cy in y0.max(0)..=y1.min(cn - 1) {
                                r.cells[(cx * cn + cy) as usize].push(idx);
                            }
                        }
                        r.stair_helpers += (t.flags & gmsky_protocol::TRI_STAIR_HELPER != 0) as u32;
                        r.tris.push(tri);
                    }
                }
                r.stamp = vec![0; r.tris.len()];
                self.stats.regions_in += 1;
                self.stats.tris_in += r.tris.len() as u64;
                self.tris += r.tris.len();
                if let Some(old) = self.regions.insert((h.rx, h.ry, h.rz), r) {
                    self.tris -= old.tris.len();
                }
                None
            }
            _ => Some(format!("shapes: unknown message type {kind} ({} bytes)", p.len())),
        }
    }

    /// The region containing point p, if the lattice is known.
    fn region_of(&self, p: [f64; 3]) -> Option<(i32, i32, i32)> {
        let (base, size) = self.lattice?;
        let f = |k: usize| ((p[k] - base[k]) / size).floor() as i32;
        Some((f(0), f(1), f(2)))
    }

    /// Drop regions far from the player (p relative to the anchor).
    pub fn evict(&mut self, p: [f64; 3]) {
        let Some((px, py, pz)) = self.region_of(p) else { return };
        let before = self.regions.len();
        let mut removed = 0;
        self.regions.retain(|&(x, y, z), r| {
            let keep = (x - px).abs() <= EVICT_REGIONS && (y - py).abs() <= EVICT_REGIONS && (z - pz).abs() <= EVICT_REGIONS;
            if !keep {
                removed += r.tris.len();
            }
            keep
        });
        self.tris -= removed;
        self.stats.evicted += (before - self.regions.len()) as u64;
    }

    /// Has the region containing p, and the one below it, arrived? (Then the player may move
    /// there: SkyCraft's "hold until Skyrim's collision around them has arrived".)
    pub fn known(&self, p: [f64; 3]) -> bool {
        match self.region_of(p) {
            Some((x, y, z)) => self.regions.contains_key(&(x, y, z)) && self.regions.contains_key(&(x, y, z - 1)),
            None => false,
        }
    }

    /// Box around every stored region, for the entity's collision bounds.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for r in self.regions.values() {
            for k in 0..3 {
                lo[k] = lo[k].min(r.lo[k]);
                hi[k] = hi[k].max(r.lo[k] + r.size);
            }
        }
        (lo[0] <= hi[0]).then_some((lo, hi))
    }

    pub fn stair_helpers(&self) -> u64 {
        self.regions.values().map(|r| r.stair_helpers as u64).sum()
    }

    /// Every stored triangle touching the box lo..hi (relative to the anchor), at most `max`, for
    /// building a Garry's Mod physics mesh (DESIGN.md "One collision system", S1). A shape touching
    /// two regions comes from Skyrim in both; identical triangles (to 1/16 unit, same vertex
    /// order) are returned once. With `clip`, every triangle is cut exactly at the box's faces
    /// (only the part inside is kept), so a mesh never reaches outside its box and boxes laid
    /// edge to edge never share a piece (S1c's chunks); the second value counts the triangles cut.
    pub fn tris_in_box(&self, lo: [f64; 3], hi: [f64; 3], max: usize, clip: bool) -> (Vec<[f64; 9]>, usize) {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut cut = 0;
        for r in self.regions.values() {
            if (0..3).any(|k| r.lo[k] > hi[k] || r.lo[k] + r.size < lo[k]) {
                continue;
            }
            for t in &r.tris {
                if (0..3).any(|k| t.lo[k] > hi[k] || t.hi[k] < lo[k]) {
                    continue;
                }
                let key: [i64; 9] = std::array::from_fn(|i| (t.v[i] * 16.0).round() as i64);
                if !seen.insert(key) {
                    continue;
                }
                if !clip || (0..3).all(|k| t.lo[k] >= lo[k] && t.hi[k] < hi[k]) {
                    if out.len() >= max {
                        return (out, cut);
                    }
                    out.push(t.v);
                    continue;
                }
                let pieces = clip_to_box(&t.v, lo, hi);
                if !(pieces.is_empty() || pieces.len() == 1 && pieces[0] == t.v) {
                    cut += 1;
                }
                for piece in pieces {
                    if out.len() >= max {
                        return (out, cut);
                    }
                    out.push(piece);
                }
            }
        }
        (out, cut)
    }

    /// Where region (0, 0, 0) starts and the region size (Garry's Mod units, relative to the
    /// anchor), once the first region has arrived.
    pub fn lattice(&self) -> Option<([f64; 3], f64)> {
        self.lattice
    }

    /// How many of the regions r0..=r1 (inclusive, Skyrim's region numbers) have arrived, and
    /// how many there are.
    pub fn regions_arrived(&self, r0: [i32; 3], r1: [i32; 3]) -> (usize, usize) {
        let (mut arrived, mut total) = (0, 0);
        for x in r0[0]..=r1[0] {
            for y in r0[1]..=r1[1] {
                for z in r0[2]..=r1[2] {
                    total += 1;
                    arrived += self.regions.contains_key(&(x, y, z)) as usize;
                }
            }
        }
        (arrived, total)
    }

    /// Triangles in the region containing p (for the log), or None if it hasn't arrived.
    pub fn tris_at(&self, p: [f64; 3]) -> Option<usize> {
        self.region_of(p).and_then(|k| self.regions.get(&k)).map(|r| r.tris.len())
    }

    /// Sweeps a box (centre c, half size e) by d. Returns fraction and normal, or None.
    pub fn sweep(&mut self, c: [f64; 3], e: [f64; 3], d: [f64; 3]) -> Option<(f64, [f64; 3])> {
        let t0 = std::time::Instant::now();
        let r = self.sweep_inner(c, e, d);
        let us = t0.elapsed().as_secs_f64() * 1e6;
        self.stats.sweeps += 1;
        self.stats.hits += r.is_some() as u64;
        self.stats.us += us;
        self.stats.worst_us = self.stats.worst_us.max(us);
        r
    }

    fn sweep_inner(&mut self, c: [f64; 3], e: [f64; 3], d: [f64; 3]) -> Option<(f64, [f64; 3])> {
        if self.regions.is_empty() {
            return None;
        }
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let m = (len / SECTION).ceil().max(1.0);
        let s = [d[0] / m, d[1] / m, d[2] / m];
        let mi = m as i64;
        for k in 0..mi {
            let kf = k as f64;
            let cc = [c[0] + d[0] * kf / m, c[1] + d[1] * kf / m, c[2] + d[2] * kf / m];
            if let Some((frac, n)) = self.sweep_section(cc, e, s) {
                let mut best = (kf + frac) / m;
                if len > 0.0 {
                    best = (best - EPS / len).max(0.0);
                }
                return Some((best, n));
            }
        }
        None
    }

    fn sweep_section(&mut self, c: [f64; 3], e: [f64; 3], d: [f64; 3]) -> Option<(f64, [f64; 3])> {
        // The moving box's bounds, 0.01 wider (as field.lua: a trace along an edge also looks at
        // the triangle on the other side of it).
        let mut lo = [0.0; 3];
        let mut hi = [0.0; 3];
        for k in 0..3 {
            lo[k] = c[k].min(c[k] + d[k]) - e[k] - 0.01;
            hi[k] = c[k].max(c[k] + d[k]) + e[k] + 0.01;
        }
        let (base, size) = self.lattice?;
        let ri = |v: f64, k: usize| ((v - base[k]) / size).floor() as i32;
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            for r in self.regions.values_mut() {
                r.stamp.iter_mut().for_each(|s| *s = 0);
            }
            self.stamp = 1;
        }
        let stamp = self.stamp;
        let mut best: Option<(f64, [f64; 3])> = None;
        let mut tested = 0u64;
        for rx in ri(lo[0], 0)..=ri(hi[0], 0) {
            for ry in ri(lo[1], 1)..=ri(hi[1], 1) {
                for rz in ri(lo[2], 2)..=ri(hi[2], 2) {
                    let Some(r) = self.regions.get_mut(&(rx, ry, rz)) else { continue };
                    let (x0, y0) = r.cell_index(lo[0], lo[1]);
                    let (x1, y1) = r.cell_index(hi[0], hi[1]);
                    for cx in x0.max(0)..=x1.min(r.cn - 1) {
                        for cy in y0.max(0)..=y1.min(r.cn - 1) {
                            for &i in &r.cells[(cx * r.cn + cy) as usize] {
                                let i = i as usize;
                                if r.stamp[i] == stamp {
                                    continue;
                                }
                                r.stamp[i] = stamp;
                                let t = &r.tris[i];
                                if t.lo[0] <= hi[0] && t.hi[0] >= lo[0] && t.lo[1] <= hi[1] && t.hi[1] >= lo[1] && t.lo[2] <= hi[2] && t.hi[2] >= lo[2] {
                                    tested += 1;
                                    if let Some((f, n)) = sweep_tri(t, c, e, d) {
                                        if best.map_or(true, |(b, _)| f < b) {
                                            best = Some((f, n));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.stats.tested += tested;
        best
    }
}

/// field.lua AxisTimes: enter/exit times along a unit axis, the move's speed along it; None if
/// the axis separates them for the whole move.
fn axis_times(t: &Tri, c: [f64; 3], e: [f64; 3], d: [f64; 3], a: [f64; 3]) -> Option<(f64, f64, f64)> {
    let p0 = t.v[0] * a[0] + t.v[1] * a[1] + t.v[2] * a[2];
    let p1 = t.v[3] * a[0] + t.v[4] * a[1] + t.v[5] * a[2];
    let p2 = t.v[6] * a[0] + t.v[7] * a[1] + t.v[8] * a[2];
    let lo = p0.min(p1).min(p2);
    let hi = p0.max(p1).max(p2);
    let r = e[0] * a[0].abs() + e[1] * a[1].abs() + e[2] * a[2].abs();
    let p = c[0] * a[0] + c[1] * a[1] + c[2] * a[2];
    let s = d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
    if s == 0.0 {
        if p + r < lo - 1e-4 || p - r > hi + 1e-4 {
            return None;
        }
        return Some((f64::NEG_INFINITY, f64::INFINITY, s));
    }
    let mut t1 = (lo - (p + r)) / s;
    let mut t2 = (hi - (p - r)) / s;
    if t1 > t2 {
        std::mem::swap(&mut t1, &mut t2);
    }
    Some((t1, t2, s))
}

/// field.lua SweepTri: earliest touch of the moving box with triangle t.
fn sweep_tri(t: &Tri, c: [f64; 3], e: [f64; 3], d: [f64; 3]) -> Option<(f64, [f64; 3])> {
    let mut enter = f64::NEG_INFINITY;
    let mut exit = f64::INFINITY;
    let mut n = [0.0; 3];
    let mut axis = |a: [f64; 3]| -> bool {
        let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        if len < 1e-6 {
            return true;
        }
        let a = [a[0] / len, a[1] / len, a[2] / len];
        let Some((t1, t2, s)) = axis_times(t, c, e, d, a) else { return false };
        if t1 > enter {
            enter = t1;
            n = if s > 0.0 { [-a[0], -a[1], -a[2]] } else { a };
        }
        if t2 < exit {
            exit = t2;
        }
        enter <= exit
    };
    if !axis([1.0, 0.0, 0.0]) || !axis([0.0, 1.0, 0.0]) || !axis([0.0, 0.0, 1.0]) || !axis(t.n) {
        return None;
    }
    let edges = [
        [t.v[3] - t.v[0], t.v[4] - t.v[1], t.v[5] - t.v[2]],
        [t.v[6] - t.v[3], t.v[7] - t.v[4], t.v[8] - t.v[5]],
        [t.v[0] - t.v[6], t.v[1] - t.v[7], t.v[2] - t.v[8]],
    ];
    for u in edges {
        if !axis([0.0, u[2], -u[1]]) || !axis([-u[2], 0.0, u[0]]) || !axis([u[1], -u[0], 0.0]) {
            return None;
        }
    }
    if enter > 1.0 || exit < 0.0 || enter == f64::NEG_INFINITY {
        return None;
    }
    if enter < 0.0 {
        // Already overlapping: stop only if moving further in; out or along is free.
        let mut f = t.n;
        if (c[0] - t.v[0]) * f[0] + (c[1] - t.v[1]) * f[1] + (c[2] - t.v[2]) * f[2] < 0.0 {
            f = [-f[0], -f[1], -f[2]];
        }
        if d[0] * f[0] + d[1] * f[1] + d[2] * f[2] >= 0.0 {
            return None;
        }
        return Some((0.0, f));
    }
    Some((enter, n))
}

/// The part of triangle v inside the box lo..hi, as triangles (Sutherland-Hodgman against the
/// six faces, then a fan). A piece lying entirely on a high face (x = hi.x, ...) is dropped: it
/// belongs to the neighbouring box, whose low face it is on. Pieces with no area are dropped.
pub fn clip_to_box(v: &[f64; 9], lo: [f64; 3], hi: [f64; 3]) -> Vec<[f64; 9]> {
    let mut poly: Vec<[f64; 3]> = (0..3).map(|i| [v[3 * i], v[3 * i + 1], v[3 * i + 2]]).collect();
    for k in 0..3 {
        for (bound, keep_above) in [(lo[k], true), (hi[k], false)] {
            let inside = |p: &[f64; 3]| if keep_above { p[k] >= bound } else { p[k] <= bound };
            let mut next = Vec::with_capacity(poly.len() + 2);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let (ia, ib) = (inside(&a), inside(&b));
                if ia {
                    next.push(a);
                }
                if ia != ib {
                    let t = (bound - a[k]) / (b[k] - a[k]);
                    let mut p: [f64; 3] = std::array::from_fn(|j| a[j] + (b[j] - a[j]) * t);
                    p[k] = bound; // exactly on the face, so neighbours meet without a gap
                    next.push(p);
                }
            }
            poly = next;
            if poly.len() < 3 {
                return Vec::new();
            }
        }
    }
    if (0..3).any(|k| poly.iter().all(|p| p[k] >= hi[k])) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 1..poly.len() - 1 {
        let (a, b, c) = (poly[0], poly[i], poly[i + 1]);
        let t = [a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]];
        if Tri::new(t).is_some() {
            out.push(t);
        }
    }
    out
}

/// Shape of a mesh, for the log: (thin triangles, shortest edge, smallest corner angle in
/// degrees). Thin = a corner under 1 degree or an edge under 0.5 units.
pub fn mesh_quality(tris: &[[f64; 9]]) -> (usize, f64, f64) {
    let (mut thin, mut shortest, mut smallest) = (0, f64::MAX, 180.0f64);
    for v in tris {
        let p: [[f64; 3]; 3] = std::array::from_fn(|i| [v[3 * i], v[3 * i + 1], v[3 * i + 2]]);
        let mut tri_short = f64::MAX;
        let mut tri_angle = 180.0f64;
        for i in 0..3 {
            let (a, b, c) = (p[i], p[(i + 1) % 3], p[(i + 2) % 3]);
            let e1: [f64; 3] = std::array::from_fn(|j| b[j] - a[j]);
            let e2: [f64; 3] = std::array::from_fn(|j| c[j] - a[j]);
            let l1 = (e1[0] * e1[0] + e1[1] * e1[1] + e1[2] * e1[2]).sqrt();
            let l2 = (e2[0] * e2[0] + e2[1] * e2[1] + e2[2] * e2[2]).sqrt();
            tri_short = tri_short.min(l1);
            let cos = if l1 > 0.0 && l2 > 0.0 { (e1[0] * e2[0] + e1[1] * e2[1] + e1[2] * e2[2]) / (l1 * l2) } else { 1.0 };
            tri_angle = tri_angle.min(cos.clamp(-1.0, 1.0).acos().to_degrees());
        }
        if tri_angle < 1.0 || tri_short < 0.5 {
            thin += 1;
        }
        shortest = shortest.min(tri_short);
        smallest = smallest.min(tri_angle);
    }
    (thin, shortest, smallest)
}

/// A region message payload (for tests and the harness's fake Skyrim).
#[cfg(test)]
pub fn region_payload(rx: i32, ry: i32, rz: i32, col_epoch: u32, lo: [f32; 3], size: f32, tris: &[[f32; 9]]) -> Vec<u8> {
    let h = ColRegion { rx, ry, rz, col_epoch, count: tris.len() as u32, flags: 0, lo, size };
    let mut out = Vec::with_capacity(40 + tris.len() * 40);
    out.extend_from_slice(unsafe { std::slice::from_raw_parts(&h as *const _ as *const u8, 40) });
    for v in tris {
        let t = ColTri { v: *v, flags: 0 };
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(&t as *const _ as *const u8, 40) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear(s: &mut Store, col: u32, origin: u32) {
        let c = ColClear { col_epoch: col, origin_epoch: origin };
        s.apply(COL_CLEAR, unsafe { std::slice::from_raw_parts(&c as *const _ as *const u8, 8) });
    }

    /// A 200 x 200 floor at z 0 as two triangles, in region (0, 0, 0) of size 272 starting at -136.
    fn floor_store() -> Store {
        let mut s = Store::default();
        clear(&mut s, 1, 1);
        let tris = [[-100.0, -100.0, 0.0, 100.0, -100.0, 0.0, 100.0, 100.0, 0.0], [-100.0, -100.0, 0.0, 100.0, 100.0, 0.0, -100.0, 100.0, 0.0]];
        s.apply(COL_REGION, &region_payload(0, 0, 0, 1, [-136.0; 3], 272.0, &tris));
        s.apply(COL_REGION, &region_payload(0, 0, -1, 1, [-136.0, -136.0, -408.0], 272.0, &[]));
        s
    }

    // Same cases and numbers as the harness's field.lua checks.
    #[test]
    fn same_answers_as_field_lua() {
        let mut s = floor_store();
        let f = |r: Option<(f64, [f64; 3])>| r.map(|(f, n)| format!("{:.4} {:.2} {:.2} {:.2}", f, n[0] + 0.0, n[1] + 0.0, n[2] + 0.0)).unwrap_or("nil".into());
        assert_eq!(f(s.sweep([0.0, 0.0, 46.0], [16.0, 16.0, 36.0], [0.0, 0.0, -20.0])), "0.4984 0.00 0.00 1.00");
        assert_eq!(f(s.sweep([0.0, 0.0, 50.0], [0.0; 3], [0.0, 0.0, -100.0])), "0.4997 0.00 0.00 1.00");
        assert_eq!(f(s.sweep([0.0, 0.0, 37.0], [16.0, 16.0, 36.0], [100.0, 0.0, 0.0])), "nil");
        assert_eq!(f(s.sweep([0.0, 0.0, 30.0], [16.0, 16.0, 36.0], [0.0, 0.0, 10.0])), "nil");
        assert_eq!(f(s.sweep([0.0, 0.0, 30.0], [16.0, 16.0, 36.0], [0.0, 0.0, -10.0])), "0.0000 0.00 0.00 1.00");
    }

    #[test]
    fn tris_in_box_dedupes_and_limits() {
        let mut s = floor_store();
        // The same floor sent again in the neighbouring region: still two triangles.
        let tris = [[-100.0, -100.0, 0.0, 100.0, -100.0, 0.0, 100.0, 100.0, 0.0], [-100.0, -100.0, 0.0, 100.0, 100.0, 0.0, -100.0, 100.0, 0.0]];
        s.apply(COL_REGION, &region_payload(1, 0, 0, 1, [136.0, -136.0, -136.0], 272.0, &tris));
        assert_eq!(s.tris_in_box([-200.0; 3], [200.0; 3], 100, false).0.len(), 2);
        assert_eq!(s.tris_in_box([-200.0; 3], [200.0; 3], 1, false).0.len(), 1);
        // A box above the floor touches nothing.
        assert_eq!(s.tris_in_box([-50.0, -50.0, 10.0], [50.0, 50.0, 20.0], 100, false).0.len(), 0);
    }

    fn area(tris: &[[f64; 9]]) -> f64 {
        tris.iter()
            .map(|v| {
                let e1 = [v[3] - v[0], v[4] - v[1], v[5] - v[2]];
                let e2 = [v[6] - v[0], v[7] - v[1], v[8] - v[2]];
                let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
            })
            .sum()
    }

    #[test]
    fn tris_in_box_clip_stays_inside_and_never_shares() {
        let s = floor_store();
        // The 200 x 200 floor cut at x = 0 into two boxes: both triangles are cut, the halves
        // stay inside their box and add up to the whole floor.
        let (left, cut_l) = s.tris_in_box([-200.0, -200.0, -10.0], [0.0, 200.0, 10.0], 100, true);
        let (right, cut_r) = s.tris_in_box([0.0, -200.0, -10.0], [200.0, 200.0, 10.0], 100, true);
        assert_eq!((cut_l, cut_r), (2, 2));
        assert!(left.iter().all(|v| v[0] <= 0.0 && v[3] <= 0.0 && v[6] <= 0.0));
        assert!(right.iter().all(|v| v[0] >= 0.0 && v[3] >= 0.0 && v[6] >= 0.0));
        assert!((area(&left) - 20000.0).abs() < 1e-6 && (area(&right) - 20000.0).abs() < 1e-6);
        // Without clipping both boxes get both whole triangles.
        assert_eq!(s.tris_in_box([-200.0, -200.0, -10.0], [0.0, 200.0, 10.0], 100, false).0.len(), 2);
    }

    #[test]
    fn clip_drops_pieces_on_the_high_face() {
        // A wall exactly on x = 0 belongs to the box whose low face is x = 0, not to the one
        // whose high face it is.
        let wall = [0.0, -50.0, 0.0, 0.0, 50.0, 0.0, 0.0, 0.0, 50.0];
        assert_eq!(clip_to_box(&wall, [-100.0; 3], [0.0, 100.0, 100.0]).len(), 0);
        assert_eq!(clip_to_box(&wall, [0.0, -100.0, -100.0], [100.0; 3]).len(), 1);
    }

    #[test]
    fn mesh_quality_finds_thin_triangles() {
        let good = [0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0];
        let sliver = [0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 50.0, 0.1, 0.0];
        let (thin, shortest, angle) = mesh_quality(&[good, sliver]);
        assert_eq!(thin, 1);
        assert!((shortest - 10.0).abs() < 1e-9);
        assert!(angle < 1.0);
    }

    #[test]
    fn lattice_and_regions_arrived() {
        let s = floor_store();
        let (base, size) = s.lattice().unwrap();
        assert_eq!(size, 272.0);
        assert_eq!(base, [-136.0, -136.0, -136.0]);
        assert_eq!(s.regions_arrived([0, 0, -1], [0, 0, 0]), (2, 2));
        assert_eq!(s.regions_arrived([0, 0, -1], [1, 0, 0]), (2, 4));
    }

    #[test]
    fn known_evict_and_epochs() {
        let mut s = floor_store();
        assert!(s.known([0.0, 0.0, 10.0]));
        assert!(!s.known([300.0, 0.0, 10.0]));
        assert_eq!(s.tris(), 2);
        // Another epoch's region is ignored.
        s.apply(COL_REGION, &region_payload(1, 0, 0, 7, [136.0, -136.0, -136.0], 272.0, &[]));
        assert_eq!(s.regions(), 2);
        // Far away player: everything goes.
        s.evict([272.0 * 20.0, 0.0, 0.0]);
        assert_eq!((s.regions(), s.tris()), (0, 0));
        assert!(s.sweep([0.0, 0.0, 10.0], [0.0; 3], [0.0, 0.0, -20.0]).is_none());
    }
}
