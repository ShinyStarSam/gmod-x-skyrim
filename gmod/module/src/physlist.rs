//! STATUS.md idea 2 (DESIGN.md "count the lists directly"): reads the physics engine's per-object
//! list of pair records, to see whose list fills up to 65,532 before vphysics.dll crashes, and
//! with what. **Read only:** nothing is called on the engine's objects and nothing is written.
//! Every read goes through `ReadProcessMemory` on our own process (d3d.rs `read`), so a wrong
//! guess about a layout gives "unreadable", not a crash.
//!
//! Layout, from Ghidra on vphysics.dll (2026-10-06; MODLOG has the addresses):
//! - A pair record ("mindist") holds two halves ("synapses") at record +0x28 and +0x60; each
//!   half points at its engine object ("IVP object") at half +0x20 (`+0x99380`, `+0x97c40`).
//! - The object's list is at object +0xa0: +0x00 slots allocated (u16, at most 0xfffc), +0x02
//!   first free slot (u16, 0xffff = none), +0x08 the slots (24 bytes each; slot +0x10 = the half,
//!   free slots chain through slot +0x04), +0x1c live entries (u32; +1 per add `+0xaaed0`, -1 per
//!   remove `+0xab1b0`).
//!
//! From Lua to the IVP object is **not** from Ghidra, so it is checked every time: a Lua PhysObj
//! is a Garry's Mod userdata whose first 8 bytes should point at Valve's physics object
//! (CPhysicsObject, function table inside vphysics.dll); one of its first pointers should be the
//! IVP object (Source SDK: +0x10), whose function table is also in vphysics.dll and which points
//! back at the CPhysicsObject. The offsets found are logged once.

use crate::d3d::{image, read, read_u64, Image};
use crate::lua::{self, Api, State};
use std::collections::HashMap;
use std::ffi::c_int;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const LIST: usize = 0xa0;
const SLOT: usize = 24;
const LUA_TUSERDATA: c_int = 7;

static VPHYSICS: OnceLock<Option<Image>> = OnceLock::new();
/// (CPhysicsObject -> IVP object offset, IVP object -> CPhysicsObject back-pointer offset).
static OFFSETS: Mutex<Option<(usize, usize)>> = Mutex::new(None);

fn vphysics() -> Option<&'static Image> {
    VPHYSICS.get_or_init(|| image("vphysics.dll")).as_ref()
}

fn u16_at(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}
fn u32_at(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}
fn u64_at(b: &[u8], at: usize) -> usize {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) as usize
}

/// True if `obj` is readable and its first 8 bytes (its function table) point into vphysics.dll.
fn engine_object(vp: &Image, obj: usize) -> bool {
    obj != 0 && read_u64(obj).is_some_and(|vt| vp.contains(vt))
}

/// The PhysObj at stack index `idx` -> its IVP object. `found` gets a line the first time the
/// offsets are worked out (to be logged once).
unsafe fn ivp_of(a: &Api, l: *mut State, idx: c_int, found: &mut Option<String>) -> Result<usize, String> {
    if (a.type_)(l, idx) != LUA_TUSERDATA {
        return Err("not a userdata".into());
    }
    let vp = vphysics().ok_or("vphysics.dll is not loaded")?;
    let ud = (a.touserdata)(l, idx) as usize;
    let cphys = read_u64(ud).ok_or("userdata unreadable")?;
    if !engine_object(vp, cphys) {
        return Err(format!("the userdata's first pointer 0x{cphys:x} is not an object of vphysics.dll"));
    }
    let mut offs = OFFSETS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((to_ivp, back)) = *offs {
        let ivp = read_u64(cphys + to_ivp).unwrap_or(0);
        if engine_object(vp, ivp) && read_u64(ivp + back) == Some(cphys) {
            return Ok(ivp);
        }
        return Err(format!("the IVP object at CPhysicsObject+0x{to_ivp:x} doesn't point back at +0x{back:x}"));
    }
    // First time: find the pointer to an engine object that points back at this CPhysicsObject.
    let head = read(cphys, 0x80).ok_or("CPhysicsObject unreadable")?;
    for to_ivp in (8..0x80).step_by(8) {
        let ivp = u64_at(&head, to_ivp);
        if !engine_object(vp, ivp) {
            continue;
        }
        let Some(body) = read(ivp, 0x200) else { continue };
        if let Some(back) = (8..0x200).step_by(8).find(|&o| u64_at(&body, o) == cphys) {
            *offs = Some((to_ivp, back));
            *found = Some(format!("physlist: PhysObj userdata -> CPhysicsObject (function table vphysics+0x{:x}) -> IVP object at +0x{to_ivp:x} (function table vphysics+0x{:x}), which points back at +0x{back:x}",
                read_u64(cphys).unwrap_or(0).wrapping_sub(vp.base), u64_at(&body, 0).wrapping_sub(vp.base)));
            return Ok(ivp);
        }
    }
    Err("no IVP object pointing back at the CPhysicsObject in its first 0x80 bytes".into())
}

