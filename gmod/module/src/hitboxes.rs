//! Skyrim's NPC hitboxes (Phase 3 step 3c-2, DESIGN.md "Hitboxes: how bullets hit Skyrim's NPCs").
//!
//! Skyrim sends the per-bone body shapes of the actors near the player every frame (the
//! `hitboxes` slot: capsules, exact; boxes for anything else, marked approximate). This store
//! keeps the newest table and answers `ENT:TestCollision` for the `gmsky_actor` stand-ins on both
//! Garry's Mod sides (server and client share this module):
//! - a line (bullets) is tested against every body part (not gear) and reports the part's hit
//!   group (head, chest, ...);
//! - a moving box (player movement, the crowbar's fallback trace) is tested against one fixed,
//!   upright, world-aligned box per living actor ("hull"), like Garry's Mod's own NPCs: placed at
//!   the actor's feet from the actors table (not from the animated parts, which made it wobble:
//!   test 57), half as wide as Skyrim's bound radius allows (at most 0.4 × its height), as tall as
//!   the actor. Corpses have none (you walk over bodies, as over Garry's Mod's ragdolls);
//! - a trace that starts inside a shape doesn't hit it, so nobody gets stuck when an NPC walks
//!   into them.
//!
//! Coordinates here are Garry's Mod units relative to the anchor, as Skyrim sends them; the Lua
//! functions in lib.rs convert from and to world units.

use gmsky_protocol::{Actors, Hitboxes, HitPart, HBA_DEAD, HBF_GEAR, HBG_GENERIC, HBK_CAPSULE, HB_MAX_ACTORS, HB_MAX_PARTS, MAX_ACTORS};

/// Stop this far before the surface (Source's DIST_EPSILON, as shapes.rs).
const EPS: f64 = 0.03125;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Capsule { a: [f64; 3], b: [f64; 3], r: f64 },
    Box { lo: [f64; 3], hi: [f64; 3] },
}

#[derive(Clone, Debug)]
pub struct Part {
    pub shape: Shape,
    pub group: u8,
    pub bone: u8,
    pub gear: bool,
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub id: u32,
    pub flags: u32,
    pub parts: Vec<Part>,
    /// Box around all parts (gear too).
    pub lo: [f64; 3],
    pub hi: [f64; 3],
    /// The hull moving boxes are tested against (a Shape::Box); None for corpses.
    pub body: Option<Shape>,
}

#[derive(Default)]
pub struct Store {
    pub seq: u32,
    pub origin_epoch: u32,
    pub skyrim_frame: u64,
    pub qpc: i64,
    pub actors: Vec<Actor>,
    /// The actors table's sequence number these hulls came from.
    pub actors_seq: u32,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add_scaled(a: [f64; 3], d: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t]
}
fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn f3(v: [f32; 3]) -> [f64; 3] {
    [v[0] as f64, v[1] as f64, v[2] as f64]
}

fn shape_box(s: &Shape) -> ([f64; 3], [f64; 3]) {
    match *s {
        Shape::Capsule { a, b, r } => (
            [a[0].min(b[0]) - r, a[1].min(b[1]) - r, a[2].min(b[2]) - r],
            [a[0].max(b[0]) + r, a[1].max(b[1]) + r, a[2].max(b[2]) + r],
        ),
        Shape::Box { lo, hi } => (lo, hi),
    }
}

