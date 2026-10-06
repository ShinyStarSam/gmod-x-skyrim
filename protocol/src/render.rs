//! Rust copy of the render block in `protocol/gmsky_protocol.h` ("Render block"; the header is the
//! source of truth, the layout checks below assert the same numbers). Geometry route, step G2
//! (DESIGN.md §12): Garry's Mod sends its models, textures and, every frame, where its objects are;
//! Skyrim draws them in its own frame. A byte ring like SkyCraft's render ring (MIT,
//! skycraft_protocol.h `kRenRing*`, Link.cpp `DrainRender`) and our collision ring. Garry's Mod
//! writes; Skyrim reads (the unit tests and `fake_render` play the other side).

use crate::shm::{map_named, unmap};
use crate::{ColMsgHeader, MAGIC_INIT};
use std::mem::{offset_of, size_of};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::*};

pub const SHM_NAME: &str = r"Local\GmodSkyrim_render_v1";
pub const MAGIC: u32 = 0x444E_524D; // "MRND"
pub const VERSION: u32 = 3; // 2: the frame carries Garry's Mod's view; objects their entity and flags (G5a + G6); 3: tangents, normal maps, phong
pub const RING_BYTES: usize = 64 << 20;
pub const HEAD_OFF: usize = 0x1000;
pub const TAIL_OFF: usize = 0x1040;
pub const DATA_OFF: usize = 0x1080;
pub const MAP_SIZE: usize = DATA_OFF + RING_BYTES;

pub const MSG_PAD: u32 = 0;
pub const MSG_RESET: u32 = 1;
pub const MSG_TEXTURE: u32 = 2;
pub const MSG_MODEL: u32 = 3;
pub const MSG_FORGET: u32 = 4;
pub const MSG_FRAME: u32 = 5;

pub const TEX_BC1: u32 = 1;
pub const TEX_BC2: u32 = 2;
pub const TEX_BC3: u32 = 3;
pub const TEX_RGBA8: u32 = 4;

pub const MAT_TRANSLUCENT: u32 = 0x1;
pub const MAT_ADDITIVE: u32 = 0x2;
pub const MAT_ALPHATEST: u32 = 0x4;
pub const MAT_SELFILLUM: u32 = 0x8;
pub const MAT_NOCULL: u32 = 0x10;
pub const MAT_PHONG: u32 = 0x20;
pub const MAT_BASEALPHA_PHONGMASK: u32 = 0x40;
pub const TEXF_LINEAR: u32 = 0x1;

pub const FRAME_VIEW: u32 = 0x1;
/// Switches from Garry's Mod's console (see gmsky_protocol.h).
pub const FRAME_FLIP_GREEN: u32 = 0x2;
pub const FRAME_NO_NORMALS: u32 = 0x4;
pub const FRAME_NO_PHONG: u32 = 0x8;
pub const FRAME_NO_SHADOWS: u32 = 0x10;
pub const OBJ_HELD: u32 = 0x1;

pub const FORGET_MODEL: u32 = 1;
pub const FORGET_TEXTURE: u32 = 2;

