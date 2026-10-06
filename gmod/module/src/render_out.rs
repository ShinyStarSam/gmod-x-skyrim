//! Garry's Mod's side of the render block (geometry route step G3b, DESIGN.md §12): models,
//! textures and every frame's bone matrices, from client Lua (`render.lua`) to Skyrim, which draws
//! them inside its own frame. Owns the ring writer; keeps every model and texture message it sent
//! so that a Skyrim that starts later (RESET) gets them all again before the frames.

use crate::vtf;
use gmsky_protocol::render::*;
use gmsky_protocol::shm::now_qpc;
use std::collections::{HashMap, VecDeque};

/// Biggest texture side sent (the VTF's biggest mip level no larger than this).
pub const MAX_TEXTURE: u32 = 1024;
/// Numbers per corner in RenderModel's flat table: position 3, normal 3, u, v, 4 x (bone, weight).
pub const VERTEX_NUMBERS: usize = 16;
/// Numbers per part: first corner, corner count, texture id, GMSKY_RND_MAT_* flags, alpha reference.
pub const PART_NUMBERS: usize = 5;

pub type M34 = [[f32; 4]; 3];

/// a x b for two [rotation | translation] matrices.
pub fn compose(a: &M34, b: &M34) -> M34 {
    let mut out = [[0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..4 {
            let mut v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            if c == 3 {
                v += a[r][3];
            }
            out[r][c] = v;
        }
    }
    out
}

/// One corner from 16 numbers. Up to 4 (bone, weight) pairs become 0-255 weights summing to 255
/// (rounding left-over goes to the heaviest); bones past `bone_count` and a corner without any
/// weight go on bone 0.
pub fn vertex_from(n: &[f64], bone_count: u32) -> Vertex {
    let mut pairs: Vec<(u8, f64)> = (0..4)
        .map(|k| (n[8 + 2 * k], n[9 + 2 * k]))
        .filter(|&(b, w)| w > 0.0 && b >= 0.0 && (b as u32) < bone_count)
        .map(|(b, w)| (b as u8, w))
        .collect();
    if pairs.is_empty() {
        pairs.push((0, 1.0));
    }
    let total: f64 = pairs.iter().map(|p| p.1).sum();
    let mut bones = [0u8; 4];
    let mut weights = [0u8; 4];
    let mut sum = 0u32;
    for (k, &(b, w)) in pairs.iter().enumerate() {
        bones[k] = b;
        weights[k] = (w / total * 255.0).round().clamp(0.0, 255.0) as u8;
        sum += weights[k] as u32;
    }
    let heaviest = (0..pairs.len()).max_by_key(|&k| weights[k]).unwrap();
    weights[heaviest] = (weights[heaviest] as i32 + 255 - sum as i32).clamp(0, 255) as u8;
    Vertex {
        pos: [n[0] as f32, n[1] as f32, n[2] as f32],
        normal: [n[3] as f32, n[4] as f32, n[5] as f32],
        u: n[6] as f32,
        v: n[7] as f32,
        bones,
        weights,
    }
}

fn m34(n: &[f64]) -> M34 {
    [
        [n[0] as f32, n[1] as f32, n[2] as f32, n[3] as f32],
        [n[4] as f32, n[5] as f32, n[6] as f32, n[7] as f32],
        [n[8] as f32, n[9] as f32, n[10] as f32, n[11] as f32],
    ]
}

#[derive(Default)]
pub struct Counts {
    pub frames: u64,
    pub frames_refused: u64,
    pub frames_no_epoch: u64,
    pub objects: u64,
    pub objects_unknown: u64,
    pub resets: u64,
    pub models_sent: u64,
    pub textures_sent: u64,
    pub bytes: u64,
}

pub struct RenderOut {
    pub writer: Writer,
    models: HashMap<u32, (Vec<u8>, Vec<M34>)>,  // message, poseToBone per bone
    textures: HashMap<u32, Vec<u8>>,
    pending: VecDeque<(u32, u32)>,  // (MSG_TEXTURE / MSG_MODEL, id) still to send
    pub counts: Counts,
}

impl RenderOut {
    pub fn new(map: RenderMapping, session: u32) -> RenderOut {
        RenderOut { writer: Writer::new(map, session), models: HashMap::new(), textures: HashMap::new(), pending: VecDeque::new(), counts: Counts::default() }
    }

    /// RESET when a new Skyrim attached (then everything again), and whatever is still waiting.
    fn service(&mut self) {
        if self.writer.begin() {
            self.counts.resets += 1;
            self.pending.clear();
            self.pending.extend(self.textures.keys().map(|&id| (MSG_TEXTURE, id)));
            self.pending.extend(self.models.keys().map(|&id| (MSG_MODEL, id)));
        }
        while let Some(&(kind, id)) = self.pending.front() {
            let msg = match kind {
                MSG_TEXTURE => self.textures.get(&id),
                _ => self.models.get(&id).map(|m| &m.0),
            };
            let Some(msg) = msg else {
                self.pending.pop_front();
                continue;
            };
            if !self.writer.map.write(kind, msg) {
                break; // ring full: try again next call
            }
            self.counts.bytes += msg.len() as u64;
            if kind == MSG_TEXTURE { self.counts.textures_sent += 1 } else { self.counts.models_sent += 1 }
            self.pending.pop_front();
        }
    }

    /// A texture from a VTF file. Returns a line for the log.
    pub fn add_texture(&mut self, id: u32, file: &[u8]) -> Result<String, String> {
        let img = vtf::decode(file, MAX_TEXTURE)?;
        let msg = encode_texture(&Texture { id, width: img.width, height: img.height, format: TEX_RGBA8, bytes: img.rgba.len() as u32, flags: 0 }, &img.rgba);
        let line = format!("texture {id}: {}x{} (level {} of {}x{}), VTF format {}", img.width, img.height, img.mip, img.full_width, img.full_height, img.format);
        self.textures.insert(id, msg);
        self.pending.push_back((MSG_TEXTURE, id));
        self.service();
        Ok(line)
    }

    /// A model from flat number lists (see VERTEX_NUMBERS, PART_NUMBERS; bind poses 12 per bone).
    /// The corners are a plain triangle list (3 per triangle). Returns a line for the log.
    pub fn add_model(&mut self, id: u32, bone_count: u32, verts: &[f64], parts: &[f64], binds: &[f64]) -> Result<String, String> {
        if bone_count == 0 || bone_count > 255 || binds.len() != bone_count as usize * 12 {
            return Err(format!("model {id}: {bone_count} bones with {} bind numbers", binds.len()));
        }
        if verts.is_empty() || verts.len() % VERTEX_NUMBERS != 0 || (verts.len() / VERTEX_NUMBERS) % 3 != 0 {
            return Err(format!("model {id}: {} vertex numbers, not whole triangles", verts.len()));
        }
        let vertices: Vec<Vertex> = verts.chunks(VERTEX_NUMBERS).map(|n| vertex_from(n, bone_count)).collect();
        let indices: Vec<u32> = (0..vertices.len() as u32).collect();
        let parts: Vec<Part> = parts
            .chunks(PART_NUMBERS)
            .filter(|p| p.len() == PART_NUMBERS)
            .map(|p| Part { first_index: p[0] as u32, index_count: p[1] as u32, texture_id: p[2] as u32, flags: p[3] as u32, alpha_ref: p[4] as f32, reserved: 0 })
            .collect();
        let msg = encode_model(id, bone_count, &parts, &vertices, &indices);
        decode_model(&msg).map_err(|e| format!("model {id} refused before sending: {e}"))?;
        let binds: Vec<M34> = binds.chunks(12).map(m34).collect();
        let line = format!("model {id}: {} triangles, {} parts, {} bones, {} KB", vertices.len() / 3, parts.len(), bone_count, msg.len() / 1024);
        self.models.insert(id, (msg, binds));
        self.pending.push_back((MSG_MODEL, id));
        self.service();
        Ok(line)
    }

    /// One Garry's Mod frame: per object, model id, colour (RGBA8 number), bone count, then 12
    /// numbers per bone (`GetBoneMatrix`, world). Each becomes bone x poseToBone, relative to the
    /// anchor. Returns the objects sent.
    pub fn frame(&mut self, frame: u64, flat: &[f64], epoch: u32, anchor: [f64; 3]) -> u32 {
        self.service();
        if epoch == 0 {
            self.counts.frames_no_epoch += 1;
            return 0;
        }
        let mut objects = Vec::new();
        let mut matrices: Vec<Matrix> = Vec::new();
        let mut at = 0;
        while at + 3 <= flat.len() {
            let (model, color, bones) = (flat[at] as u32, flat[at + 1] as u32, flat[at + 2] as usize);
            at += 3;
            if at + bones * 12 > flat.len() {
                break;
            }
            let worlds = &flat[at..at + bones * 12];
            at += bones * 12;
            let Some((_, binds)) = self.models.get(&model) else {
                self.counts.objects_unknown += 1;
                continue;
            };
            if binds.len() != bones {
                self.counts.objects_unknown += 1;
                continue;
            }
            let first = matrices.len() as u32;
            for (b, bind) in binds.iter().enumerate() {
                // Anchor-relative before multiplying, so the floats stay small.
                let mut w = m34(&worlds[b * 12..b * 12 + 12]);
                for r in 0..3 {
                    w[r][3] = (worlds[b * 12 + r * 4 + 3] - anchor[r]) as f32;
                }
                matrices.push(Matrix { m: compose(&w, bind) });
            }
            objects.push(Object { model_id: model, first_matrix: first, matrix_count: bones as u32, color, flags: 0, reserved: 0 });
        }
        let msg = encode_frame(&Frame { frame, qpc: now_qpc(), origin_epoch: epoch, ..Default::default() }, &objects, &matrices);
        if self.writer.map.write(MSG_FRAME, &msg) {
            self.counts.frames += 1;
            self.counts.objects += objects.len() as u64;
            self.counts.bytes += msg.len() as u64;
        } else {
            self.counts.frames_refused += 1;
        }
        objects.len() as u32
    }

    pub fn status(&mut self) -> String {
        let c = std::mem::take(&mut self.counts);
        format!("render out: {} frames ({} objects, {} unknown models), {} refused, {} without an origin epoch; {} resets; sent {} models, {} textures, {} KB; holding {} models, {} textures, {} waiting; ring {} KB unread",
                c.frames, c.objects, c.objects_unknown, c.frames_refused, c.frames_no_epoch, c.resets, c.models_sent, c.textures_sent, c.bytes / 1024,
                self.models.len(), self.textures.len(), self.pending.len(), self.writer.map.pending() / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(tag: &str) -> String {
        format!(r"Local\GmodSkyrim_render_out_test_{}_{}", std::process::id(), tag)
    }

    fn identity_with(t: [f64; 3]) -> Vec<f64> {
        vec![1.0, 0.0, 0.0, t[0], 0.0, 1.0, 0.0, t[1], 0.0, 0.0, 1.0, t[2]]
    }

    #[test]
    fn weights_sum_to_255() {
        let mut n = vec![0.0; 16];
        n[8..14].copy_from_slice(&[0.0, 0.5, 1.0, 0.3, 2.0, 0.2]);
        let v = vertex_from(&n, 3);
        assert_eq!(v.bones[..3], [0, 1, 2]);
        assert_eq!(v.weights.iter().map(|&w| w as u32).sum::<u32>(), 255);
        assert_eq!(v.weights[..3], [127, 77, 51]); // 128 + 77 + 51 = 256: the heaviest gives one back
        // No weights, or a bone past the model's: bone 0, full weight.
        let v = vertex_from(&[0.0; 16], 1);
        assert_eq!((v.bones[0], v.weights[0]), (0, 255));
        let mut n = vec![0.0; 16];
        n[8..10].copy_from_slice(&[7.0, 1.0]);
        assert_eq!(vertex_from(&n, 2).weights[0], 255);
    }

    #[test]
    fn compose_is_matrix_product() {
        // Rotate 90 degrees about Z, then move by (10, 0, 0); b moves by (1, 2, 3) first.
        let a: M34 = [[0.0, -1.0, 0.0, 10.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        let b: M34 = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 2.0], [0.0, 0.0, 1.0, 3.0]];
        let c = compose(&a, &b);
        // The point 0 -> b -> (1,2,3) -> a -> (-2+10, 1, 3).
        assert_eq!([c[0][3], c[1][3], c[2][3]], [8.0, 1.0, 3.0]);
        assert_eq!(c[0][1], -1.0);
    }

    /// Model, texture and a frame reach the ring; the frame's matrix is bone x poseToBone minus the
    /// anchor; a Skyrim that attaches later gets RESET and everything again.
    #[test]
    fn sends_and_resends() {
        let name = unique("a");
        let mut out = RenderOut::new(RenderMapping::open_named(&name).unwrap(), 77);
        // A 4x4 BGRA texture as a VTF.
        let mut vtf = Vec::new();
        vtf.extend_from_slice(b"VTF\0");
        for v in [7u32, 2, 80] { vtf.extend_from_slice(&v.to_le_bytes()); }
        vtf.extend_from_slice(&4u16.to_le_bytes());
        vtf.extend_from_slice(&4u16.to_le_bytes());
        vtf.extend_from_slice(&[0; 32]); // flags .. bumpmap scale
        vtf.extend_from_slice(&vtf::FMT_BGRA8888.to_le_bytes());
        vtf.push(1);
        vtf.extend_from_slice(&u32::MAX.to_le_bytes());
        vtf.extend_from_slice(&[0, 0, 1, 0, 0, 0, 0]);
        vtf.resize(80, 0);
        vtf.extend(std::iter::repeat([10u8, 20, 30, 255]).take(16).flatten());
        assert!(out.add_texture(5, &vtf).unwrap().contains("4x4"));
        // One triangle on bone 0 of 1; poseToBone moves by (-1, 0, 0).
        let mut verts = Vec::new();
        for x in [0.0, 1.0, 0.0] {
            verts.extend_from_slice(&[x, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
        assert!(out.add_model(9, 1, &verts, &[0.0, 3.0, 5.0, 0.0, 0.0], &identity_with([-1.0, 0.0, 0.0])).is_ok());
        assert!(out.add_model(10, 1, &verts[..16], &[], &identity_with([0.0; 3])).is_err(), "not whole triangles");
        // The bone at (101, 2, 3) in the world, the anchor at (100, 0, 0): (1, 2, 3) - 1 in x.
        let mut flat = vec![9.0, 4294967295.0, 1.0];
        flat.extend(identity_with([101.0, 2.0, 3.0]));
        flat.extend_from_slice(&[42.0, 0.0, 1.0]); // unknown model
        flat.extend(identity_with([0.0; 3]));
        assert_eq!(out.frame(1, &flat, 3, [100.0, 0.0, 0.0]), 1);
        assert_eq!(out.frame(2, &flat, 0, [0.0; 3]), 0, "no epoch, no frame");

        let reader = RenderMapping::open_named(&name).unwrap();
        let mut kinds = vec![];
        reader.read(u64::MAX, |kind, p| {
            kinds.push(kind);
            if kind == MSG_FRAME {
                let (f, o, m) = decode_frame(p).unwrap();
                assert_eq!((f.origin_epoch, o.len(), o[0].model_id, o[0].color), (3, 1, 9, 0xFFFF_FFFF));
                assert_eq!([m[0].m[0][3], m[0].m[1][3], m[0].m[2][3]], [0.0, 2.0, 3.0]);
            }
            if kind == MSG_TEXTURE {
                let (t, px) = decode_texture(p).unwrap();
                assert_eq!((t.id, t.format, &px[0..4]), (5, TEX_RGBA8, &[30u8, 20, 10, 255][..]));
            }
        }).unwrap();
        assert_eq!(kinds, vec![MSG_RESET, MSG_TEXTURE, MSG_MODEL, MSG_FRAME]);

        reader.attach_reader();
        out.frame(3, &flat, 3, [100.0, 0.0, 0.0]);
        let mut again = vec![];
        reader.read(u64::MAX, |kind, _| again.push(kind)).unwrap();
        assert_eq!(again, vec![MSG_RESET, MSG_TEXTURE, MSG_MODEL, MSG_FRAME]);
        assert!(out.status().contains("2 resets"), "first stream + the new reader");
    }
}