/// First hit of the ray o + t·u (u unit length, 0 ≤ t ≤ max) with a capsule: (t, outward normal).
/// None if it misses or starts inside. (Capsule intersection after Inigo Quilez's formulation.)
pub fn ray_capsule(o: [f64; 3], u: [f64; 3], max: f64, a: [f64; 3], b: [f64; 3], r: f64) -> Option<(f64, [f64; 3])> {
    let sphere = |c: [f64; 3]| -> Option<(f64, [f64; 3])> {
        let oc = sub(o, c);
        let bb = dot(u, oc);
        let cc = dot(oc, oc) - r * r;
        if cc <= 0.0 {
            return None; // starts inside
        }
        let h = bb * bb - cc;
        if h < 0.0 {
            return None;
        }
        let t = -bb - h.sqrt();
        if t < 0.0 || t > max {
            return None;
        }
        let p = add_scaled(o, u, t);
        let n = sub(p, c);
        let l = len(n);
        Some((t, [n[0] / l, n[1] / l, n[2] / l]))
    };
    let ba = sub(b, a);
    let baba = dot(ba, ba);
    if baba < 1e-9 {
        return sphere(a);
    }
    // Inside the capsule at the start: no hit.
    let oa = sub(o, a);
    let h0 = (dot(oa, ba) / baba).clamp(0.0, 1.0);
    if len(sub(oa, [ba[0] * h0, ba[1] * h0, ba[2] * h0])) <= r {
        return None;
    }
    let bard = dot(ba, u);
    let baoa = dot(ba, oa);
    let rdoa = dot(u, oa);
    let oaoa = dot(oa, oa);
    let qa = baba - bard * bard;
    if qa > 1e-12 {
        let qb = baba * rdoa - baoa * bard;
        let qc = baba * oaoa - baoa * baoa - r * r * baba;
        let h = qb * qb - qa * qc;
        if h >= 0.0 {
            let t = (-qb - h.sqrt()) / qa;
            let y = baoa + t * bard;
            if y > 0.0 && y < baba {
                if t < 0.0 || t > max {
                    return None;
                }
                let p = add_scaled(o, u, t);
                let axis = add_scaled(a, ba, y / baba);
                let n = sub(p, axis);
                let l = len(n);
                return Some((t, [n[0] / l, n[1] / l, n[2] / l]));
            }
        }
    }
    // The end caps.
    match (sphere(a), sphere(b)) {
        (Some(x), Some(y)) => Some(if x.0 <= y.0 { x } else { y }),
        (x, y) => x.or(y),
    }
}

/// First hit of the ray with a world-aligned box: (t, normal). None if it misses or starts inside.
pub fn ray_box(o: [f64; 3], u: [f64; 3], max: f64, lo: [f64; 3], hi: [f64; 3]) -> Option<(f64, [f64; 3])> {
    if (0..3).all(|i| o[i] > lo[i] && o[i] < hi[i]) {
        return None;
    }
    let (mut tmin, mut tmax) = (0.0f64, max);
    let mut axis = usize::MAX;
    let mut sign = 0.0;
    for i in 0..3 {
        if u[i].abs() < 1e-12 {
            if o[i] < lo[i] || o[i] > hi[i] {
                return None;
            }
            continue;
        }
        let (mut t1, mut t2) = ((lo[i] - o[i]) / u[i], (hi[i] - o[i]) / u[i]);
        let mut s = -1.0;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
            s = 1.0;
        }
        if t1 > tmin {
            tmin = t1;
            axis = i;
            sign = s;
        }
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    if axis == usize::MAX {
        return None;
    }
    let mut n = [0.0; 3];
    n[axis] = sign;
    Some((tmin, n))
}

fn ray_shape(o: [f64; 3], u: [f64; 3], max: f64, s: &Shape) -> Option<(f64, [f64; 3])> {
    match *s {
        Shape::Capsule { a, b, r } => ray_capsule(o, u, max, a, b, r),
        Shape::Box { lo, hi } => ray_box(o, u, max, lo, hi),
    }
}

/// Distance from p to a shape's surface (negative inside).
pub fn distance(p: [f64; 3], s: &Shape) -> f64 {
    match *s {
        Shape::Capsule { a, b, r } => {
            let ba = sub(b, a);
            let baba = dot(ba, ba);
            let h = if baba < 1e-12 { 0.0 } else { (dot(sub(p, a), ba) / baba).clamp(0.0, 1.0) };
            len(sub(p, add_scaled(a, ba, h))) - r
        }
        Shape::Box { lo, hi } => {
            let mut out = [0.0; 3];
            let mut inside = f64::MAX;
            for i in 0..3 {
                out[i] = (lo[i] - p[i]).max(p[i] - hi[i]).max(0.0);
                inside = inside.min((p[i] - lo[i]).min(hi[i] - p[i]));
            }
            let d = len(out);
            if d > 0.0 { d } else { -inside }
        }
    }
}

