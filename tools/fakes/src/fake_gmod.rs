//! Pretends to be Garry's Mod: walks the player in a circle (radius 200 units, one lap per
//! 10 seconds) and writes PlayerState at 66 ticks per second, like a Garry's Mod server.
//! Usage: fake_gmod [--seconds N]   (default 600)
//! Log: logs/fake_gmod.log

mod common;

use common::{seconds_arg, Log};
use gmsky_protocol::shm::{now_ms, Mapping, Side};
use gmsky_protocol::{PlayerState, PF_ALIVE, PF_ON_GROUND};
use std::time::Duration;

fn main() {
    let mut log = Log::new("fake_gmod");
    let m = match Mapping::open(Side::Gmod) {
        Ok(m) => m,
        Err(e) => {
            log.line(&format!("ERROR opening shared memory: {e}"));
            std::process::exit(1);
        }
    };
    let s = m.shared();
    log.line("fake_gmod: shared memory open, walking a circle");

    let start = now_ms();
    let end = start + seconds_arg(600) * 1000;
    let mut tick = 0u64;
    let mut epoch = 0u32;
    let mut anchor = [0.0f64; 3];
    let mut last_print = 0u64;
    const FLOOR_Z: f64 = -12700.0; // like gm_flatgrass' floor
    while now_ms() < end {
        tick += 1;
        m.heartbeat();
        let t = (now_ms() - start) as f64 / 1000.0;
        let a = t * std::f64::consts::TAU / 10.0;
        let pos = [200.0 * a.cos(), 200.0 * a.sin(), FLOOR_Z];
        let speed = 200.0 * std::f64::consts::TAU / 10.0;
        let vel = [(-speed * a.sin()) as f32, (speed * a.cos()) as f32, 0.0];
        let yaw = (a.to_degrees() + 90.0).rem_euclid(360.0) as f32; // facing along the circle

        // New origin epoch from Skyrim: record where we stand now as the anchor.
        if let Some((o, _)) = s.origin.read() {
            if o.epoch != epoch {
                epoch = o.epoch;
                anchor = pos;
                log.line(&format!("new origin epoch {epoch}: skyrim origin {:?}, anchor {:?}", o.skyrim_origin, anchor));
            }
        }
        s.player_state.write(&PlayerState {
            tick, origin_epoch: epoch, flags: PF_ON_GROUND | PF_ALIVE, anchor, pos,
            eye_height: 64.0, pitch_deg: 0.0, yaw_deg: yaw, roll_deg: 0.0, vel, fov_deg: 75.0,
        });
        if now_ms() - last_print >= 1000 {
            last_print = now_ms();
            let sk = s.skyrim_state.read().map(|(k, _)| format!("frame={} pos={:?}", k.frame, k.pos))
                .unwrap_or_else(|| "none yet".into());
            log.line(&format!("tick={tick} pos=({:.1}, {:.1}, {:.1}) yaw={yaw:.1} skyrim alive={} skyrim: {sk}",
                              pos[0], pos[1], pos[2], m.peer_alive()));
        }
        std::thread::sleep(Duration::from_millis(15)); // ~66 ticks per second
    }
    log.line("fake_gmod: done");
}