/// The CPhysicsObject an IVP object points back at (0 if the offsets aren't known yet).
fn cphys_of(ivp: usize) -> usize {
    let offs = *OFFSETS.lock().unwrap_or_else(|e| e.into_inner());
    offs.and_then(|(_, back)| read_u64(ivp + back)).unwrap_or(0)
}

/// Whether an IVP object still looks alive: its function table is in vphysics.dll and it points
/// back at a CPhysicsObject that points at it. (Freed memory can still look alive; not proof.)
fn looks_alive(vp: &Image, ivp: usize) -> bool {
    let offs = *OFFSETS.lock().unwrap_or_else(|e| e.into_inner());
    let Some((to_ivp, _)) = offs else { return false };
    let c = cphys_of(ivp);
    engine_object(vp, ivp) && engine_object(vp, c) && read_u64(c + to_ivp) == Some(ivp)
}

/// Valve's "game data" of a CPhysicsObject (Source SDK: the first field, +0x08, the entity).
/// Not checked against Ghidra: only used to say two objects share it.
fn game_data(ivp: usize) -> usize {
    match cphys_of(ivp) {
        0 => 0,
        c => read_u64(c + 8).unwrap_or(0),
    }
}

/// The list header: (slots allocated, first free slot, slots address, live entries).
fn header(ivp: usize) -> Option<(usize, usize, usize, usize)> {
    let h = read(ivp + LIST, 0x20)?;
    Some((u16_at(&h, 0), u16_at(&h, 2), u64_at(&h, 8), u32_at(&h, 0x1c)))
}

fn log_found(side: u32, found: Option<String>) {
    if let Some(line) = found {
        crate::log(side, &line);
    }
}

/// gmsky.PhysList(physobj) -> live entries, slots allocated; or nil, why.
pub unsafe extern "C" fn l_phys_list(l: *mut State) -> c_int {
    crate::guarded(l, |a, side| {
        let mut found = None;
        let r = ivp_of(a, l, 1, &mut found);
        log_found(side, found);
        match r.and_then(|ivp| header(ivp).ok_or_else(|| "list header unreadable".to_string())) {
            Ok((slots, _, _, live)) => {
                (a.pushnumber)(l, live as f64);
                (a.pushnumber)(l, slots as f64);
                2
            }
            Err(e) => {
                (a.settop)(l, 0);
                (a.pushnil)(l);
                lua::push_str(a, l, &e);
                2
            }
        }
    })
}

/// gmsky.PhysAddress(physobj) -> the IVP object's address as "0x..." (to match a crash dump's
/// registers: at `vphysics+0xaaff0`, RDI is the object whose list overflowed); or nil, why.
pub unsafe extern "C" fn l_phys_address(l: *mut State) -> c_int {
    crate::guarded(l, |a, side| {
        let mut found = None;
        let r = ivp_of(a, l, 1, &mut found);
        log_found(side, found);
        match r {
            Ok(ivp) => lua::push_str(a, l, &format!("0x{ivp:x}")),
            Err(e) => {
                (a.settop)(l, 0);
                (a.pushnil)(l);
                lua::push_str(a, l, &e);
                return 2;
            }
        }
        1
    })
}

