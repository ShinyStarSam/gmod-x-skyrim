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
    /// GMSKY_TRI_* as Skyrim sent them: one of its invisible stair ramps (collision layer
    /// kStairHelper), a stair material (S2i-1).
    flags: u32,
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
        Some(Tri { v, n: [n[0] / len, n[1] / len, n[2] / len], lo, hi, flags: 0 })
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
    /// Triangles with a stair material (S2i-1).
    stairs: u32,
    /// Fingerprint of the region's content as Skyrim sent it (FNV-1a of the triangle bytes): a
    /// re-sent but unchanged region keeps it, so its physics chunk isn't rebuilt (S2).
    hash: u64,
}

// ---- S2i-2: ramps over Skyrim's stairs (DESIGN.md S2i) ----
const RAMP_MARGIN: f64 = 64.0; // stair triangles this far around a chunk are looked at
const RISER_MIN: f64 = 1.0; // the height between two noses is 1-20 units
const RISER_MAX: f64 = 20.0;
const NOSE_MIN_LEN: f64 = 2.0;
const RUN_MIN: f64 = 3.0; // two noses are 3-48 units apart across them
const RUN_MAX: f64 = 48.0;
const WALKABLE: f64 = 1.0; // strips rise / run at most this (45 degrees): Source walks up to normal
                           // z 0.7 (45.6 degrees); a steeper step gets a strip this steep and a lip
                           // at its top (test 133: a strip-less steep step left walls beside it)
const LIP_MAX: f64 = 3.0; // ... up to this lip; steeper still: no partner (test 133: a fan-shaped
                          // step 60 degrees steep made a twisted strip with a 5-unit lip)
const FLUSH: f64 = 1.0; // a ramp end within this of the surface it meets counts as flush
const TREAD_NZ: f64 = 0.9; // a tread: a stair triangle this flat (normal z, either side)
const LOOK_OUT: f64 = 1.5; // the surface just beyond an edge is looked at this far out
const SAME: f64 = 0.75; // ... a surface within this of the edge's height continues the tread
const INSIDE: f64 = 0.5; // ... and this far inside, nothing may lie within step height above
const TILT: f64 = 0.2; // a nose may rise this much per unit of its length (test 132: the stone
                       // stairs' steps rise 2-3 units along their 30-unit width), plus a unit

/// What `ramps_in_box` made, and the numbers for the log.
#[derive(Default)]
pub struct Ramps {
    pub tris: Vec<[f64; 9]>,
    /// Stair noses owned by the box (their middle is inside it).
    pub noses: usize,
    /// Nose-to-nose strips; bottom strips (lowest nose down to the floor).
    pub strips: usize,
    pub bottoms: usize,
    /// Bottom strip ends that found no floor within RUN_MAX (they end in the air).
    pub bottoms_floating: usize,
    /// Top noses (a ramp ends there), and of them the ones whose landing just beyond is not
    /// within FLUSH of the nose's height.
    pub tops: usize,
    pub tops_off: usize,
    /// The steepest step a strip was laid over (rise over run), in degrees; over 45 the strips
    /// there are 45 degrees with a lip at the top.
    pub steepest: f64,
    /// Fingerprint of `tris` (to 1/16 unit).
    pub hash: u64,
}

/// Why stair triangles' edges did or didn't become noses (the STUCK lines' ramp check).
#[derive(Default)]
struct NoseStats {
    tris: usize,
    treads: usize,      // flat enough to be a tread
    edges: usize,       // tread edges looked at
    not_level: usize,   // its two ends more than a unit apart in height
    short: usize,       // under NOSE_MIN_LEN
    continues: usize,   // the surface just beyond is at the same height (inside a tread)
    covered: usize,     // something within step height above just beyond (the back of a tread)
    under: usize,       // something within step height above just inside (a box's bottom face)
    noses: usize,
}

/// A step's nose: the front edge of a tread, where the stairs drop away. It may be tilted along
/// its length (heights za at a, zb at b); z is its height in the middle.
struct Nose {
    a: [f64; 2],
    b: [f64; 2],
    za: f64,
    zb: f64,
    z: f64,
    mid: [f64; 2],
    len: f64,
    dir: [f64; 2], // unit, a -> b
    p: [f64; 2],   // unit, across the nose (sign arbitrary)
}

impl Nose {
    /// Its height `t` units along it from a (beyond its ends: carried on in a straight line).
    fn z_at(&self, t: f64) -> f64 {
        self.za + (self.zb - self.za) * t / self.len
    }
    /// How far along it the point p lies, from a.
    fn along(&self, p: [f64; 2]) -> f64 {
        (p[0] - self.a[0]) * self.dir[0] + (p[1] - self.a[1]) * self.dir[1]
    }
}

fn tri9(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 9] {
    [a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]]
}

/// Surface heights straight down through a set of triangles, with a lookup grid (x, y).
struct Heights<'a> {
    tris: Vec<&'a Tri>,
    grid: HashMap<(i64, i64), Vec<u32>>,
}

impl<'a> Heights<'a> {
    const CELL: f64 = 16.0;