impl Actor {
    /// `feet`: the actor's position, Skyrim's bound radius and height from the actors table
    /// (Garry's Mod units relative to the anchor), if it is there.
    fn from_parts(id: u32, flags: u32, parts: Vec<Part>, feet: Option<([f64; 3], f64, f64)>) -> Actor {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        let (mut blo, mut bhi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in &parts {
            let (l, h) = shape_box(&p.shape);
            for i in 0..3 {
                lo[i] = lo[i].min(l[i]);
                hi[i] = hi[i].max(h[i]);
                if !p.gear {
                    blo[i] = blo[i].min(l[i]);
                    bhi[i] = bhi[i].max(h[i]);
                }
            }
        }
        if blo[0] > bhi[0] {
            (blo, bhi) = (lo, hi);
        }
        let body = if flags & HBA_DEAD != 0 {
            None
        } else if let Some((p, radius, height)) = feet {
            let w = radius.min(0.4 * height).clamp(6.0, 24.0);
            Some(Shape::Box { lo: [p[0] - w, p[1] - w, p[2]], hi: [p[0] + w, p[1] + w, p[2] + height.max(8.0)] })
        } else {
            // Not in the actors table (yet): around the parts, as wide as the narrower side.
            let w = (0.5 * (bhi[0] - blo[0]).min(bhi[1] - blo[1])).clamp(4.0, 24.0);
            let (cx, cy) = ((blo[0] + bhi[0]) * 0.5, (blo[1] + bhi[1]) * 0.5);
            Some(Shape::Box { lo: [cx - w, cy - w, blo[2]], hi: [cx + w, cy + w, bhi[2]] })
        };
        Actor { id, flags, parts, lo, hi, body }
    }

    /// The first part a line from s along d (0..1 of d) hits: (fraction, normal, group, bone).
    /// A box with half size `ext` (s = its centre) is tested against the hull instead, widened by it.
    pub fn trace(&self, s: [f64; 3], d: [f64; 3], ext: [f64; 3], gear: bool) -> Option<(f64, [f64; 3], u8, u8)> {
        let l = len(d);
        if l < 1e-9 {
            return None;
        }
        let u = [d[0] / l, d[1] / l, d[2] / l];
        if ext.iter().any(|&e| e > 0.0) {
            let Some(Shape::Box { lo, hi }) = self.body else { return None };
            let lo2 = [lo[0] - ext[0], lo[1] - ext[1], lo[2] - ext[2]];
            let hi2 = [hi[0] + ext[0], hi[1] + ext[1], hi[2] + ext[2]];
            return ray_box(s, u, l, lo2, hi2).map(|(t, n)| (((t - EPS).max(0.0)) / l, n, HBG_GENERIC, 255));
        }
        let mut best: Option<(f64, [f64; 3], u8, u8)> = None;
        for p in &self.parts {
            if p.gear && !gear {
                continue;
            }
            if let Some((t, n)) = ray_shape(s, u, l, &p.shape) {
                if best.map_or(true, |b| t < b.0) {
                    best = Some((t, n, p.group, p.bone));
                }
            }
        }
        best.map(|(t, n, g, b)| (((t - EPS).max(0.0)) / l, n, g, b))
    }

    /// The part nearest to p (not gear): (group, bone, distance to its surface).
    pub fn part_at(&self, p: [f64; 3]) -> Option<(u8, u8, f64)> {
        self.parts
            .iter()
            .filter(|x| !x.gear)
            .map(|x| (x.group, x.bone, distance(p, &x.shape)))
            .min_by(|a, b| a.2.total_cmp(&b.2))
    }
}

impl Store {
    /// Takes a new table from Skyrim, with the actors table (same origin) for the hulls.
    pub fn load(&mut self, t: &Hitboxes, seq: u32, actors: Option<(&Actors, u32)>) {
        self.seq = seq;
        self.actors_seq = actors.map_or(0, |(_, s)| s);
        let actors = actors.map(|(a, _)| a).filter(|a| a.origin_epoch == t.origin_epoch);
        self.origin_epoch = t.origin_epoch;
        self.skyrim_frame = t.skyrim_frame;
        self.qpc = t.qpc;
        self.actors.clear();
        let parts_n = (t.part_count as usize).min(HB_MAX_PARTS);
        for a in &t.actors[..(t.actor_count as usize).min(HB_MAX_ACTORS)] {
            let first = a.first_part as usize;
            let end = (first + a.part_count as usize).min(parts_n);
            if first >= end {
                continue;
            }
            let parts = t.parts[first..end].iter().map(part_from).collect();
            let feet = actors.and_then(|tab| {
                tab.actors[..(tab.count as usize).min(MAX_ACTORS)].iter().find(|x| x.form_id == a.form_id)
            }).map(|x| ([x.pos[0] as f64, x.pos[1] as f64, x.pos[2] as f64], x.radius as f64, x.height as f64));
            self.actors.push(Actor::from_parts(a.form_id, a.flags, parts, feet));
        }
    }

