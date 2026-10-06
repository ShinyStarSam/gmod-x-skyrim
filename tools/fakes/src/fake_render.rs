//! Pretends to be Garry's Mod's render writer (geometry route, step G2): sends a test texture and
//! a test model into the render block, then 60 frames a second where the model spins and bends,
//! so Skyrim's reader (and later its drawing, G3) can be tested without Garry's Mod.
//! The model: a bar 64 units long, 8 thick, in two halves on two bones (bone 0: x < 0, bone 1:
//! x > 0), textured with a 64 x 64 orange / white checker (RGBA8). The object sits at (150, 0, 40)
//! relative to the anchor, turns 90 degrees a second, and its second half bends up and down.
//! Positions are relative to the anchor of the origin epoch in the frames: by default the epoch
//! Skyrim published last (read every second), so with no Garry's Mod the anchor is Skyrim's origin.
//! Usage: fake_render [--seconds N] [--epoch N]   (defaults 600, the current origin epoch)
//! Log: logs/fake_render.log

mod common;

use common::{seconds_arg, Log};
use gmsky_protocol::render::*;
use gmsky_protocol::shm::{now_ms, now_qpc};
use std::time::Duration;

fn epoch_arg() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == "--epoch").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok())
}

/// The origin epoch Skyrim published last (0 if none yet).
fn current_epoch(log: &mut Log, last: &mut u32) -> u32 {
    let e = match gmsky_protocol::shm::peek_origin() {
        Ok(o) => o.map(|o| o.epoch).unwrap_or(0),
        Err(e) => {
            log.line(&format!("fake_render: can't read the origin: {e}"));
            0
        }
    };
    if e != *last {
        log.line(&format!("fake_render: origin epoch now {e}"));
        *last = e;
    }
    e
}

/// A box from `lo` to `hi` on one bone: 6 faces, 4 corners each (own normals and texture corners).
fn push_box(lo: [f32; 3], hi: [f32; 3], bone: u8, verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    for axis in 0..3 {
        for side in [0usize, 1] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut normal = [0.0f32; 3];
            normal[axis] = if side == 1 { 1.0 } else { -1.0 };
            let base = verts.len() as u32;
            for (cu, cv) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                let mut p = [0.0f32; 3];
                p[axis] = if side == 1 { hi[axis] } else { lo[axis] };
                p[u] = if cu == 1 { hi[u] } else { lo[u] };
                p[v] = if cv == 1 { hi[v] } else { lo[v] };
                verts.push(Vertex { pos: p, normal, u: cu as f32, v: cv as f32, bones: [bone, 0, 0, 0], weights: [255, 0, 0, 0] });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

fn checker() -> Vec<u8> {
    let mut px = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            px.extend_from_slice(if (x / 8 + y / 8) % 2 == 0 { &[255, 140, 0, 255] } else { &[255, 255, 255, 255] });
        }
    }
    px
}

/// Rotation about Z by `yaw`, then about the bar's own Y by `bend` (radians), then moved to `t`.
fn matrix(yaw: f32, bend: f32, t: [f32; 3]) -> Matrix {
    let (sy, cy) = yaw.sin_cos();
    let (sb, cb) = bend.sin_cos();
    // Rz(yaw) * Ry(bend)
    let r = [
        [cy * cb, -sy, cy * sb],
        [sy * cb, cy, sy * sb],
        [-sb, 0.0, cb],
    ];
    Matrix { m: [[r[0][0], r[0][1], r[0][2], t[0]], [r[1][0], r[1][1], r[1][2], t[1]], [r[2][0], r[2][1], r[2][2], t[2]]] }
}

fn main() {
    let mut log = Log::new("fake_render");
    let map = match RenderMapping::open() {
        Ok(m) => m,
        Err(e) => {
            log.line(&format!("ERROR opening the render block: {e}"));
            std::process::exit(1);
        }
    };
    let mut w = Writer::new(map, std::process::id());
    let fixed_epoch = epoch_arg();
    let mut seen_epoch = u32::MAX;
    let mut epoch = fixed_epoch.unwrap_or_else(|| current_epoch(&mut log, &mut seen_epoch));
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    push_box([-32.0, -4.0, -4.0], [0.0, 4.0, 4.0], 0, &mut verts, &mut indices);
    push_box([0.0, -4.0, -4.0], [32.0, 4.0, 4.0], 1, &mut verts, &mut indices);
    let parts = [Part { first_index: 0, index_count: indices.len() as u32, texture_id: 1, flags: 0, alpha_ref: 0.0, reserved: 0 }];
    let model = encode_model(1, 2, &parts, &verts, &indices);
    let px = checker();
    let texture = encode_texture(&Texture { id: 1, width: 64, height: 64, format: TEX_RGBA8, bytes: px.len() as u32, flags: 0 }, &px);
    log.line(&format!("fake_render: block open, session {}, origin epoch {epoch}; model {} bytes ({} vertices, {} triangles), texture {} bytes",
                      std::process::id(), model.len(), verts.len(), indices.len() / 3, texture.len()));

    let start = now_ms();
    let end = start + seconds_arg(600) * 1000;
    let (mut frame, mut last_print) = (0u64, start);
    let (mut sent_frames, mut dropped_frames, mut resets) = (0u64, 0u64, 0u64);
    let mut need_assets = true;
    while now_ms() < end {
        if w.begin() {
            resets += 1;
            need_assets = true;
            log.line(&format!("fake_render: RESET sent (reader epoch {})", w.map.ctl().reader_epoch.load(std::sync::atomic::Ordering::Acquire)));
        }
        if need_assets && w.map.write(MSG_TEXTURE, &texture) && w.map.write(MSG_MODEL, &model) {
            need_assets = false;
            log.line("fake_render: texture 1 and model 1 sent");
        }
        frame += 1;
        let t = (now_ms() - start) as f32 / 1000.0;
        let yaw = t * std::f32::consts::FRAC_PI_2;
        let bend = (t * 2.0).sin() * 0.6;
        let at = [150.0, 0.0, 40.0];
        let m0 = matrix(yaw, 0.0, at);
        let m1 = matrix(yaw, bend, at);
        let objects = [Object { model_id: 1, first_matrix: 0, matrix_count: 2, color: 0xFFFF_FFFF, flags: 0, reserved: 0 }];
        let msg = encode_frame(&Frame { frame, qpc: now_qpc(), origin_epoch: epoch, ..Default::default() }, &objects, &[m0, m1]);
        if !need_assets && w.map.write(MSG_FRAME, &msg) { sent_frames += 1 } else { dropped_frames += 1 }
        if now_ms() - last_print >= 1000 {
            last_print = now_ms();
            if fixed_epoch.is_none() {
                epoch = current_epoch(&mut log, &mut seen_epoch);
            }
            let c = w.map.ctl();
            log.line(&format!("fake_render: frame {frame}: {sent_frames} sent, {dropped_frames} not sent, {resets} resets; ring {} bytes unread, {} written / {} read / {} refused in all",
                              w.map.pending(), c.messages_written.load(std::sync::atomic::Ordering::Relaxed),
                              c.messages_read.load(std::sync::atomic::Ordering::Relaxed), c.writes_refused.load(std::sync::atomic::Ordering::Relaxed)));
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    log.line(&format!("fake_render: done, {sent_frames} frames sent, {dropped_frames} not sent, {resets} resets"));
}