    fn new(tris: Vec<&'a Tri>) -> Heights<'a> {
        let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            if t.n[2].abs() < 0.05 {
                continue; // a vertical wall has no height to give
            }
            let c = |v: f64| (v / Self::CELL).floor() as i64;
            for x in c(t.lo[0])..=c(t.hi[0]) {
                for y in c(t.lo[1])..=c(t.hi[1]) {
                    grid.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        Heights { tris, grid }
    }

    /// The highest surface straight below / above (x, y) between `zmin` and `zmax`.
    fn highest(&self, x: f64, y: f64, zmin: f64, zmax: f64) -> Option<f64> {
        let key = ((x / Self::CELL).floor() as i64, (y / Self::CELL).floor() as i64);
        let mut best: Option<f64> = None;
        for &i in self.grid.get(&key).map(|v| v.as_slice()).unwrap_or(&[]) {
            let t = self.tris[i as usize];
            if x < t.lo[0] || x > t.hi[0] || y < t.lo[1] || y > t.hi[1] || t.lo[2] > zmax || t.hi[2] < zmin {
                continue;
            }
            let v = &t.v;
            // Inside the triangle seen from above (either winding).
            let side = |i: usize, j: usize| (v[j * 3] - v[i * 3]) * (y - v[i * 3 + 1]) - (v[j * 3 + 1] - v[i * 3 + 1]) * (x - v[i * 3]);
            let (s0, s1, s2) = (side(0, 1), side(1, 2), side(2, 0));
            if !((s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)) {
                continue;
            }
            let z = v[2] - (t.n[0] * (x - v[0]) + t.n[1] * (y - v[1])) / t.n[2];
            if z <= zmax && z >= zmin && best.map_or(true, |b| z > b) {
                best = Some(z);
            }
        }
        best
    }
}

/// The noses among the stair triangles `stairs`, judged against every surface `all` (test 130:
/// the big stone stairs' risers are irregular triangles without a level top edge, so the risers
/// can't be relied on; the treads can). A nose is an edge of a tread (a stair triangle with
/// normal z 0.9 or more, either side) that is nearly level (its ends within a unit plus TILT per
/// unit of length in height: test 132's stone steps are tilted), at least 2 long, and where,
/// just beyond it (LOOK_OUT out, at a quarter, half and three quarters along),
/// the surface does not continue at the same height and nothing lies within step height above,
/// at two of the three points at least, and nothing lies within step height above just inside
/// it either (test 133: steps built from stacked boxes; a box's bottom face has an edge right
/// under its top face's edge). The same edge from two triangles counts once.
fn find_noses(stairs: &[&Tri], all: &Heights, st: &mut NoseStats) -> Vec<Nose> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in stairs {
        st.tris += 1;
        if t.n[2].abs() < TREAD_NZ {
            continue;
        }
        st.treads += 1;
        for (i, j, k) in [(0, 1, 2), (1, 2, 0), (2, 0, 1)] {
            st.edges += 1;
            let (za, zb) = (t.v[i * 3 + 2], t.v[j * 3 + 2]);
            let a = [t.v[i * 3], t.v[i * 3 + 1]];
            let b = [t.v[j * 3], t.v[j * 3 + 1]];
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if len < NOSE_MIN_LEN {
                st.short += 1;
                continue;
            }
            if (za - zb).abs() > 1.0 + TILT * len {
                st.not_level += 1;
                continue;
            }
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            // Outward: across the edge, away from the triangle's third corner.
            let mut o = [-dir[1], dir[0]];
            if (t.v[k * 3] - a[0]) * o[0] + (t.v[k * 3 + 1] - a[1]) * o[1] > 0.0 {
                o = [-o[0], -o[1]];
            }
            let (mut cont, mut cover) = (0, 0);
            for f in [0.25, 0.5, 0.75] {
                let z = za + (zb - za) * f;
                let (x, y) = (a[0] + (b[0] - a[0]) * f + o[0] * LOOK_OUT, a[1] + (b[1] - a[1]) * f + o[1] * LOOK_OUT);
                if all.highest(x, y, z - SAME, z + SAME).is_some() {
                    cont += 1;
                } else if all.highest(x, y, z + SAME, z + RISER_MAX).is_some() {
                    cover += 1;
                }
            }
            if cont >= 2 {
                st.continues += 1;
                continue;
            }
            if cont + cover >= 2 {
                st.covered += 1;
                continue;
            }
            let under = [0.25, 0.5, 0.75].iter().filter(|&&f| {
                let z = za + (zb - za) * f;
                let (x, y) = (a[0] + (b[0] - a[0]) * f - o[0] * INSIDE, a[1] + (b[1] - a[1]) * f - o[1] * INSIDE);
                all.highest(x, y, z + SAME, z + RISER_MAX).is_some()
            });
            if under.count() >= 2 {
                st.under += 1;
                continue;
            }
            let q = |p: [f64; 2], z: f64| ((p[0] * 4.0).round() as i64, (p[1] * 4.0).round() as i64, (z * 4.0).round() as i64);
            let (qa, qb) = (q(a, za), q(b, zb));
            if !seen.insert((qa.min(qb), qa.max(qb))) {
                continue;
            }
            out.push(Nose { a, b, za, zb, z: (za + zb) * 0.5, mid: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5], len, dir, p: [-dir[1], dir[0]] });
        }
    }
    st.noses = out.len();
    out
}

/// Why a nose has no nose above it: how far its best candidate got through the checks.
const UNPAIRED: [&str; 5] = ["no nose 1-20 units higher within reach", "higher noses not parallel", "parallel noses not 3-48 units across",
    "noses across don't overlap sideways", "the only partners are too steep to walk"];

/// For each nose: the next nose up (index, run across = signed distance along its `p`), or the
/// stage its best candidate failed at (index into UNPAIRED): 1-20 units higher, parallel within
/// ~25 degrees, 3-48 units across, overlapping it sideways by more than a unit; of those the
/// nearest across (a further nose would make the strip cut through the step in between), if
/// a strip at WALKABLE leaves a lip of at most LIP_MAX.
fn pair_noses(noses: &[Nose]) -> Vec<Result<(usize, f64), usize>> {
    let cell = |p: [f64; 2]| ((p[0] / RUN_MAX).floor() as i64, (p[1] / RUN_MAX).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, n) in noses.iter().enumerate() {
        grid.entry(cell(n.mid)).or_default().push(i);
    }
    noses
        .iter()
        .map(|s| {
            let c = cell(s.mid);
            let mut best: Option<(usize, f64, f64)> = None;
            let mut stage = 0;
            // A nose up to RUN_MAX + half of both lengths away can still overlap; look two cells out.
            let reach = 2 + ((s.len / RUN_MAX).ceil() as i64);
            for gx in c.0 - reach..=c.0 + reach {
                for gy in c.1 - reach..=c.1 + reach {
                    for &j in grid.get(&(gx, gy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let u = &noses[j];
                        // Heights compared across from the middle of s (noses may be tilted).
                        let dz = u.z_at(u.along(s.mid)) - s.z;
                        if !(RISER_MIN..=RISER_MAX).contains(&dz) {
                            continue;
                        }
                        stage = stage.max(1);
                        if (s.dir[0] * u.dir[0] + s.dir[1] * u.dir[1]).abs() < 0.9 {
                            continue;
                        }
                        stage = stage.max(2);
                        let run = (u.mid[0] - s.mid[0]) * s.p[0] + (u.mid[1] - s.mid[1]) * s.p[1];
                        if !(RUN_MIN..=RUN_MAX).contains(&run.abs()) {
                            continue;
                        }
                        stage = stage.max(3);
                        let along = |p: [f64; 2]| (p[0] - s.a[0]) * s.dir[0] + (p[1] - s.a[1]) * s.dir[1];
                        let (ta, tb) = (along(u.a), along(u.b));
                        if ta.max(tb).min(s.len) - ta.min(tb).max(0.0) <= 1.0 {
                            continue;
                        }
                        stage = stage.max(4);
                        if best.map_or(true, |b| run.abs() < b.2.abs()) {
                            best = Some((j, dz, run));
                        }
                    }
                }
            }
            match best {
                Some((j, dz, run)) if dz <= run.abs() * WALKABLE + LIP_MAX => Ok((j, run)),
                _ => Err(stage),
            }
        })
        .collect()
}

/// FNV-1a, 64-bit, continuing from `h`.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}
const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

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
                let body = &p[std::mem::size_of::<ColRegion>()..need];
                let hash = fnv(fnv(FNV_START, &h.count.to_le_bytes()), body);
                let mut r = Region { tris: Vec::with_capacity(h.count as usize), stamp: Vec::new(), cells: vec![Vec::new(); (cn * cn) as usize],
                                     cn, lo, size, stair_helpers: 0, stairs: 0, hash };
                for i in 0..h.count as usize {
                    let t: ColTri = read(p, std::mem::size_of::<ColRegion>() + i * std::mem::size_of::<ColTri>())?;
                    let mut v = [0.0; 9];
                    for k in 0..9 {
                        v[k] = t.v[k] as f64;
                    }
                    if let Some(mut tri) = Tri::new(v) {
                        tri.flags = t.flags;
                        let idx = r.tris.len() as u32;
                        let (x0, y0) = r.cell_index(tri.lo[0], tri.lo[1]);
                        let (x1, y1) = r.cell_index(tri.hi[0], tri.hi[1]);
                        for cx in x0.max(0)..=x1.min(cn - 1) {
                            for cy in y0.max(0)..=y1.min(cn - 1) {
                                r.cells[(cx * cn + cy) as usize].push(idx);
                            }
                        }
                        r.stair_helpers += (tri.flags & gmsky_protocol::TRI_STAIR_HELPER != 0) as u32;
                        r.stairs += (tri.flags & gmsky_protocol::TRI_STAIRS != 0) as u32;
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

    /// Stored triangles with a stair material (S2i-1).
    pub fn stairs(&self) -> u64 {
        self.regions.values().map(|r| r.stairs as u64).sum()
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

    /// Skyrim's triangles touching the box (each once), at most `max`: (lowest z, highest z,
    /// unit normal, GMSKY_TRI_* flags). For the stuck detector (S2e).
    pub fn tris_info(&self, lo: [f64; 3], hi: [f64; 3], max: usize) -> Vec<(f64, f64, [f64; 3], u32)> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
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
                if out.len() >= max {
                    return out;
                }
                out.push((t.lo[2], t.hi[2], t.n, t.flags));
            }
        }
        out
    }

    /// The triangles touching lo..hi grown by `margin`, each once: (stair material, others).
    fn gather(&self, lo: [f64; 3], hi: [f64; 3], margin: f64) -> (Vec<&Tri>, Vec<&Tri>) {
        let (glo, ghi) = (lo.map(|v| v - margin), hi.map(|v| v + margin));
        let mut seen = std::collections::HashSet::new();
        let (mut stairs, mut others) = (Vec::new(), Vec::new());
        for r in self.regions.values() {
            if (0..3).any(|k| r.lo[k] > ghi[k] || r.lo[k] + r.size < glo[k]) {
                continue;
            }
            for t in &r.tris {
                if (0..3).any(|k| t.lo[k] > ghi[k] || t.hi[k] < glo[k]) {
                    continue;
                }
                let key: [i64; 9] = std::array::from_fn(|i| (t.v[i] * 16.0).round() as i64);
                if seen.insert(key) {
                    if t.flags & gmsky_protocol::TRI_STAIRS != 0 { stairs.push(t) } else { others.push(t) }
                }
            }
        }
        (stairs, others)
    }

    /// S2i-2: ramps over Skyrim's stairs for the box lo..hi (relative to the anchor), from the
    /// stair-material triangles in it and RAMP_MARGIN around it (DESIGN.md S2i). Each step's
    /// nose is joined to the next nose up by a flat strip, like a board laid on the stairs; the
    /// bottom strip runs down until it meets the real floor (flush), and the landing beyond the
    /// top nose is checked.
    pub fn ramps_in_box(&self, lo: [f64; 3], hi: [f64; 3]) -> Ramps {
        let (stairs, others) = self.gather(lo, hi, RAMP_MARGIN);
        let all = Heights::new(stairs.iter().chain(others.iter()).copied().collect());
        let owned = |n: &Nose| (0..2).all(|k| n.mid[k] >= lo[k] && n.mid[k] < hi[k]) && n.z >= lo[2] && n.z < hi[2];
        let mut out = Ramps::default();
        let noses = find_noses(&stairs, &all, &mut NoseStats::default());
        let up = pair_noses(&noses);
        // Test 133 (curved flights): a step's nose comes in several straight pieces meeting at
        // bends. Pieces that share a corner: `ends` finds them; at a shared corner both strips
        // cross the step in the same direction (the average of both pieces' uphill), and climb
        // to the step above's nose pieces themselves, bends included, so neighbouring strips
        // meet exactly instead of leaving a slot at every bend.
        let key = |p: [f64; 2], z: f64| ((p[0] * 16.0).round() as i64, (p[1] * 16.0).round() as i64, (z * 16.0).round() as i64);
        let mut ends: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        for (i, n) in noses.iter().enumerate() {
            ends.entry(key(n.a, n.za)).or_default().push(i);
            ends.entry(key(n.b, n.zb)).or_default().push(i);
        }
        // A step's whole nose: nose j and the pieces joined to it end to end (at most 8).
        let chain = |j: usize| {
            let mut got = vec![j];
            let mut k = 0;
            while k < got.len() && got.len() < 8 {
                let n = &noses[got[k]];
                for (p, z) in [(n.a, n.za), (n.b, n.zb)] {
                    for &m in ends.get(&key(p, z)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        if !got.contains(&m) && (noses[m].dir[0] * n.dir[0] + noses[m].dir[1] * n.dir[1]).abs() > 0.7 {
                            got.push(m);
                        }
                    }
                }
                k += 1;
            }
            got
        };
        // Uphill direction of each nose with a step below it: a lower nose that pairs with any
        // piece of its step's nose and lies below this piece, side by side with it (test 133: a
        // middle piece nobody paired with was taken for the bottom step and got a strip down
        // through the steps below).
        let mut uphill: Vec<Option<[f64; 2]>> = vec![None; noses.len()];
        for (i, u) in up.iter().enumerate() {
            if let Ok((j, run)) = *u {
                let d = [noses[i].p[0] * run.signum(), noses[i].p[1] * run.signum()];
                for m in chain(j) {
                    let (n, l) = (&noses[m], &noses[i]);
                    let (ta, tb) = (n.along(l.a), n.along(l.b));
                    if ta.max(tb).min(n.len) - ta.min(tb).max(0.0) <= 1.0 {
                        continue;
                    }
                    let p = n.p;
                    let sign = if p[0] * d[0] + p[1] * d[1] < 0.0 { -1.0 } else { 1.0 };
                    uphill[m].get_or_insert([p[0] * sign, p[1] * sign]);
                }
            }
        }
        let up_of = |i: usize| match up[i] {
            Ok((_, run)) => Some([noses[i].p[0] * run.signum(), noses[i].p[1] * run.signum()]),
            Err(_) => uphill[i],
        };
        // Which way to cross from nose i's end p (height z): uphill, shared with the pieces
        // meeting it there.
        let cross = |i: usize, p: [f64; 2], z: f64| -> Option<[f64; 2]> {
            let mut d = up_of(i)?;
            let own = d;
            for &k in ends.get(&key(p, z)).map(|v| v.as_slice()).unwrap_or(&[]) {
                if let Some(u) = up_of(k).filter(|u| k != i && u[0] * own[0] + u[1] * own[1] > 0.5) {
                    d = [d[0] + u[0], d[1] + u[1]];
                }
            }
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
            Some([d[0] / l, d[1] / l])
        };
        // Where the line from e going d first meets the chain's pieces (None: it passes its ends).
        let hit = |e: [f64; 2], d: [f64; 2], pieces: &[usize]| {
            let mut best: Option<(f64, [f64; 3])> = None;
            for &m in pieces {
                let n = &noses[m];
                let den = d[0] * n.dir[1] - d[1] * n.dir[0];
                if den.abs() < 1e-6 {
                    continue;
                }
                let (wx, wy) = (n.a[0] - e[0], n.a[1] - e[1]);
                let t = (wx * n.dir[1] - wy * n.dir[0]) / den; // along d
                let w = (wx * d[1] - wy * d[0]) / den; // along the piece, from a
                if t > 0.0 && t < 2.0 * RUN_MAX && w >= -0.01 && w <= n.len + 0.01 && best.map_or(true, |b| t < b.0) {
                    best = Some((t, [e[0] + d[0] * t, e[1] + d[1] * t, n.z_at(w)]));
                }
            }
            best.map(|b| b.1)
        };
        // The slope a bottom nose's strip climbs at (rise over run, at most WALKABLE).
        let slope = |m: usize| {
            let (j, run) = up[m].ok()?;
            let dz = noses[j].z_at(noses[j].along(noses[m].mid)) - noses[m].z;
            Some(dz.min(run.abs() * WALKABLE) / run.abs())
        };
        for (i, s) in noses.iter().enumerate() {
            if !owned(s) {
                continue;
            }
            out.noses += 1;
            let Ok((j, run)) = up[i] else {
                if let Some(d) = uphill[i] {
                    // A top nose: the landing just beyond it should be at the nose's height.
                    out.tops += 1;
                    let (x, y) = (s.mid[0] + d[0] * 3.0, s.mid[1] + d[1] * 3.0);
                    let f = all.highest(x, y, s.z - RISER_MAX, s.z + RISER_MAX);
                    out.tops_off += f.map_or(true, |f| (f - s.z).abs() > FLUSH) as usize;
                }
                continue;
            };
            let u = &noses[j];
            let dz = u.z_at(u.along(s.mid)) - s.z;
            // The two ends of this nose, carried straight across onto the upper nose's line, at
            // that line's height there (the next strip starts on that same line, so strips meet
            // flush; tilted noses keep their tilt).
            let onto = |p: [f64; 3]| {
                let t = u.along([p[0], p[1]]);
                [u.a[0] + u.dir[0] * t, u.a[1] + u.dir[1] * t, u.z_at(t)]
            };
            let (a, b) = ([s.a[0], s.a[1], s.za], [s.b[0], s.b[1], s.zb]);
            let (da, db) = (cross(i, s.a, s.za).unwrap(), cross(i, s.b, s.zb).unwrap());
            let pieces = chain(j);
            let (a2, b2) = (hit(s.a, da, &pieces).unwrap_or_else(|| onto(a)), hit(s.b, db, &pieces).unwrap_or_else(|| onto(b)));
            // The step above's bends between a2 and b2 (sideways along this nose), in order.
            let side = |p: [f64; 2]| s.along(p);
            let (ta, tb) = (side([a2[0], a2[1]]), side([b2[0], b2[1]]));
            let mut top = vec![(ta, a2)];
            for &m in &pieces {
                let n = &noses[m];
                for (p, z) in [(n.a, n.za), (n.b, n.zb)] {
                    let t = side(p);
                    if t > ta + 0.05 && t < tb - 0.05 && !top.iter().any(|q| (q.0 - t).abs() < 0.05) {
                        top.push((t, [p[0], p[1], z]));
                    }
                }
            }
            top.push((tb, b2));
            top.sort_by(|x, y| x.0.total_cmp(&y.0));
            // Steeper than WALKABLE: the top edge comes down to that slope, leaving a lip below
            // the step above (Source steps up the lip like a small step).
            let lip = (dz - run.abs() * WALKABLE).max(0.0);
            for p in &mut top {
                p.1[2] -= lip;
            }
            // A fan from a over the first half of the top edge, from b over the rest, and one
            // triangle a-b-(the top point where they switch).
            let m = (0..top.len()).min_by(|&x, &y| (top[x].0 - s.len * 0.5).abs().total_cmp(&(top[y].0 - s.len * 0.5).abs())).unwrap();
            for k in 0..top.len() - 1 {
                out.tris.push(tri9(if k < m { a } else { b }, top[k].1, top[k + 1].1));
            }
            out.tris.push(tri9(a, b, top[m].1));
            out.strips += 1;
            out.steepest = out.steepest.max(dz.atan2(run.abs()).to_degrees());
            if uphill[i].is_none() {
                // The bottom step: down at the same slope, each end until it meets the floor
                // below (flush with it); no floor within RUN_MAX: it ends in the air there. A
                // corner shared with the next piece gets the same bottom point from both strips
                // (the gentlest of their slopes; worked out from the corner alone, so the same in
                // neighbouring chunks too).
                let end = |e: [f64; 3], d: [f64; 2], out: &mut Ramps| {
                    let k = ends.get(&key([e[0], e[1]], e[2])).into_iter().flatten().filter_map(|&m| slope(m)).fold(f64::INFINITY, f64::min);
                    for step in 1..=(RUN_MAX as i32) {
                        let t = step as f64;
                        let (x, y, zr) = (e[0] - d[0] * t, e[1] - d[1] * t, e[2] - t * k);
                        if let Some(f) = all.highest(x, y, zr - RISER_MAX, e[2] - 0.5) {
                            if f >= zr {
                                return [x, y, f];
                            }
                        }
                    }
                    out.bottoms_floating += 1;
                    [e[0] - d[0] * RUN_MAX, e[1] - d[1] * RUN_MAX, e[2] - RUN_MAX * k]
                };
                let (a0, b0) = (end(a, da, &mut out), end(b, db, &mut out));
                out.tris.push(tri9(a0, b0, b));
                out.tris.push(tri9(a0, b, a));
                out.bottoms += 1;
            }
        }
        let mut h = FNV_START;
        for t in &out.tris {
            for v in t {
                h = fnv(h, &((v * 16.0).round() as i64).to_le_bytes());
            }
        }
        out.hash = h;
        out
    }

    /// Test 131: every triangle touching lo..hi (each once), stair material or not, as text: one
    /// line each, three corners, unit normal, GMSKY_TRI_* flags (relative to the anchor). For
    /// working out the stairs from the real geometry instead of guessing (logs/tris/).
    pub fn dump(&self, lo: [f64; 3], hi: [f64; 3]) -> (usize, String) {
        let (stairs, others) = self.gather(lo, hi, 0.0);
        let mut s = format!("# box ({:.1}, {:.1}, {:.1}) - ({:.1}, {:.1}, {:.1}), relative to the anchor; flags: 1 = Skyrim stair ramp, 2 = stair material
# x1 y1 z1 x2 y2 z2 x3 y3 z3 nx ny nz flags
",
            lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]);
        for t in stairs.iter().chain(others.iter()) {
            for v in t.v {
                s.push_str(&format!("{v:.3} "));
            }
            s.push_str(&format!("{:.4} {:.4} {:.4} {}
", t.n[0], t.n[1], t.n[2], t.flags));
        }
        (stairs.len() + others.len(), s)
    }

    /// For the STUCK lines: why the stair triangles around lo..hi did or didn't make ramps.
    pub fn ramp_check(&self, lo: [f64; 3], hi: [f64; 3]) -> String {
        let (stairs, others) = self.gather(lo, hi, RAMP_MARGIN);
        let all = Heights::new(stairs.iter().chain(others.iter()).copied().collect());
        let inside = |t: &&Tri| (0..3).all(|k| t.hi[k] >= lo[k] && t.lo[k] <= hi[k]);
        let here: Vec<&Tri> = stairs.iter().copied().filter(inside).collect();
        let mut st = NoseStats::default();
        find_noses(&here, &all, &mut st);
        let noses = find_noses(&stairs, &all, &mut NoseStats::default());
        let up = pair_noses(&noses);
        let (mut paired, mut why) = (0, [0usize; 5]);
        for (n, u) in noses.iter().zip(&up) {
            if (0..2).all(|k| n.mid[k] >= lo[k] && n.mid[k] <= hi[k]) && n.z >= lo[2] && n.z <= hi[2] {
                match u {
                    Ok(_) => paired += 1,
                    Err(stage) => why[*stage] += 1,
                }
            }
        }
        let mut reasons: Vec<String> = why.iter().zip(UNPAIRED).filter(|(n, _)| **n > 0).map(|(n, r)| format!("{n} {r}")).collect();
        if reasons.is_empty() {
            reasons.push("none".into());
        }
        format!("ramp check: {} stair triangles here, {} of them treads with {} edges: {} not level, {} under 2 long, {} inside a tread, {} at a tread's back (covered above), {} under a surface (a box's bottom), {} noses; noses here: {} paired, unpaired: {}",
            st.tris, st.treads, st.edges, st.not_level, st.short, st.continues, st.covered, st.under, st.noses, paired, reasons.join(", "))
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

    /// Like `regions_arrived`, plus a fingerprint of the arrived regions' content (S2: a physics
    /// chunk is rebuilt only when it changes). Missing regions count too, so a region arriving
    /// or being dropped changes it.
    pub fn chunk_version(&self, r0: [i32; 3], r1: [i32; 3]) -> (usize, usize, u64) {
        let (mut arrived, mut total, mut h) = (0, 0, FNV_START);
        for x in r0[0]..=r1[0] {
            for y in r0[1]..=r1[1] {
                for z in r0[2]..=r1[2] {
                    total += 1;
                    let r = self.regions.get(&(x, y, z));
                    arrived += r.is_some() as usize;
                    h = fnv(h, &r.map_or(0, |r| r.hash ^ 1).to_le_bytes());
                }
            }
        }
        (arrived, total, h)
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
    fn chunk_version_changes_only_with_content() {
        let mut s = floor_store();
        let (a, t, v1) = s.chunk_version([0, 0, -1], [0, 0, 0]);
        assert_eq!((a, t), (2, 2));
        // The same region sent again: same fingerprint.
        let tris = [[-100.0, -100.0, 0.0, 100.0, -100.0, 0.0, 100.0, 100.0, 0.0], [-100.0, -100.0, 0.0, 100.0, 100.0, 0.0, -100.0, 100.0, 0.0]];
        s.apply(COL_REGION, &region_payload(0, 0, 0, 1, [-136.0; 3], 272.0, &tris));
        assert_eq!(s.chunk_version([0, 0, -1], [0, 0, 0]).2, v1);
        // One vertex moved: a new fingerprint.
        let mut moved = tris;
        moved[0][2] = 1.0;
        s.apply(COL_REGION, &region_payload(0, 0, 0, 1, [-136.0; 3], 272.0, &moved));
        let v2 = s.chunk_version([0, 0, -1], [0, 0, 0]).2;
        assert_ne!(v2, v1);
        // An empty region arriving where none was changes it too; a missing one isn't "empty".
        let (_, _, before) = s.chunk_version([0, 0, -1], [1, 0, 0]);
        s.apply(COL_REGION, &region_payload(1, 0, 0, 1, [136.0, -136.0, -136.0], 272.0, &[]));
        let (a, t, after) = s.chunk_version([0, 0, -1], [1, 0, 0]);
        assert_eq!((a, t), (3, 4));
        assert_ne!(before, after);
    }

    /// Four steps up along +x (risers `rise` high, treads `run` deep, 40 wide) with a top landing
    /// 12 deep, as Skyrim would send them; `flags` on every triangle. With `floor`, a floor at
    /// z = 0 in front of the bottom step (no stair material).
    fn stairs_msg(flags: u32, rise: f32, run: f32, floor: bool) -> Vec<u8> {
        let mut tris = Vec::new();
        for k in 0..4 {
            let (x, z0, z1) = (k as f32 * run, k as f32 * rise, (k + 1) as f32 * rise);
            let deep = if k == 3 { 12.0 } else { run };
            tris.push([x, -20.0, z0, x, 20.0, z0, x, 20.0, z1]); // riser, lower half
            tris.push([x, -20.0, z0, x, 20.0, z1, x, -20.0, z1]); // riser, upper half (its top edge is the nose)
            tris.push([x, -20.0, z1, x + deep, -20.0, z1, x + deep, 20.0, z1]); // tread
            tris.push([x, -20.0, z1, x + deep, 20.0, z1, x, 20.0, z1]);
        }
        let stair_tris = tris.len();
        if floor {
            tris.push([-60.0, -40.0, 0.0, 0.0, -40.0, 0.0, 0.0, 40.0, 0.0]);
            tris.push([-60.0, -40.0, 0.0, 0.0, 40.0, 0.0, -60.0, 40.0, 0.0]);
        }
        let mut msg = region_payload(0, 0, 0, 1, [-136.0; 3], 272.0, &tris);
        for i in 0..stair_tris {
            msg[40 + i * 40 + 36..40 + i * 40 + 40].copy_from_slice(&flags.to_le_bytes());
        }
        msg
    }

    fn store_with(msg: &[u8]) -> Store {
        let mut s = Store::default();
        clear(&mut s, 1, 1);
        s.apply(COL_REGION, msg);
        s
    }

    #[test]
    fn ramps_lie_on_the_step_noses_and_meet_floor_and_landing_flush() {
        let s = store_with(&stairs_msg(gmsky_protocol::TRI_STAIRS, 10.0, 12.0, true));
        let r = s.ramps_in_box([-136.0; 3], [136.0; 3]);
        // 13 noses: the 4 front edges, plus each tread's two side edges and the landing's back
        // edge (nothing above them); side edges of steps lie on one line, so they never pair.
        assert_eq!((r.noses, r.strips, r.bottoms, r.bottoms_floating, r.tops, r.tops_off), (13, 3, 1, 0, 1, 0));
        assert!((r.steepest - (10.0f64 / 12.0).atan().to_degrees()).abs() < 1e-6, "steepest {}", r.steepest);
        assert_eq!(r.tris.len(), 8, "two triangles per strip, three strips and the bottom one");
        let r_hash = r.hash;
        // Every corner on the board through the noses (z = 10 + x * 10 / 12) except the bottom
        // end, which sits on the floor (z = 0) where the board meets it (x = -12, one unit step).
        for t in &r.tris {
            for c in 0..3 {
                let (x, z) = (t[c * 3], t[c * 3 + 2]);
                if x < -11.0 {
                    assert!(z == 0.0 && (-13.0..=-12.0).contains(&x), "bottom end ({x}, {z}) not flush with the floor");
                } else {
                    assert!((z - (10.0 + x * 10.0 / 12.0)).abs() < 1e-9, "corner ({x}, {z}) off the board");
                    assert!(x <= 36.0, "the ramp ends at the top nose (x 36), flush with the landing");
                }
            }
        }
        let check = s.ramp_check([-136.0; 3], [136.0; 3]);
        assert!(check.contains("16 stair triangles here, 8 of them treads with 24 edges") && check.contains("13 noses; noses here: 3 paired"), "{check}");
        // A box owning only the top nose makes nothing (it has no nose above it) but checks the landing.
        let top = s.ramps_in_box([30.0, -136.0, 35.0], [136.0, 136.0, 136.0]);
        assert_eq!((top.strips, top.tris.len(), top.tops, top.tops_off), (0, 0, 1, 0));
        // No floor in front: the bottom strip ends in the air at 48 units (both ends counted).
        let r = store_with(&stairs_msg(gmsky_protocol::TRI_STAIRS, 10.0, 12.0, false)).ramps_in_box([-136.0; 3], [136.0; 3]);
        assert_eq!((r.bottoms, r.bottoms_floating), (1, 2));
        // Steps a little steeper than Source can walk (rise 14, run 12): strips at 45 degrees,
        // each 2 below the next nose at its top (a lip Source steps up).
        let r = store_with(&stairs_msg(gmsky_protocol::TRI_STAIRS, 14.0, 12.0, true)).ramps_in_box([-136.0; 3], [136.0; 3]);
        assert_eq!(r.strips, 3);
        assert!((r.steepest - (14.0f64 / 12.0).atan().to_degrees()).abs() < 1e-6, "steepest {}", r.steepest);
        for t in &r.tris {
            let (u, v) = ([t[3] - t[0], t[4] - t[1], t[5] - t[2]], [t[6] - t[0], t[7] - t[1], t[8] - t[2]]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let nz = n[2].abs() / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!(nz > 0.707 - 1e-6, "a strip steeper than 45 degrees: {t:?}");
        }
        // Much steeper (rise 15, run 6: a 9-unit lip): no strips at all.
        let r = store_with(&stairs_msg(gmsky_protocol::TRI_STAIRS, 15.0, 6.0, true)).ramps_in_box([-136.0; 3], [136.0; 3]);
        assert_eq!((r.strips, r.tris.len()), (0, 0));
        // Test 130 (big stone stairs: risers without a level top edge): only the treads are
        // used, so the same steps with risers that give nothing (no stair material) make the
        // same ramp.
        let mut msg = stairs_msg(gmsky_protocol::TRI_STAIRS, 10.0, 12.0, true);
        for k in 0..4 {
            for i in [k * 4, k * 4 + 1] {
                msg[40 + i * 40 + 36..40 + i * 40 + 40].copy_from_slice(&0u32.to_le_bytes());
            }
        }
        let bare = store_with(&msg).ramps_in_box([-136.0; 3], [136.0; 3]);
        assert_eq!((bare.strips, bare.bottoms, bare.hash), (3, 1, r_hash));
        // Same steps without a stair material: no ramp.
        let p = store_with(&stairs_msg(0, 10.0, 12.0, true)).ramps_in_box([-136.0; 3], [136.0; 3]);
        assert_eq!((p.noses, p.tris.len()), (0, 0));
    }

    /// For working on a dump: what a walking player stands on around `at`, 4-unit cells, found by
    /// walking out from the spot (each cell: the highest surface at most a step above its
    /// neighbour). Map 1: r = ramp on top, S = stair surface with no ramp, o = other surface,
    /// ! = a Skyrim surface more than half a unit above the ramp under it. Map 2: the biggest
    /// height jump to a neighbouring cell beyond a 45-degree slope, in units (. = smooth).
    fn print_walk_map(s: &Store, ramp: &[Tri], lo: [f64; 3], hi: [f64; 3], at: [f64; 3]) {
        let e = walk(s, ramp, lo, hi, at, 4.0, 16);
        println!("    walk map around the spot (x across, y up the page, 4 units a cell; @ = the spot):");
        for j in (-16..=16).rev() {
            let mut l1 = String::new();
            let mut l2 = String::new();
            for i in -16..=16 {
                let Some(&(z, c)) = e.get(&(i, j)) else {
                    l1.push(' ');
                    l2.push(' ');
                    continue;
                };
                l1.push(if (i, j) == (0, 0) { '@' } else { c });
                let jump = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().filter_map(|(di, dj)| e.get(&(i + di, j + dj))).map(|n| (n.0 - z).abs() - 4.0).fold(0.0, f64::max);
                l2.push(if jump < 1.0 { '.' } else if jump < 9.5 { char::from(b'0' + jump.round() as u8) } else { 'X' });
            }
            println!("      {l1}   {l2}");
        }
        if std::env::var("GMSKY_DUMP_HEIGHTS").is_ok() {
            println!("    heights relative to the spot (4-unit cells, ±10 cells; lower case = ramp on top):");
            for j in (-10..=10).rev() {
                let row: String = (-10..=10).map(|i| match e.get(&(i, j)) {
                    Some(&(z, c)) => format!("{:>4}{}", (z - at[2]).round() as i64, if c == 'r' || c == '!' { 'r' } else { ' ' }),
                    None => "    _".to_string(),
                }).collect();
                println!("      {row}");
            }
        }
    }

    /// Bumps a walking player meets on the ramps within 48 units of `at`: neighbouring 1-unit
    /// cells, both on a ramp or a stair surface (not the stairs' sides, where a ramp meets a wall
    /// or the floor beside it), more than 2 units apart in height (steeper than 60 degrees).
    /// Returns (bumps, of them 6+ units, of them within 24 units, cells walked).
    fn bumps(s: &Store, ramp: &[Tri], lo: [f64; 3], hi: [f64; 3], at: [f64; 3]) -> (usize, usize, usize, usize) {
        let e = walk(s, ramp, lo, hi, at, 1.0, 48);
        let (mut n, mut big, mut near) = (0, 0, 0);
        for (&(i, j), &(z, c)) in &e {
            for (di, dj) in [(1, 0), (0, 1)] {
                if let Some(&(z2, c2)) = e.get(&(i + di, j + dj)) {
                    if (z2 - z).abs() > 2.0 && c != 'o' && c2 != 'o' {
                        n += 1;
                        near += (i * i + j * j < 24 * 24) as usize;
                        if std::env::var("GMSKY_DUMP_BUMPS").is_ok() {
                            println!("      bump at ({:.0}, {:.0}) {c}{c2}: {z:.1} -> {z2:.1}", at[0] + i as f64, at[1] + j as f64);
                        }
                        big += ((z2 - z).abs() >= 6.0) as usize;
                    }
                }
            }
        }
        (n, big, near, e.len())
    }

    /// What a walking player stands on around `at`, cells of `c` units out to `n` cells, found by
    /// walking out from the spot (each cell: the highest surface at most a step above its
    /// neighbour): (height, r = ramp on top, S = stair surface with no ramp, o = other surface,
    /// ! = a Skyrim surface more than half a unit above the ramp under it).
    fn walk(s: &Store, ramp: &[Tri], lo: [f64; 3], hi: [f64; 3], at: [f64; 3], c: f64, n: i64) -> HashMap<(i64, i64), (f64, char)> {
        let (stairs, others) = s.gather(lo, hi, 0.0);
        let sky = Heights::new(stairs.iter().chain(others.iter()).copied().collect());
        let stair = Heights::new(stairs.clone());
        let rmp = Heights::new(ramp.iter().collect());
        let mut e: HashMap<(i64, i64), (f64, char)> = HashMap::new();
        let top = |x: f64, y: f64, zmin: f64, zmax: f64| {
            let (zs, zr) = (sky.highest(x, y, zmin, zmax), rmp.highest(x, y, zmin, zmax));
            let z = zs.into_iter().chain(zr).fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))))?;
            let c = match (zs, zr) {
                (Some(a), Some(r)) if a > r + 0.5 => '!',
                (_, Some(_)) => 'r',
                _ if stair.highest(x, y, z - 0.01, z + 0.01).is_some() => 'S',
                _ => 'o',
            };
            Some((z, c))
        };
        let mut queue = std::collections::VecDeque::new();
        // Start from the highest surface under the player's 32-unit box: a player standing on an
        // edge can have nothing under its centre.
        let start = [-16.0, 0.0, 16.0].iter().flat_map(|dx| [-16.0, 0.0, 16.0].map(|dy| (*dx, dy)))
            .filter_map(|(dx, dy)| top(at[0] + dx, at[1] + dy, at[2] - 60.0, at[2] + 2.0).map(|t| ((dx / c).round() as i64, (dy / c).round() as i64, t)))
            .max_by(|a, b| a.2 .0.total_cmp(&b.2 .0));
        if let Some((i, j, t)) = start {
            e.insert((i, j), t);
            queue.push_back((i, j));
        }
        while let Some((i, j)) = queue.pop_front() {
            let z0 = e[&(i, j)].0;
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (a, b) = (i + di, j + dj);
                if a.abs() > n || b.abs() > n || e.contains_key(&(a, b)) {
                    continue;
                }
                if let Some(t) = top(at[0] + a as f64 * c, at[1] + b as f64 * c, z0 - 40.0, z0 + 18.0) {
                    e.insert((a, b), t);
                    queue.push_back((a, b));
                }
            }
        }
        e
    }