    pub fn actor(&self, id: u32) -> Option<&Actor> {
        self.actors.iter().find(|a| a.id == id)
    }
}

fn part_from(p: &HitPart) -> Part {
    let shape = if p.kind == HBK_CAPSULE {
        Shape::Capsule { a: f3(p.a), b: f3(p.b), r: p.r as f64 }
    } else {
        let (a, b) = (f3(p.a), f3(p.b));
        Shape::Box { lo: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])], hi: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])] }
    };
    Part { shape, group: p.group, bone: p.bone, gear: p.flags & HBF_GEAR != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gmsky_protocol::{HitActor, HBG_CHEST, HBG_GEAR, HBG_HEAD, HBK_BOX};

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn capsule_side_cap_inside_parallel() {
        // Vertical capsule from z 0 to 10, radius 2, at the origin.
        let (a, b, r) = ([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], 2.0);
        // From the side at z 5: hits the side at x = -2.
        let (t, n) = ray_capsule([-10.0, 0.0, 5.0], [1.0, 0.0, 0.0], 100.0, a, b, r).unwrap();
        assert!(near(t, 8.0) && near(n[0], -1.0) && near(n[2], 0.0), "{t} {n:?}");
        // From above: hits the top cap at z 12.
        let (t, n) = ray_capsule([0.0, 0.0, 20.0], [0.0, 0.0, -1.0], 100.0, a, b, r).unwrap();
        assert!(near(t, 8.0) && near(n[2], 1.0), "{t} {n:?}");
        // Grazing past the cap's rim at an angle hits the cap sphere, not the cylinder.
        let (t, _) = ray_capsule([-10.0, 0.0, 11.0], [1.0, 0.0, 0.0], 100.0, a, b, r).unwrap();
        assert!(near(t, 10.0 - 3.0f64.sqrt()), "{t}");
        // Misses: too high, too far to the side, too short.
        assert!(ray_capsule([-10.0, 0.0, 12.5], [1.0, 0.0, 0.0], 100.0, a, b, r).is_none());
        assert!(ray_capsule([-10.0, 3.0, 5.0], [1.0, 0.0, 0.0], 100.0, a, b, r).is_none());
        assert!(ray_capsule([-10.0, 0.0, 5.0], [1.0, 0.0, 0.0], 7.9, a, b, r).is_none());
        // Starting inside: no hit (lets the player out).
        assert!(ray_capsule([0.5, 0.0, 5.0], [1.0, 0.0, 0.0], 100.0, a, b, r).is_none());
        // Along the axis from below (the cylinder part is parallel): the bottom cap.
        let (t, _) = ray_capsule([0.0, 0.0, -10.0], [0.0, 0.0, 1.0], 100.0, a, b, r).unwrap();
        assert!(near(t, 8.0), "{t}");
        // A sphere (a == b).
        let (t, _) = ray_capsule([-10.0, 0.0, 0.0], [1.0, 0.0, 0.0], 100.0, a, a, r).unwrap();
        assert!(near(t, 8.0), "{t}");
        // Pointing away: no hit.
        assert!(ray_capsule([-10.0, 0.0, 5.0], [-1.0, 0.0, 0.0], 100.0, a, b, r).is_none());
    }

    #[test]
    fn box_hits() {
        let (lo, hi) = ([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        let (t, n) = ray_box([-5.0, 1.0, 1.0], [1.0, 0.0, 0.0], 100.0, lo, hi).unwrap();
        assert!(near(t, 5.0) && n == [-1.0, 0.0, 0.0]);
        let (t, n) = ray_box([1.0, 1.0, 9.0], [0.0, 0.0, -1.0], 100.0, lo, hi).unwrap();
        assert!(near(t, 7.0) && n == [0.0, 0.0, 1.0]);
        assert!(ray_box([-5.0, 3.0, 1.0], [1.0, 0.0, 0.0], 100.0, lo, hi).is_none());
        assert!(ray_box([1.0, 1.0, 1.0], [1.0, 0.0, 0.0], 100.0, lo, hi).is_none(), "inside");
        assert!(near(distance([5.0, 1.0, 1.0], &Shape::Box { lo, hi }), 3.0));
        assert!(near(distance([1.0, 1.0, 1.5], &Shape::Box { lo, hi }), -0.5));
    }

    /// A made-up person: a body capsule, a head sphere above it, a bow (gear) in front.
    fn person() -> Hitboxes {
        let mut t = Hitboxes::default();
        t.actor_count = 1;
        t.part_count = 3;
        t.origin_epoch = 4;
        t.actors[0] = HitActor { form_id: 0x1234, first_part: 0, part_count: 3, flags: 0, reserved0: 0 };
        t.parts[0] = HitPart { a: [0.0, 0.0, 10.0], b: [0.0, 0.0, 50.0], r: 8.0, kind: HBK_CAPSULE, group: HBG_CHEST, flags: 0, bone: 0 };
        t.parts[1] = HitPart { a: [0.0, 0.0, 64.0], b: [0.0, 0.0, 64.0], r: 5.0, kind: HBK_CAPSULE, group: HBG_HEAD, flags: 0, bone: 1 };
        t.parts[2] = HitPart { a: [-20.0, -5.0, 20.0], b: [-15.0, 5.0, 60.0], r: 0.0, kind: HBK_BOX, group: HBG_GEAR, flags: HBF_GEAR, bone: 2 };
        t
    }

    #[test]
    fn store_traces_parts_and_body() {
        let mut s = Store::default();
        s.load(&person(), 7, None);
        let a = s.actor(0x1234).unwrap();
        assert_eq!(a.parts.len(), 3);
        // A bullet at head height from the -x side hits the head (the bow is gear: skipped).
        let (f, n, g, bone) = a.trace([-100.0, 0.0, 64.0], [200.0, 0.0, 0.0], [0.0; 3], false).unwrap();
        assert_eq!((g, bone), (HBG_HEAD, 1));
        assert!(near(f, (95.0 - EPS) / 200.0) && near(n[0], -1.0), "{f} {n:?}");
        // With gear on, the bow (in front, box from x -20 to -15) is hit first at chest height.
        let (_, _, g, _) = a.trace([-100.0, 0.0, 30.0], [200.0, 0.0, 0.0], [0.0; 3], true).unwrap();
        assert_eq!(g, HBG_GEAR);
        let (_, _, g, _) = a.trace([-100.0, 0.0, 30.0], [200.0, 0.0, 0.0], [0.0; 3], false).unwrap();
        assert_eq!(g, HBG_CHEST);
        // Between the head and the shoulders' top (z 58, outside both): a miss.
        assert!(a.trace([-100.0, 0.0, 58.5], [200.0, 0.0, 0.0], [0.0; 3], false).is_none());
        // A moving box (player) without the actors table: a hull around the parts, half as wide as
        // the narrower side (16 / 2 = 8), widened by the box's 16.
        let (f, n, g, bone) = a.trace([-100.0, 0.0, 30.0], [200.0, 0.0, 0.0], [16.0, 16.0, 36.0], false).unwrap();
        assert_eq!((g, bone), (HBG_GENERIC, 255));
        assert!(near(f * 200.0, 100.0 - 8.0 - 16.0 - EPS) && n == [-1.0, 0.0, 0.0], "{} {n:?}", f * 200.0);
        // With the actors table: a fixed hull at the feet, half width min(radius 12, 0.4 × 70) = 12,
        // 70 tall; flat-faced (a box, like Garry's Mod's NPCs), whatever the parts do.
        let mut tab = Actors::default();
        tab.count = 1;
        tab.origin_epoch = 4;
        tab.actors[0].form_id = 0x1234;
        tab.actors[0].pos = [0.0, 0.0, 0.0];
        tab.actors[0].radius = 12.0;
        tab.actors[0].height = 70.0;
        let mut s2 = Store::default();
        s2.load(&person(), 8, Some((&tab, 3)));
        let b = s2.actor(0x1234).unwrap();
        let (f, _, _, _) = b.trace([-100.0, 0.0, 36.0], [200.0, 0.0, 0.0], [16.0, 16.0, 36.0], false).unwrap();
        assert!(near(f * 200.0, 100.0 - 12.0 - 16.0 - EPS), "{}", f * 200.0);
        // Walking past its corner: a box face, not a curve (y = 27.9 still hits, 28.1 misses).
        assert!(b.trace([-100.0, 27.9, 36.0], [200.0, 0.0, 0.0], [16.0, 16.0, 36.0], false).is_some());
        assert!(b.trace([-100.0, 28.1, 36.0], [200.0, 0.0, 0.0], [16.0, 16.0, 36.0], false).is_none());
        // Starting inside (an NPC walked into the player): no hit, so the player can get out.
        assert!(b.trace([0.0, 0.0, 36.0], [200.0, 0.0, 0.0], [16.0, 16.0, 36.0], false).is_none());
        // A corpse has no hull: you walk over it; bullets still hit its parts.
        let mut dead = person();
        dead.actors[0].flags = HBA_DEAD;
        let mut s3 = Store::default();
        s3.load(&dead, 9, Some((&tab, 3)));
        let c = s3.actor(0x1234).unwrap();
        assert!(c.trace([-100.0, 0.0, 36.0], [200.0, 0.0, 0.0], [16.0, 16.0, 36.0], false).is_none());
        assert!(c.trace([-100.0, 0.0, 64.0], [200.0, 0.0, 0.0], [0.0; 3], false).is_some());
        // Nearest part to a point just right of the head.
        let (g, bone, d) = a.part_at([7.0, 0.0, 64.0]).unwrap();
        assert_eq!((g, bone), (HBG_HEAD, 1));
        assert!(near(d, 2.0));
    }
}
