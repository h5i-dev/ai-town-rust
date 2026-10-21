//! Spatial-grid `run_ticks` versus the PTX `seek_one` batch.
//!
//! The CPU number is one product step: up to 60 ticks of path following on the
//! one-tile grid. The GPU number is 60 greedy steps of 12 milli-tiles, which is
//! the same per-tick distance budget (`SPEED * TICK_MS / 1000`) but not the
//! same algorithm. The O(n^2) scalar scan is not timed here.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{
    run_ticks, transition, Command, PathNode, Player, Principal, Snapshot, ENGINE, FACE_BIAS, MILLI, PF_MOVE, TICK_MS,
};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let ptx = std::env::var("AI_TOWN_PTX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/dev/shm/ai-town-rust/nvvm-target/nvptx64-nvidia-cuda/release/ai_town_seek_device.ptx"));
    let dev = RustDevice::open_path(&ptx).unwrap_or_else(|err| panic!("cuda {}: {}", err.op, err.code));
    let dir = std::env::var("AI_TOWN_BENCH_DIR").unwrap_or_else(|_| "/dev/shm/ai-town-rust/bench".into());
    std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("create {dir}: {err}"));
    let mut rows = vec!["agents,cpu_tick_ms,cpu_step_ms,gpu_one_kernel_ms,gpu_one_wall_ms,gpu_60_kernel_ms,gpu_60_wall_ms".to_string()];
    for n in [1_000usize, 10_000, 50_000] {
        let world = world(n);
        let end = world.meta.now_ms + 60 * TICK_MS;
        let mut warmed = world.clone();
        let ticks = run_ticks(&mut warmed, end);
        assert_eq!(ticks, 60, "warmup ticks at {n}");
        let mut timed = world.clone();
        let cpu_started = Instant::now();
        let ticks = run_ticks(&mut timed, end);
        let cpu_tick_ms = cpu_started.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(ticks, 60, "timed ticks at {n}");
        assert_ne!(timed.players[0].x, world.players[0].x, "cpu tick did not move agent 0 at {n}");
        let actor = Principal { org: 1, user: ENGINE, now_ms: end };
        let step_started = Instant::now();
        let stepped = transition(&actor, &world, &Command::Step).unwrap_or_else(|err| panic!("step {err:?}"));
        let cpu_step_ms = step_started.elapsed().as_secs_f64() * 1_000.0;
        let _ = stepped;

        let (xs, ys, txs, tys, mask, width, height) = columns(&world);
        let step = 12u32;
        let _ = dev
            .seek(&xs, &ys, &txs, &tys, step, &mask, width, height)
            .unwrap_or_else(|err| panic!("warmup {}: {}", err.op, err.code));
        let one = dev
            .seek_repeat(&xs, &ys, &txs, &tys, step, &mask, width, height, n, 1)
            .unwrap_or_else(|err| panic!("seek {}: {}", err.op, err.code));
        assert!(one.out.errors.iter().all(|e| *e == 0), "gpu error at {n}");
        let sixty = dev
            .seek_repeat(&xs, &ys, &txs, &tys, step, &mask, width, height, n, 60)
            .unwrap_or_else(|err| panic!("seek60 {}: {}", err.op, err.code));
        assert_ne!(sixty.out.xs[0], xs[0], "gpu seek did not move agent 0 at {n}");
        let row = format!(
            "{n},{cpu_tick_ms:.3},{cpu_step_ms:.3},{:.3},{:.3},{:.3},{:.3}",
            one.kernel_ms, one.wall_ms, sixty.kernel_ms, sixty.wall_ms
        );
        println!(
            "fair agents={n} cpu_tick_ms={cpu_tick_ms:.2} cpu_step_ms={cpu_step_ms:.2} gpu60_kernel_ms={:.2} gpu60_wall_ms={:.2}",
            sixty.kernel_ms, sixty.wall_ms
        );
        rows.push(row);
    }
    let path = format!("{dir}/fair.csv");
    std::fs::write(&path, rows.join("\n") + "\n").unwrap_or_else(|err| panic!("write {path}: {err}"));
    println!("wrote {path}");
}

fn side_for(n: usize) -> u32 {
    let mut side = 48u32;
    while (side as usize) * (side as usize) < n {
        side = side.saturating_mul(2);
    }
    side
}

fn world(n: usize) -> Snapshot {
    let side = side_for(n);
    let cells = (side as usize) * (side as usize);
    let mut snap = Snapshot::default();
    snap.meta.width = side;
    snap.meta.height = side;
    snap.meta.next_id = (n as u64) + 1;
    snap.meta.now_ms = 1_000_000;
    snap.terrain.mask = vec![0u8; (cells - 1) / 8 + 1];
    let now = snap.meta.now_ms;
    let mut i = 0usize;
    while i < n {
        let tx = (i % side as usize) as u32;
        let ty = (i / side as usize) as u32;
        let x = tx * MILLI;
        let y = ty * MILLI;
        let dest_tx = if tx + 3 < side { tx + 3 } else { tx - 3 };
        let dest_x = dest_tx * MILLI;
        let id = (i as u64) + 1;
        snap.players.push(Player {
            id,
            owner: id,
            is_human: false,
            x,
            y,
            face_dx: FACE_BIAS + MILLI,
            face_dy: FACE_BIAS,
            speed: 0,
            dest_x,
            dest_y: y,
            pf: PF_MOVE,
            pf_started: now,
            wait_until: 0,
            last_input: now,
            activity: 0,
            activity_until: 0,
            character: 0,
            name: Vec::new(),
        });
        let dt = 4_000u64;
        snap.paths.push(PathNode {
            player: id,
            seq: 0,
            x,
            y,
            face_dx: FACE_BIAS + MILLI,
            face_dy: FACE_BIAS,
            t: now,
        });
        snap.paths.push(PathNode {
            player: id,
            seq: 1,
            x: dest_x,
            y,
            face_dx: FACE_BIAS + MILLI,
            face_dy: FACE_BIAS,
            t: now + dt,
        });
        i += 1;
    }
    snap
}

fn columns(snap: &Snapshot) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u8>, u32, u32) {
    let n = snap.players.len();
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut txs = Vec::with_capacity(n);
    let mut tys = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        xs.push(snap.players[i].x);
        ys.push(snap.players[i].y);
        txs.push(snap.players[i].dest_x / MILLI);
        tys.push(snap.players[i].dest_y / MILLI);
        i += 1;
    }
    (xs, ys, txs, tys, snap.terrain.mask.clone(), snap.meta.width, snap.meta.height)
}