    /// Test 132's dumps (logs/tris/*.txt, real Skyrim triangles; game data, so never in the repo):
    /// run by hand with GMSKY_DUMPS=<folder> and `--ignored --nocapture`. For each dump: the ramps
    /// made from it, the ramp check, and whether a ramp lies under the player's box at the spot the
    /// dump was taken at (the dump box's middle across, 48 above its bottom), and the bumps a walking
    /// player meets there (GMSKY_DUMP_MAP=<part of a name>: as maps too).
    #[test]
    #[ignore]
    fn ramps_on_real_dumps() {
        let Some(dir) = std::env::var_os("GMSKY_DUMPS") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let header = text.lines().next().unwrap();
            // "# box (lx, ly, lz) - (hx, hy, hz), ..."
            let inner: Vec<f64> = header.split(['(', ')']).enumerate().filter(|(i, _)| i % 2 == 1).take(2)
                .flat_map(|(_, s)| s.split(',').map(|v| v.trim().parse::<f64>().unwrap()).collect::<Vec<_>>()).collect();
            let (lo, hi) = ([inner[0], inner[1], inner[2]], [inner[3], inner[4], inner[5]]);
            let mut tris = Vec::new();
            let mut flags = Vec::new();
            for l in text.lines().filter(|l| !l.starts_with('#')) {
                let v: Vec<f64> = l.split_whitespace().map(|w| w.parse().unwrap()).collect();
                tris.push(std::array::from_fn::<f32, 9, _>(|i| v[i] as f32));
                flags.push(v[12] as u32);
            }
            let size = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max) as f32 + 2.0;
            let mut msg = region_payload(0, 0, 0, 1, [lo[0] as f32 - 1.0, lo[1] as f32 - 1.0, lo[2] as f32 - 1.0], size, &tris);
            for (i, f) in flags.iter().enumerate() {
                msg[40 + i * 40 + 36..40 + i * 40 + 40].copy_from_slice(&f.to_le_bytes());
            }
            let s = store_with(&msg);
            // Noses a step below the box: in the game the chunk below lays their strips.
            let r = s.ramps_in_box([lo[0], lo[1], lo[2] - RISER_MAX], hi);
            let (px, py, pz) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, lo[2] + 48.0);
            let ramp: Vec<Tri> = r.tris.iter().filter_map(|t| Tri::new(*t)).collect();
            // Under the player's 32-unit box (centre, corners, edge middles), not just its centre.
            let h = Heights::new(ramp.iter().collect());
            let under = [-16.0, 0.0, 16.0].iter().flat_map(|dx| [-16.0, 0.0, 16.0].map(|dy| (*dx, dy)))
                .filter_map(|(dx, dy)| h.highest(px + dx, py + dy, pz - 30.0, pz + 30.0)).fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))));
            let feet = s.gather([px - 16.0, py - 16.0, pz - 30.0], [px + 16.0, py + 16.0, pz + 30.0], 0.0);
            println!("{}: {} triangles ({} stair); ramps: {} noses, {} strips, {} bottoms ({} ends in the air), {} tops ({} not flush), steepest {:.1}; ramp under the spot: {}",
                path.file_name().unwrap().to_string_lossy(), tris.len(), flags.iter().filter(|f| *f & 2 != 0).count(), r.noses, r.strips, r.bottoms,
                r.bottoms_floating, r.tops, r.tops_off, r.steepest,
                under.map_or("NONE under the box".to_string(), |z| format!("yes, highest {:.1} relative to the feet", z - pz)));
            if std::env::var("GMSKY_DUMP_DETAIL").map_or(false, |d| path.to_string_lossy().contains(&d)) {
                let (stairs, others) = s.gather(lo, hi, 0.0);
                let all = Heights::new(stairs.iter().chain(others.iter()).copied().collect());
                let noses = find_noses(&stairs, &all, &mut NoseStats::default());
                let up = pair_noses(&noses);
                for (i, n) in noses.iter().enumerate() {
                    if (n.mid[0] - px).abs() < 70.0 && (n.mid[1] - py).abs() < 70.0 {
                        println!("    nose {i}: ({:.1}, {:.1}, {:.1}) - ({:.1}, {:.1}, {:.1}) -> {:?}", n.a[0], n.a[1], n.za, n.b[0], n.b[1], n.zb,
                            up[i].map(|(j, run)| format!("nose {j} run {run:.1}")).map_err(|e| UNPAIRED[e]));
                    }
                }
            }
            if let Some(q) = std::env::var("GMSKY_PROBE").ok().filter(|_| std::env::var("GMSKY_DUMP_DETAIL").map_or(false, |d| path.to_string_lossy().contains(&d))) {
                let q: Vec<f64> = q.split(',').map(|v| v.parse().unwrap()).collect();
                for (k, t) in ramp.iter().enumerate() {
                    let one = Heights::new(vec![t]);
                    if let Some(z) = one.highest(q[0], q[1], -1e9, 1e9) {
                        println!("    probe ({}, {}): ramp triangle {k} at {z:.1}: {:?}", q[0], q[1], t.v.map(|v| (v * 10.0).round() / 10.0));
                    }
                }
            }
            let (nb, big, near, cells) = bumps(&s, &ramp, lo, hi, [px, py, pz]);
            println!("    bumps on ramps/stairs within 48 units (1-unit cells, over 2 units high, not at the sides): {nb} ({big} of 6+ units, {near} within 24 units) over {cells} cells walked");
            if std::env::var("GMSKY_DUMP_MAP").map_or(false, |d| path.to_string_lossy().contains(&d)) {
                print_walk_map(&s, &ramp, lo, hi, [px, py, pz]);
            }
            println!("    {}; stair triangles within 16 of the spot: {}", s.ramp_check([px - 48.0, py - 48.0, pz - 24.0], [px + 48.0, py + 48.0, pz + 40.0]), feet.0.len());
        }
    }

    #[test]
    fn tris_info_keeps_the_stair_flags() {
        let mut s = Store::default();
        clear(&mut s, 1, 1);
        // A floor triangle and a 45-degree ramp; the ramp is marked as a stair helper and as a
        // stair material (the flags word after a triangle's 9 floats).
        let tris = [[-100.0, -100.0, 0.0, 100.0, -100.0, 0.0, 100.0, 100.0, 0.0], [0.0, -50.0, 0.0, 0.0, 50.0, 0.0, 50.0, 0.0, 50.0]];
        let mut msg = region_payload(0, 0, 0, 1, [-136.0; 3], 272.0, &tris);
        msg[40 + 40 + 36..40 + 40 + 40].copy_from_slice(&(gmsky_protocol::TRI_STAIR_HELPER | gmsky_protocol::TRI_STAIRS).to_le_bytes());
        s.apply(COL_REGION, &msg);
        assert_eq!((s.stair_helpers(), s.stairs()), (1, 1));
        let info = s.tris_info([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0], 10);
        assert_eq!(info.len(), 2);
        let ramp = info.iter().find(|t| t.3 != 0).expect("the ramp comes back marked");
        assert_eq!(ramp.3, gmsky_protocol::TRI_STAIR_HELPER | gmsky_protocol::TRI_STAIRS, "both flags kept");
        assert_eq!((ramp.0, ramp.1), (0.0, 50.0));
        assert!((ramp.2[2].abs() - 0.5f64.sqrt()).abs() < 1e-9);
        assert!(info.iter().any(|t| t.3 == 0 && t.2[2].abs() > 0.999));
        assert_eq!(s.tris_info([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0], 1).len(), 1);
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
