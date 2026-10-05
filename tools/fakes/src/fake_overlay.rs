//! Pretends to be Garry's Mod's picture writer (step 2b-2): publishes a test picture into the
//! overlay picture block 60 times a second, so tools/overlay_read.py (and later Skyrim's drawing,
//! step 2c) can be tested without Garry's Mod.
//! The picture (premultiplied BGRA, like Garry's Mod's back buffer): transparent, with a white
//! square at alpha 255, one at 128 and one at 46 (premultiplied: colour = alpha), a pure "glow"
//! square (colour, alpha 0: adds light), and a 32 x 32 marker that moves one pixel per frame.
//! Usage: fake_overlay [--seconds N] [--size WxH]   (defaults 600, 1280x720)
//! Log: logs/fake_overlay.log

mod common;

use common::{seconds_arg, Log};
use gmsky_protocol::overlay::{OverlayMapping, SlotHdr, FMT_BGRA8, F_PREMULTIPLIED, F_SRGB, MAX_H, MAX_W};
use gmsky_protocol::shm::{now_ms, now_qpc};
use std::time::Duration;

fn size_arg() -> (u32, u32) {
    let args: Vec<String> = std::env::args().collect();
    let parsed = args.iter().position(|a| a == "--size").and_then(|i| args.get(i + 1)).and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    });
    let (w, h) = parsed.unwrap_or((1280, 720));
    (w.clamp(64, MAX_W), h.clamp(64, MAX_H))
}

fn main() {
    let mut log = Log::new("fake_overlay");
    let mut m = match OverlayMapping::open() {
        Ok(m) => m,
        Err(e) => {
            log.line(&format!("ERROR opening the overlay block: {e}"));
            std::process::exit(1);
        }
    };
    let (w, h) = size_arg();
    let slot = m.attach(true);
    log.line(&format!("fake_overlay: block open, writer slot {slot}, publishing {w}x{h} at 60 per second"));
    let start = now_ms();
    let end = start + seconds_arg(600) * 1000;
    let (mut frame, mut last_print) = (0u64, 0u64);
    while now_ms() < end {
        frame += 1;
        let px = unsafe { std::slice::from_raw_parts_mut(m.slot_ptr(m.own_slot()) as *mut [u8; 4], (w * h) as usize) };
        px.fill([0, 0, 0, 0]);
        let mut rect = |x0: u32, y0: u32, size: u32, bgra: [u8; 4]| {
            for y in y0..(y0 + size).min(h) {
                for x in x0..(x0 + size).min(w) {
                    px[(y * w + x) as usize] = bgra;
                }
            }
        };
        rect(40, 40, 80, [255, 255, 255, 255]);
        rect(140, 40, 80, [128, 128, 128, 128]);
        rect(240, 40, 80, [46, 46, 46, 46]);
        rect(340, 40, 80, [255, 200, 0, 0]); // glow: light blue-ish colour, alpha 0
        rect((frame % (w as u64 - 32)) as u32, h / 2, 32, [0, 0, 255, 255]); // red marker
        m.publish(&SlotHdr { width: w, height: h, format: FMT_BGRA8, flags: F_PREMULTIPLIED | F_SRGB,
            frame, capture_qpc: now_qpc(), row_bytes: w * 4, ..Default::default() });
        if now_ms() - last_print >= 1000 {
            last_print = now_ms();
            let c = m.ctl();
            log.line(&format!("frame {frame}, published {}, taken by Skyrim {}, Skyrim's size {:?}, state {:#x}",
                c.frames_published.load(std::sync::atomic::Ordering::Relaxed),
                c.frames_taken.load(std::sync::atomic::Ordering::Relaxed), m.skyrim_size(), m.state()));
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    log.line("fake_overlay: done");
}