#[repr(C)]
pub struct Ctl {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    pub map_size: AtomicU64,
    /// Garry's Mod's stream (process id); 0 = no writer yet.
    pub writer_session: AtomicU32,
    /// +1 each time a Skyrim attaches: Garry's Mod then sends RESET and everything again.
    pub reader_epoch: AtomicU32,
    pub messages_written: AtomicU64,
    pub messages_read: AtomicU64,
    pub writes_refused: AtomicU64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reset {
    pub session: u32,
    pub reader_epoch: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Texture {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub bytes: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Model {
    pub id: u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub part_count: u32,
    pub bone_count: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part {
    pub first_index: u32,
    pub index_count: u32,
    pub texture_id: u32,
    pub flags: u32,
    pub alpha_ref: f32,
    pub normal_texture_id: u32,
    pub phong_exponent: f32,
    pub phong_boost: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// xyz model space, w handedness (+1 / -1).
    pub tangent: [f32; 4],
    pub u: f32,
    pub v: f32,
    pub bones: [u8; 4],
    pub weights: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Forget {
    pub kind: u32,
    pub id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub frame: u64,
    pub qpc: i64,
    pub origin_epoch: u32,
    pub object_count: u32,
    pub matrix_count: u32,
    pub flags: u32,
    /// Garry's Mod's camera for this frame, relative to the anchor; pitch, yaw, roll in degrees.
    pub view_eye: [f32; 3],
    pub view_angles: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Object {
    pub model_id: u32,
    pub first_matrix: u32,
    pub matrix_count: u32,
    pub color: u32,
    pub flags: u32,
    /// Garry's Mod's entity number (for motion vectors).
    pub entity: u32,
}

/// Rows of [rotation | translation]: a model-space point -> Garry's Mod units relative to the anchor.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Matrix {
    pub m: [[f32; 4]; 3],
}

const _: () = {
    assert!(size_of::<Ctl>() == 48);
    assert!(offset_of!(Ctl, writer_session) == 16);
    assert!(offset_of!(Ctl, messages_written) == 24);
    assert!(offset_of!(Ctl, writes_refused) == 40);
    assert!(size_of::<Reset>() == 8);
    assert!(size_of::<Texture>() == 24);
    assert!(size_of::<Model>() == 24);
    assert!(size_of::<Part>() == 32);
    assert!(size_of::<Vertex>() == 56);
    assert!(offset_of!(Vertex, u) == 40);
    assert!(offset_of!(Vertex, bones) == 48);
    assert!(size_of::<Forget>() == 8);
    assert!(size_of::<Frame>() == 56);
    assert!(offset_of!(Frame, view_eye) == 32);
    assert!(offset_of!(Frame, origin_epoch) == 16);
    assert!(size_of::<Object>() == 24);
    assert!(size_of::<Matrix>() == 48);
    assert!(MAP_SIZE == 67_113_088);
};

/// The bytes of plain `repr(C)` values (every type above is made of numbers only).
pub fn bytes_of<T: Copy>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn read<T: Copy + Default>(p: &[u8], at: usize) -> Option<T> {
    if at + size_of::<T>() > p.len() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(p.as_ptr().add(at) as *const T) })
}

fn read_vec<T: Copy + Default>(p: &[u8], at: usize, n: usize) -> Option<Vec<T>> {
    let end = at.checked_add(n.checked_mul(size_of::<T>())?)?;
    if end > p.len() {
        return None;
    }
    Some((0..n).map(|i| unsafe { std::ptr::read_unaligned(p.as_ptr().add(at + i * size_of::<T>()) as *const T) }).collect())
}

/// The pixel bytes a texture of this size and format must have (one mip level).
pub fn texture_bytes(width: u32, height: u32, format: u32) -> Option<u64> {
    let (bw, bh) = ((width as u64).div_ceil(4), (height as u64).div_ceil(4));
    match format {
        TEX_BC1 => Some(bw * bh * 8),
        TEX_BC2 | TEX_BC3 => Some(bw * bh * 16),
        TEX_RGBA8 => Some(width as u64 * height as u64 * 4),
        _ => None,
    }
}

// ---- Messages: built by the writer, checked and taken apart by the reader ----

pub fn encode_reset(r: &Reset) -> Vec<u8> {
    bytes_of(std::slice::from_ref(r)).to_vec()
}

pub fn encode_texture(t: &Texture, pixels: &[u8]) -> Vec<u8> {
    let mut v = bytes_of(std::slice::from_ref(t)).to_vec();
    v.extend_from_slice(pixels);
    v
}

pub fn encode_model(id: u32, bone_count: u32, parts: &[Part], verts: &[Vertex], indices: &[u32]) -> Vec<u8> {
    let m = Model {
        id,
        vertex_count: verts.len() as u32,
        index_count: indices.len() as u32,
        part_count: parts.len() as u32,
        bone_count,
        flags: 0,
    };
    let mut v = bytes_of(std::slice::from_ref(&m)).to_vec();
    v.extend_from_slice(bytes_of(parts));
    v.extend_from_slice(bytes_of(verts));
    v.extend_from_slice(bytes_of(indices));
    v
}

pub fn encode_forget(f: &Forget) -> Vec<u8> {
    bytes_of(std::slice::from_ref(f)).to_vec()
}

pub fn encode_frame(f: &Frame, objects: &[Object], matrices: &[Matrix]) -> Vec<u8> {
    let f = Frame { object_count: objects.len() as u32, matrix_count: matrices.len() as u32, ..*f };
    let mut v = bytes_of(std::slice::from_ref(&f)).to_vec();
    v.extend_from_slice(bytes_of(objects));
    v.extend_from_slice(bytes_of(matrices));
    v
}

pub fn decode_texture(p: &[u8]) -> Result<(Texture, &[u8]), String> {
    let t: Texture = read(p, 0).ok_or("texture header cut short")?;
    let want = texture_bytes(t.width, t.height, t.format).ok_or(format!("texture {} has unknown format {}", t.id, t.format))?;
    if t.width == 0 || t.height == 0 || t.bytes as u64 != want || p.len() != size_of::<Texture>() + t.bytes as usize {
        return Err(format!("texture {} is {}x{} format {} with {} bytes (needs {want}, message has {})",
                           t.id, t.width, t.height, t.format, t.bytes, p.len() - size_of::<Texture>()));
    }
    Ok((t, &p[size_of::<Texture>()..]))
}

#[derive(Debug)]
pub struct ModelData {
    pub model: Model,
    pub parts: Vec<Part>,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Checks everything Skyrim's C++ reader checks: sizes add up, parts inside the indices, indices
/// inside the vertices, bone numbers inside the bones.
pub fn decode_model(p: &[u8]) -> Result<ModelData, String> {
    let m: Model = read(p, 0).ok_or("model header cut short")?;
    let (np, nv, ni) = (m.part_count as usize, m.vertex_count as usize, m.index_count as usize);
    let at_v = size_of::<Model>() + np * size_of::<Part>();
    let at_i = at_v + nv * size_of::<Vertex>();
    if p.len() != at_i + ni * 4 {
        return Err(format!("model {}: {} bytes, header says {}", m.id, p.len(), at_i + ni * 4));
    }
    let parts: Vec<Part> = read_vec(p, size_of::<Model>(), np).unwrap();
    let vertices: Vec<Vertex> = read_vec(p, at_v, nv).unwrap();
    let indices: Vec<u32> = read_vec(p, at_i, ni).unwrap();
    if ni % 3 != 0 {
        return Err(format!("model {}: {ni} indices, not whole triangles", m.id));
    }
    for (k, part) in parts.iter().enumerate() {
        if part.index_count % 3 != 0 || part.first_index as u64 + part.index_count as u64 > ni as u64 {
            return Err(format!("model {}: part {k} (indices {}+{}) outside {ni}", m.id, part.first_index, part.index_count));
        }
    }
    if let Some(i) = indices.iter().find(|&&i| i as usize >= nv) {
        return Err(format!("model {}: index {i} past {nv} vertices", m.id));
    }
    for v in &vertices {
        for k in 0..4 {
            if v.weights[k] != 0 && v.bones[k] as u32 >= m.bone_count {
                return Err(format!("model {}: bone {} past {} bones", m.id, v.bones[k], m.bone_count));
            }
        }
    }
    Ok(ModelData { model: m, parts, vertices, indices })
}

pub fn decode_frame(p: &[u8]) -> Result<(Frame, Vec<Object>, Vec<Matrix>), String> {
    let f: Frame = read(p, 0).ok_or("frame header cut short")?;
    let (no, nm) = (f.object_count as usize, f.matrix_count as usize);
    let at_m = size_of::<Frame>() + no * size_of::<Object>();
    if p.len() != at_m + nm * size_of::<Matrix>() {
        return Err(format!("frame {}: {} bytes, header says {}", f.frame, p.len(), at_m + nm * size_of::<Matrix>()));
    }
    let objects: Vec<Object> = read_vec(p, size_of::<Frame>(), no).unwrap();
    let matrices: Vec<Matrix> = read_vec(p, at_m, nm).unwrap();
    for (k, o) in objects.iter().enumerate() {
        if o.matrix_count == 0 || o.first_matrix as u64 + o.matrix_count as u64 > nm as u64 {
            return Err(format!("frame {}: object {k} matrices {}+{} outside {nm}", f.frame, o.first_matrix, o.matrix_count));
        }
    }
    Ok((f, objects, matrices))
}

/// The render block, mapped.
pub struct RenderMapping {
    handle: isize,
    base: *mut u8,
}

// Safety: the view stays mapped for the lifetime of RenderMapping; the counters are atomics, and
// ring bytes are written only ahead of head (writer) and read only behind it (reader).
unsafe impl Send for RenderMapping {}
unsafe impl Sync for RenderMapping {}

impl RenderMapping {
    pub fn open() -> Result<RenderMapping, String> {
        Self::open_named(SHM_NAME)
    }

    /// Creates the block if it doesn't exist yet, otherwise opens it; the first side to arrive
    /// fills in the header (as the overlay block does).
    pub fn open_named(name: &str) -> Result<RenderMapping, String> {
        let (handle, base) = map_named(name, MAP_SIZE)?;
        let m = RenderMapping { handle, base };
        let c = m.ctl();
        if c.magic.compare_exchange(0, MAGIC_INIT, AcqRel, Acquire).is_ok() {
            c.version.store(VERSION, Relaxed);
            c.map_size.store(MAP_SIZE as u64, Relaxed);
            c.magic.store(MAGIC, Release);
        } else {
            let start = crate::shm::now_ms();
            while c.magic.load(Acquire) != MAGIC {
                if crate::shm::now_ms() - start > 1000 {
                    return Err(format!("render block never initialized (magic {:#x})", c.magic.load(Acquire)));
                }
                std::thread::yield_now();
            }
        }
        let (v, size) = (c.version.load(Acquire), c.map_size.load(Acquire));
        if v != VERSION || size != MAP_SIZE as u64 {
            return Err(format!("render block mismatch: version {v} size {size}, this build {VERSION} / {MAP_SIZE}"));
        }
        Ok(m)
    }

    pub fn ctl(&self) -> &Ctl {
        unsafe { &*(self.base as *const Ctl) }
    }

    fn counter(&self, off: usize) -> &AtomicU64 {
        unsafe { &*(self.base.add(off) as *const AtomicU64) }
    }

    /// Bytes written but not read yet.
    pub fn pending(&self) -> u64 {
        self.counter(HEAD_OFF).load(Acquire).saturating_sub(self.counter(TAIL_OFF).load(Acquire))
    }

    /// Writer: one message. False if it doesn't fit now (ring too full; counted) or ever (over
    /// half the ring).
    pub fn write(&self, kind: u32, payload: &[u8]) -> bool {
        let size = RING_BYTES as u64;
        let msg = (8 + payload.len() as u64 + 7) & !7;
        let head_ref = self.counter(HEAD_OFF);
        let mut head = head_ref.load(Relaxed);
        let tail = self.counter(TAIL_OFF).load(Acquire);
        let mut pos = head % size;
        let pad = if pos + msg > size { size - pos } else { 0 };
        if msg > size / 2 || size - (head - tail) < msg + pad {
            self.ctl().writes_refused.fetch_add(1, Relaxed);
            return false;
        }
        let data = unsafe { self.base.add(DATA_OFF) };
        unsafe {
            if pad > 0 {
                std::ptr::write_unaligned(data.add(pos as usize) as *mut ColMsgHeader, ColMsgHeader { kind: MSG_PAD, bytes: 0 });
                head += pad;
                pos = 0;
            }
            std::ptr::write_unaligned(data.add(pos as usize) as *mut ColMsgHeader, ColMsgHeader { kind, bytes: payload.len() as u32 });
            std::ptr::copy_nonoverlapping(payload.as_ptr(), data.add(pos as usize + 8), payload.len());
        }
        head_ref.store(head + msg, Release);
        self.ctl().messages_written.fetch_add(1, Relaxed);
        true
    }

    /// Reader (Skyrim; tests and tools here): calls `f(type, payload)` per message, oldest first,
    /// until the ring is empty or about `max_bytes` were read. Something that isn't a message
    /// skips everything pending and returns an error (the C++ reader does the same).
    pub fn read(&self, max_bytes: u64, mut f: impl FnMut(u32, &[u8])) -> Result<u64, String> {
        let size = RING_BYTES as u64;
        let tail_ref = self.counter(TAIL_OFF);
        let head = self.counter(HEAD_OFF).load(Acquire);
        let start = tail_ref.load(Relaxed);
        let mut tail = start;
        let data = unsafe { self.base.add(DATA_OFF) };
        while tail < head && tail - start < max_bytes {
            let pos = tail % size;
            let h: ColMsgHeader = unsafe { std::ptr::read_unaligned(data.add(pos as usize) as *const ColMsgHeader) };
            if h.kind == MSG_PAD {
                tail += size - pos;
                continue;
            }
            let msg = (8 + h.bytes as u64 + 7) & !7;
            if pos + msg > size || tail + msg > head {
                tail_ref.store(head, Release);
                return Err(format!("bad message in the render ring (type {}, {} bytes at {}); skipped {} bytes", h.kind, h.bytes, pos, head - start));
            }
            f(h.kind, unsafe { std::slice::from_raw_parts(data.add(pos as usize + 8), h.bytes as usize) });
            self.ctl().messages_read.fetch_add(1, Relaxed);
            tail += msg;
        }
        tail_ref.store(tail, Release);
        Ok(tail - start)
    }

    /// Reader: a Skyrim arriving skips whatever is unread (it can't use half a stream) and asks
    /// the writer to start again.
    pub fn attach_reader(&self) -> u32 {
        self.counter(TAIL_OFF).store(self.counter(HEAD_OFF).load(Acquire), Release);
        self.ctl().reader_epoch.fetch_add(1, AcqRel) + 1
    }
}

impl Drop for RenderMapping {
    fn drop(&mut self) {
        unmap(self.handle, self.base);
    }
}

/// Garry's Mod's side of the stream: sends RESET at the start and whenever a new Skyrim attached;
/// the caller then sends every model and texture again.
pub struct Writer {
    pub map: RenderMapping,
    session: u32,
    reader_epoch: Option<u32>,
}

impl Writer {
    pub fn new(map: RenderMapping, session: u32) -> Writer {
        map.ctl().writer_session.store(session, Release);
        Writer { map, session, reader_epoch: None }
    }

    /// Call before writing each frame. True = a RESET went out: everything Skyrim held is gone,
    /// resend it all. (If the RESET doesn't fit, it is tried again next call.)
    pub fn begin(&mut self) -> bool {
        let epoch = self.map.ctl().reader_epoch.load(Acquire);
        if self.reader_epoch == Some(epoch) {
            return false;
        }
        if self.map.write(MSG_RESET, &encode_reset(&Reset { session: self.session, reader_epoch: epoch })) {
            self.reader_epoch = Some(epoch);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_name(tag: &str) -> String {
        format!(r"Local\GmodSkyrim_render_test_{}_{}", std::process::id(), tag)
    }

    fn cube_model(id: u32) -> Vec<u8> {
        let parts = [Part { first_index: 0, index_count: 6, texture_id: 7, flags: MAT_ALPHATEST, alpha_ref: 0.5, normal_texture_id: 0, phong_exponent: 0.0, phong_boost: 0.0 }];
        let vertex = |x: f32, b: u8| Vertex { pos: [x, 1.0, 2.0], normal: [0.0, 0.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0], u: 0.25, v: 0.75, bones: [b, 0, 0, 0], weights: [255, 0, 0, 0] };
        let verts = [vertex(0.0, 0), vertex(1.0, 1), vertex(2.0, 1), vertex(3.0, 0)];
        encode_model(id, 2, &parts, &verts, &[0, 1, 2, 0, 2, 3])
    }

    /// Every message comes back exactly as built, and the checks catch broken ones.
    #[test]
    fn messages_round_trip_and_checks() {
        let m = decode_model(&cube_model(5)).unwrap();
        assert_eq!(m.model.id, 5);
        assert_eq!((m.model.vertex_count, m.model.index_count, m.model.part_count, m.model.bone_count), (4, 6, 1, 2));
        assert_eq!(m.parts[0].texture_id, 7);
        assert_eq!(m.vertices[2].pos, [2.0, 1.0, 2.0]);
        assert_eq!(m.vertices[2].bones[0], 1);
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3]);

        let mut bad = cube_model(5);
        let last = bad.len() - 4;
        bad[last..].copy_from_slice(&9u32.to_le_bytes()); // index past the vertices
        assert!(decode_model(&bad).unwrap_err().contains("past 4 vertices"));
        assert!(decode_model(&cube_model(5)[..30]).is_err());
        let mut bone = cube_model(5);
        bone[size_of::<Model>() + size_of::<Part>() + 48] = 2; // vertex 0 on bone 2 of 2
        assert!(decode_model(&bone).unwrap_err().contains("bone 2"));

        let px: Vec<u8> = (0..(8 * 4 / 2) as u8).collect(); // 8x4 BC1 = 2 blocks = 16 bytes
        let t = Texture { id: 3, width: 8, height: 4, format: TEX_BC1, bytes: 16, flags: 0 };
        let msg = encode_texture(&t, &px);
        let (t2, p2) = decode_texture(&msg).unwrap();
        assert_eq!((t2, p2), (t, &px[..]));
        assert!(decode_texture(&encode_texture(&Texture { bytes: 15, ..t }, &px[..15])).is_err());
        assert_eq!(texture_bytes(5, 5, TEX_BC3), Some(4 * 16));
        assert_eq!(texture_bytes(5, 5, TEX_RGBA8), Some(100));

        let objects = [Object { model_id: 5, first_matrix: 0, matrix_count: 2, color: 0xFF00FF80, flags: 0, entity: 0 }];
        let mut mat = Matrix::default();
        mat.m[0][3] = 12.5;
        let f = Frame { frame: 99, qpc: -4, origin_epoch: 3, ..Default::default() };
        let (f2, o2, m2) = decode_frame(&encode_frame(&f, &objects, &[mat, Matrix::default()])).unwrap();
        assert_eq!((f2.frame, f2.qpc, f2.origin_epoch, f2.object_count, f2.matrix_count), (99, -4, 3, 1, 2));
        assert_eq!(o2[0], objects[0]);
        assert_eq!(m2[0].m[0][3], 12.5);
        assert!(decode_frame(&encode_frame(&f, &objects, &[mat])).is_err(), "object needs 2 matrices");
    }

    /// Messages come out in order and whole across the wrap-around; a full ring refuses.
    #[test]
    fn ring_wraps_and_fills() {
        let name = unique_name("ring");
        let w = RenderMapping::open_named(&name).unwrap();
        let r = RenderMapping::open_named(&name).unwrap();
        let big = |n: u32| -> Vec<u8> { let mut v = vec![(n % 251) as u8; 3_000_011]; v[0..4].copy_from_slice(&n.to_le_bytes()); v };
        let (mut next_w, mut next_r, mut refused) = (0u32, 0u32, 0);
        for round in 0..6 {
            loop {
                if w.write(MSG_TEXTURE, &big(next_w)) { next_w += 1 } else { refused += 1; break }
            }
            let max = if round % 2 == 0 { 20_000_000 } else { u64::MAX };
            let mut got = 0;
            r.read(max, |kind, p| {
                assert_eq!(kind, MSG_TEXTURE);
                assert_eq!(p.len(), 3_000_011);
                let n = u32::from_le_bytes(p[0..4].try_into().unwrap());
                assert_eq!(n, next_r, "messages in order");
                assert!(p[4..].iter().all(|&b| b == (n % 251) as u8), "payload intact");
                next_r += 1;
                got += 1;
            }).unwrap();
            assert!(got > 0);
        }
        r.read(u64::MAX, |_, _| next_r += 1).unwrap();
        assert_eq!(next_r, next_w);
        assert_eq!(r.pending(), 0);
        assert!(refused == 6 && next_w > 60, "ring filled each round ({next_w} written)");
        assert!(!w.write(MSG_TEXTURE, &vec![0u8; RING_BYTES / 2]), "over half the ring");
        assert_eq!(w.ctl().writes_refused.load(Relaxed), 7);
    }

    /// The case the fill rule exists for: there is room for the message, but not for the message
    /// plus the skipped end of the ring, so writing it at the start would overwrite unread data.
    #[test]
    fn wrap_never_overwrites_unread() {
        let name = unique_name("overwrite");
        let w = RenderMapping::open_named(&name).unwrap();
        let r = RenderMapping::open_named(&name).unwrap();
        const MB: usize = 1 << 20;
        assert!(w.write(MSG_TEXTURE, &vec![1u8; 30 * MB - 8]));
        assert!(w.write(MSG_TEXTURE, &vec![2u8; 30 * MB - 8])); // head at 60 MB
        let mut first = 0;
        r.read(1, |_, p| first = p[0]).unwrap(); // only A: tail at 30 MB
        assert_eq!(first, 1);
        // 34 MB free, but C needs 31 MB at the start plus the 4 MB end it skips.
        assert!(!w.write(MSG_TEXTURE, &vec![3u8; 31 * MB - 8]));
        let mut b_ok = false;
        r.read(u64::MAX, |_, p| b_ok = p.iter().all(|&b| b == 2)).unwrap();
        assert!(b_ok, "B intact");
    }

    /// A Skyrim that attaches late skips the old stream and gets a RESET and everything again;
    /// so does a second Skyrim after a restart.
    #[test]
    fn late_reader_gets_a_reset() {
        let name = unique_name("late");
        let mut w = Writer::new(RenderMapping::open_named(&name).unwrap(), 1234);
        assert!(w.begin(), "first stream starts with RESET");
        assert!(w.map.write(MSG_MODEL, &cube_model(1)));
        assert!(!w.begin(), "no reader change, no RESET");

        let r = RenderMapping::open_named(&name).unwrap();
        assert_eq!(r.attach_reader(), 1);
        assert_eq!(r.pending(), 0, "the half stream before the reader came is skipped");
        assert!(w.begin(), "new reader: RESET");
        assert!(w.map.write(MSG_MODEL, &cube_model(1)));
        let mut seen = vec![];
        r.read(u64::MAX, |kind, p| {
            if kind == MSG_RESET {
                let rs: Reset = read(p, 0).unwrap();
                assert_eq!((rs.session, rs.reader_epoch), (1234, 1));
            }
            seen.push(kind);
        }).unwrap();
        assert_eq!(seen, vec![MSG_RESET, MSG_MODEL]);

        drop(r);
        let r2 = RenderMapping::open_named(&name).unwrap();
        assert_eq!(r2.attach_reader(), 2);
        assert!(w.begin());
        let mut kinds = vec![];
        r2.read(u64::MAX, |kind, _| kinds.push(kind)).unwrap();
        assert_eq!(kinds, vec![MSG_RESET]);
    }
}