/// gmsky.PhysListPartners(physobj, others) -> table, or nil, why. For every live entry of the
/// object's list: the object on the other side of the pair record, counted in t[i] when it is
/// others[i]. Also: live (the +0x1c counter), slots, chained_live (slots minus the free chain:
/// should equal live), unlisted (records with an object not in `others`), unlisted_objects (how
/// many different ones), not_records (entries that don't look like a record half), ambiguous
/// (look like either half), ms (time taken), and `unlisted_list`: up to 6 of the objects not in
/// `others`, most records first, each { addr = "0x...", records, live, slots, alive, same }
/// (`same` = index in `others` of an object with the same game data, i.e. the same entity; 0 = none).
pub unsafe extern "C" fn l_phys_list_partners(l: *mut State) -> c_int {
    crate::guarded(l, |a, side| {
        let t0 = Instant::now();
        let mut found = None;
        let r = ivp_of(a, l, 1, &mut found);
        log_found(side, found);
        let fail = |why: String| {
            (a.settop)(l, 0);
            (a.pushnil)(l);
            lua::push_str(a, l, &why);
            2
        };
        let ivp = match r {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let Some(vp) = vphysics() else { return fail("vphysics.dll is not loaded".into()) };
        // others[i] -> i
        let mut index: HashMap<usize, usize> = HashMap::new();
        let mut by_game: HashMap<usize, usize> = HashMap::new();
        let mut n_others = 0;
        if (a.type_)(l, 2) == 5 {
            n_others = (a.objlen)(l, 2);
            for i in 1..=n_others {
                (a.rawgeti)(l, 2, i as c_int);
                let mut f = None;
                if let Ok(p) = ivp_of(a, l, -1, &mut f) {
                    index.entry(p).or_insert(i);
                    let g = game_data(p);
                    if g != 0 {
                        by_game.entry(g).or_insert(i);
                    }
                }
                log_found(side, f);
                (a.settop)(l, -2);
            }
        }
        let Some((slots, free, base, live)) = header(ivp) else { return fail("list header unreadable".into()) };
        let Some(buf) = (if slots > 0 { read(base, slots * SLOT) } else { Some(Vec::new()) }) else {
            return fail(format!("{slots} slots at 0x{base:x} unreadable"));
        };
        let mut is_free = vec![false; slots];
        let (mut s, mut steps) = (free, 0);
        while s < slots && !is_free[s] && steps <= slots {
            is_free[s] = true;
            s = u16_at(&buf, s * SLOT + 4);
            steps += 1;
        }
        let mut counts = vec![0usize; n_others + 1];
        let (mut chained_live, mut unlisted, mut not_records, mut ambiguous) = (0, 0, 0, 0);
        let mut unlisted_objects: HashMap<usize, usize> = HashMap::new();
        let mut is_engine: HashMap<usize, bool> = HashMap::new();
        for e in (0..slots).filter(|&e| !is_free[e]) {
            chained_live += 1;
            let half = u64_at(&buf, e * SLOT + 0x10);
            // 0xc0 bytes around the half: record +0 if it is the second half (half - 0x60),
            // or record +0 at half - 0x28 if it is the first; the partner's half is the other.
            let Some(w) = half.checked_sub(0x60).and_then(|p| read(p, 0xc0)) else { not_records += 1; continue };
            if u64_at(&w, 0x80) != ivp {
                not_records += 1;
                continue;
            }
            let second = vp.contains(u64_at(&w, 0x00)); // record's function table at half - 0x60
            let first = vp.contains(u64_at(&w, 0x38)); // ... or at half - 0x28
            let partner = match (first, second) {
                (true, false) => u64_at(&w, 0xb8),  // record + 0x60 + 0x20 = half + 0x58
                (false, true) => u64_at(&w, 0x48),  // record + 0x28 + 0x20 = half - 0x18
                (true, true) => { ambiguous += 1; continue }
                (false, false) => { not_records += 1; continue }
            };
            if !*is_engine.entry(partner).or_insert_with(|| engine_object(vp, partner)) {
                not_records += 1;
                continue;
            }
            match index.get(&partner) {
                Some(&i) => counts[i] += 1,
                None => {
                    unlisted += 1;
                    *unlisted_objects.entry(partner).or_insert(0) += 1;
                }
            }
        }
        (a.createtable)(l, n_others as c_int, 10);
        for (i, &c) in counts.iter().enumerate().skip(1) {
            (a.pushnumber)(l, c as f64);
            (a.rawseti)(l, -2, i as c_int);
        }
        for (k, v) in [
            ("live", live),
            ("slots", slots),
            ("chained_live", chained_live),
            ("unlisted", unlisted),
            ("unlisted_objects", unlisted_objects.len()),
            ("not_records", not_records),
            ("ambiguous", ambiguous),
        ] {
            (a.pushnumber)(l, v as f64);
            lua::set_field(a, l, -2, k);
        }
        let mut worst: Vec<(usize, usize)> = unlisted_objects.iter().map(|(&p, &n)| (p, n)).collect();
        worst.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        (a.createtable)(l, worst.len().min(6) as c_int, 0);
        for (k, &(p, n)) in worst.iter().take(6).enumerate() {
            let (live, slots) = header(p).map(|h| (h.3, h.0)).unwrap_or((0, 0));
            let g = game_data(p);
            (a.createtable)(l, 0, 6);
            lua::push_str(a, l, &format!("0x{p:x}"));
            lua::set_field(a, l, -2, "addr");
            for (key, v) in [("records", n), ("live", live), ("slots", slots), ("same", if g != 0 { by_game.get(&g).copied().unwrap_or(0) } else { 0 })] {
                (a.pushnumber)(l, v as f64);
                lua::set_field(a, l, -2, key);
            }
            (a.pushboolean)(l, looks_alive(vp, p) as c_int);
            lua::set_field(a, l, -2, "alive");
            (a.rawseti)(l, -2, k as c_int + 1);
        }
        lua::set_field(a, l, -2, "unlisted_list");
        (a.pushnumber)(l, t0.elapsed().as_secs_f64() * 1000.0);
        lua::set_field(a, l, -2, "ms");
        1
    })
}
