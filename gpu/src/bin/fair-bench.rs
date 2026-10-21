//! Spatial-grid `run_ticks` versus the same tick on the PTX image.
//!
//! The CPU number is one product step: 60 ticks of path following. The GPU
//! number is those 60 ticks of `tick_moves`: the host asks `path_pose` for
//! each candidate, rebuilds the one-tile grid, and launches one thread. The
//! O(n^2) `seek_one` scan is not timed here.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{
    near_index, path_pose, run_ticks, transition, Command, PathNode, Player, Principal, Snapshot, ENGINE, FACE_BIAS, MILLI,
    PF_MOVE, TICK_MS,
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

        let _ = gpu_rounds(&dev, &world, 1);
        let one = gpu_rounds(&dev, &world, 1);
        let sixty = gpu_rounds(&dev, &world, 60);
        assert_ne!(sixty.2[0], world.players[0].x, "gpu tick did not move agent 0 at {n}");
        let mut i = 0;
        while i < n {
            assert_eq!(sixty.2[i], timed.players[i].x, "x {i} at {n}");
            assert_eq!(sixty.3[i], timed.players[i].y, "y {i} at {n}");
            i += 1;
        }
        let row = format!(
            "{n},{cpu_tick_ms:.3},{cpu_step_ms:.3},{:.3},{:.3},{:.3},{:.3}",
            one.0, one.1, sixty.0, sixty.1
        );
        println!(
            "fair agents={n} cpu_tick_ms={cpu_tick_ms:.2} cpu_step_ms={cpu_step_ms:.2} gpu60_kernel_ms={:.2} gpu60_wall_ms={:.2}",
            sixty.0, sixty.1
        );
        rows.push(row);
    }
    let path = format!("{dir}/fair.csv");
    std::fs::write(&path, rows.join("\n") + "\n").unwrap_or_else(|err| panic!("write {path}: {err}"));
    println!("wrote {path}");
}

/// Kernel milliseconds, wall milliseconds, and the landed positions.
fn gpu_rounds(dev: &RustDevice, world: &Snapshot, rounds: u32) -> (f32, f64, Vec<u32>, Vec<u32>) {
    let n = world.players.len();
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut moving = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        xs.push(world.players[i].x);
        ys.push(world.players[i].y);
        moving.push(if world.players[i].pf == PF_MOVE { 1 } else { 0 });
        i += 1;
    }
    let width = world.meta.width;
    let height = world.meta.height;
    let now = world.meta.now_ms;
    let wall = Instant::now();
    let mut kernel_ms = 0f32;
    let mut round = 0u32;
    while round < rounds {
        let t = now + TICK_MS * (u64::from(round) + 1);
        let mut cand_x = vec![0u32; n];
        let mut cand_y = vec![0u32; n];
        let mut i = 0;
        while i < n {
            if moving[i] != 0 {
                let lo = i * 2;
                let pose = path_pose(&world.paths[lo..lo + 2], t).unwrap_or_else(|| panic!("path {i}"));
                cand_x[i] = pose.x;
                cand_y[i] = pose.y;
            } else {
                cand_x[i] = xs[i];
                cand_y[i] = ys[i];
            }
            i += 1;
        }
        let g = near_index(width, height, &xs, &ys);
        let report = dev
            .step(&xs, &ys, &moving, &cand_x, &cand_y, &world.terrain.mask, width, height, &g.head, &g.next)
            .unwrap_or_else(|err| panic!("step {}: {}", err.op, err.code));
        assert_eq!(report.out.error, 0, "device error at round {round}");
        xs = report.out.xs;
        ys = report.out.ys;
        kernel_ms += report.kernel_ms;
        round += 1;
    }
    (kernel_ms, wall.elapsed().as_secs_f64() * 1_000.0, xs, ys)
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
        // Every agent walks east by the same amount. A neighbor who stayed
        // would fall inside the collision radius within 60 ticks, and the
        // CPU tick would then wait while the device kept retrying.
        let dest_x = x + 3 * MILLI;
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
