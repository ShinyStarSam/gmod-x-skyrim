//! Pretends to be the Skyrim plugin: publishes an origin and a standing Skyrim player, and logs
//! what Garry's Mod writes. Usage: fake_skyrim [--seconds N]   (default 600)
//! Log: logs/fake_skyrim.log

mod common;

use common::{seconds_arg, Log};
use gmsky_protocol::shm::{now_ms, Mapping, Side};
use gmsky_protocol::{Origin, SkyrimState};
use std::time::Duration;

fn main() {
    let mut log = Log::new("fake_skyrim");
    let m = match Mapping::open(Side::Skyrim) {
        Ok(m) => m,
        Err(e) => {
            log.line(&format!("ERROR opening shared memory: {e}"));
            std::process::exit(1);
        }
    };
    let s = m.shared();
    log.line("fake_skyrim: shared memory open, publishing origin epoch 1");

    // Roughly outside Whiterun's gate. Any numbers work for a fake.
    s.origin.write(&Origin { epoch: 1, worldspace_form_id: 0x3C, skyrim_origin: [19000.0, -7000.0, -3300.0] });

    let end = now_ms() + seconds_arg(600) * 1000;
    let mut frame = 0u64;
    let mut last_print = 0u64;
    let mut last_seq = 0u32;
    let mut was_alive = false;
    let mut updates_since_print = 0u32;
    let mut lost = false;
    while now_ms() < end {
        frame += 1;
        if !m.owns_side() {
            if !lost {
                log.line("another Skyrim-side process took over the link; pausing until it exits");
                lost = true;
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        lost = false;
        m.heartbeat();
        s.skyrim_state.write(&SkyrimState {
            frame, pos: [19000.0, -7000.0, -3300.0], height: 128.0, cell_form_id: 0, ..Default::default()
        });

        let alive = m.peer_alive();
        if alive != was_alive {
            log.line(if alive { "gmod CONNECTED (heartbeat fresh)" } else { "gmod DISCONNECTED (no heartbeat)" });
            was_alive = alive;
        }
        if let Some((_, seq)) = s.player_state.read() {
            if seq != last_seq {
                updates_since_print += 1;
                last_seq = seq;
            }
        }
        if now_ms() - last_print >= 500 {
            last_print = now_ms();
            match s.player_state.read() {
                Some((p, _)) => log.line(&format!(
                    "gmod tick={} epoch={} pos=({:.1}, {:.1}, {:.1}) anchor=({:.1}, {:.1}, {:.1}) \
                     yaw={:.1} pitch={:.1} eye={:.1} flags={:#x} speed={:.1} updates/0.5s={}",
                    p.tick, p.origin_epoch, p.pos[0], p.pos[1], p.pos[2], p.anchor[0], p.anchor[1], p.anchor[2],
                    p.yaw_deg, p.pitch_deg, p.eye_height, p.flags,
                    (p.vel[0] * p.vel[0] + p.vel[1] * p.vel[1]).sqrt(), updates_since_print)),
                None => log.line("gmod has not written a player state yet"),
            }
            updates_since_print = 0;
        }
        std::thread::sleep(Duration::from_millis(16)); // ~60 fps like a game frame
    }
    log.line("fake_skyrim: done");
}
