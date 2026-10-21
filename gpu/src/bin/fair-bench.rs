//! Two-phase movement: one CPU thread, several CPU threads, and the GPU.
//!
//! `cpu_tick_ms` is the product `run_ticks` (path pose, wait, and the round).
//! `cpu_round_ms` is 60 calls of `tick_round` on one thread. `cpu_parallel_ms`
//! is the same rounds on a pool of at most eight threads, started once.
//! The grid build stays on the main thread. The GPU number is those 60 rounds
//! resident on the device: one copy in, one copy out. The O(n^2) `seek_one`
//! scan is not timed.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{
    grid_build, path_pose, propose_one, resolve_one, run_ticks, tick_round, PathNode, Player, Snapshot, FACE_BIAS, MILLI, PF_MOVE,
    TICK_MS,
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
    let available = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    // Eight workers is enough to overlap the agent loops. More threads spend
    // the minute waking up, and the grid build stays on one thread either way.
    let threads = available.min(8);
    println!("cpu_threads={threads} cpu_available={available}");
    let mut rows = vec!["agents,cpu_tick_ms,cpu_round_ms,cpu_parallel_ms,gpu_one_kernel_ms,gpu_one_wall_ms,gpu_60_kernel_ms,gpu_60_wall_ms".to_string()];
    for n in [1_000usize, 10_000, 50_000, 100_000] {
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

        let pack = pack(&world);
        let _ = drive(&pack);
        let serial = drive(&pack);
        let _ = drive_parallel(&pack, threads);
        let parallel = drive_parallel(&pack, threads);
        assert_eq!(serial.0, parallel.0, "parallel x at {n}");
        assert_eq!(serial.1, parallel.1, "parallel y at {n}");

        let _ = gpu_rounds(&dev, &pack, 1);
        let one = gpu_rounds(&dev, &pack, 1);
        let sixty = gpu_rounds(&dev, &pack, 60);
        assert_ne!(sixty.2[0], world.players[0].x, "gpu tick did not move agent 0 at {n}");
        let mut i = 0;
        while i < n {
            assert_eq!(sixty.2[i], serial.0[i], "gpu x {i} at {n}");
            assert_eq!(sixty.3[i], serial.1[i], "gpu y {i} at {n}");
            assert_eq!(timed.players[i].x, serial.0[i], "tick x {i} at {n}");
            assert_eq!(timed.players[i].y, serial.1[i], "tick y {i} at {n}");
            i += 1;
        }
        let row = format!(
            "{n},{cpu_tick_ms:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
            serial.2, parallel.2, one.0, one.1, sixty.0, sixty.1
        );
        println!(
            "fair agents={n} cpu_tick_ms={cpu_tick_ms:.2} cpu_round_ms={:.2} cpu_parallel_ms={:.2} gpu60_kernel_ms={:.2} gpu60_wall_ms={:.2}",
            serial.2, parallel.2, sixty.0, sixty.1
        );
        rows.push(row);
    }
    let path = format!("{dir}/fair.csv");
    std::fs::write(&path, rows.join("\n") + "\n").unwrap_or_else(|err| panic!("write {path}: {err}"));
    println!("wrote {path}");
}

struct Pack {
    xs: Vec<u32>,
    ys: Vec<u32>,
    moving: Vec<u32>,
    ids: Vec<u64>,
    cand_x: Vec<u32>,
    cand_y: Vec<u32>,
    mask: Vec<u8>,
    width: u32,
    height: u32,
}

fn pack(world: &Snapshot) -> Pack {
    let n = world.players.len();
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut moving = Vec::with_capacity(n);
    let mut ids = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        xs.push(world.players[i].x);
        ys.push(world.players[i].y);
        moving.push(if world.players[i].pf == PF_MOVE { 1 } else { 0 });
        ids.push(world.players[i].id);
        i += 1;
    }
    let mut cand_x = vec![0u32; n * 60];
    let mut cand_y = vec![0u32; n * 60];
    let now = world.meta.now_ms;
    let mut round = 0usize;
    while round < 60 {
        let t = now + TICK_MS * ((round as u64) + 1);
        let base = round * n;
        let mut i = 0;
        while i < n {
            let lo = i * 2;
            let pose = path_pose(&world.paths[lo..lo + 2], t).unwrap_or_else(|| panic!("path {i}"));
            cand_x[base + i] = pose.x;
            cand_y[base + i] = pose.y;
            i += 1;
        }
        round += 1;
    }
    Pack { xs, ys, moving, ids, cand_x, cand_y, mask: world.terrain.mask.clone(), width: world.meta.width, height: world.meta.height }
}

/// Landed x, landed y, and milliseconds. Candidates are already filled.
fn drive(pack: &Pack) -> (Vec<u32>, Vec<u32>, f64) {
    let n = pack.xs.len();
    let mut xs = pack.xs.clone();
    let mut ys = pack.ys.clone();
    let cells = (pack.width as usize) * (pack.height as usize);
    let mut head = vec![u32::MAX; cells];
    let mut next = vec![u32::MAX; n];
    let mut prop_x = vec![0u32; n];
    let mut prop_y = vec![0u32; n];
    let mut prop_on = vec![0u32; n];
    let started = Instant::now();
    let mut round = 0usize;
    while round < 60 {
        let base = round * n;
        let ok = tick_round(
            &mut xs,
            &mut ys,
            &pack.moving,
            &pack.cand_x[base..base + n],
            &pack.cand_y[base..base + n],
            &pack.ids,
            &pack.mask,
            pack.width,
            pack.height,
            &mut head,
            &mut next,
            &mut prop_x,
            &mut prop_y,
            &mut prop_on,
        );
        assert!(ok, "round {round}");
        round += 1;
    }
    (xs, ys, started.elapsed().as_secs_f64() * 1_000.0)
}

struct Ctrl {
    epoch: usize,
    phase: u8,
    arrived: usize,
    base: usize,
}

/// Same decisions as `tick_round`. Workers are started once and each writes a
/// private index range. The main thread rebuilds the grid between phases.
fn drive_parallel(pack: &Pack, threads: usize) -> (Vec<u32>, Vec<u32>, f64) {
    let n = pack.xs.len();
    let threads = threads.max(1).min(n.max(1));
    let mut xs = pack.xs.clone();
    let mut ys = pack.ys.clone();
    let cells = (pack.width as usize) * (pack.height as usize);
    let mut head = vec![u32::MAX; cells];
    let mut next = vec![u32::MAX; n];
    let mut prop_x = vec![0u32; n];
    let mut prop_y = vec![0u32; n];
    let mut prop_on = vec![0u32; n];
    let ctrl = std::sync::Mutex::new(Ctrl { epoch: 0, phase: 0, arrived: 0, base: 0 });
    let cv = std::sync::Condvar::new();
    let xs_p = xs.as_mut_ptr() as usize;
    let ys_p = ys.as_mut_ptr() as usize;
    let px_p = prop_x.as_mut_ptr() as usize;
    let py_p = prop_y.as_mut_ptr() as usize;
    let po_p = prop_on.as_mut_ptr() as usize;
    let head_p = head.as_ptr() as usize;
    let next_p = next.as_ptr() as usize;
    let chunk = (n + threads - 1) / threads;
    let mut worker_count = 0usize;
    let mut cursor = 0usize;
    while cursor < n {
        worker_count += 1;
        cursor += chunk.min(n - cursor);
    }
    let ctrl = &ctrl;
    let cv = &cv;
    let started = std::thread::scope(|scope| {
        let mut start = 0usize;
        while start < n {
            let take = chunk.min(n - start);
            let range = start;
            let limit = worker_count;
            scope.spawn(move || {
                let mut seen = 0usize;
                loop {
                    let (phase, base) = {
                        let mut guard = ctrl.lock().unwrap_or_else(|err| err.into_inner());
                        while guard.epoch == seen {
                            guard = cv.wait(guard).unwrap_or_else(|err| err.into_inner());
                        }
                        seen = guard.epoch;
                        (guard.phase, guard.base)
                    };
                    if phase == 3 {
                        break;
                    }
                    // Each worker's mutable range is disjoint. The main thread
                    // does not touch these buffers until every worker arrives.
                    // Addresses are `usize` so the closure can move to a thread.
                    unsafe {
                        let xs_p = xs_p as *mut u32;
                        let ys_p = ys_p as *mut u32;
                        let px_p = px_p as *mut u32;
                        let py_p = py_p as *mut u32;
                        let po_p = po_p as *mut u32;
                        let xs_s = std::slice::from_raw_parts(xs_p, n);
                        let ys_s = std::slice::from_raw_parts(ys_p, n);
                        let head_s = std::slice::from_raw_parts(head_p as *const u32, cells);
                        let next_s = std::slice::from_raw_parts(next_p as *const u32, n);
                        if phase == 1 {
                            let px = std::slice::from_raw_parts_mut(px_p.add(range), take);
                            let py = std::slice::from_raw_parts_mut(py_p.add(range), take);
                            let po = std::slice::from_raw_parts_mut(po_p.add(range), take);
                            let mut k = 0;
                            while k < take {
                                let i = range + k;
                                let proposal = propose_one(
                                    xs_s[i], ys_s[i], 0, 0, pack.moving[i] != 0, pack.cand_x[base + i], pack.cand_y[base + i], 0, 0, 0, &pack.mask, pack.width, pack.height, head_s, next_s, xs_s, ys_s, i as u32,
                                );
                                px[k] = proposal.x;
                                py[k] = proposal.y;
                                po[k] = if proposal.proposed { 1 } else { 0 };
                                k += 1;
                            }
                        } else {
                            let sx = std::slice::from_raw_parts_mut(xs_p.add(range), take);
                            let sy = std::slice::from_raw_parts_mut(ys_p.add(range), take);
                            let po = std::slice::from_raw_parts_mut(po_p.add(range), take);
                            let px = std::slice::from_raw_parts(px_p, n);
                            let py = std::slice::from_raw_parts(py_p, n);
                            let mut k = 0;
                            while k < take {
                                let i = range + k;
                                let land = resolve_one(
                                    sx[k], sy[k], 0, 0, px[i], py[i], 0, 0, 0, po[k] != 0, pack.ids[i], head_s, next_s, px, py, &pack.ids, i as u32, pack.width, pack.height,
                                );
                                sx[k] = land.x;
                                sy[k] = land.y;
                                po[k] = if land.moved { 1 } else { 0 };
                                k += 1;
                            }
                        }
                    }
                    let mut guard = ctrl.lock().unwrap_or_else(|err| err.into_inner());
                    guard.arrived += 1;
                    if guard.arrived == limit {
                        cv.notify_all();
                    }
                }
            });
            start += take;
        }
        let clock = Instant::now();
        let mut round = 0usize;
        while round < 60 {
            let mut i = 0;
            while i < n {
                prop_on[i] = 1;
                i += 1;
            }
            assert!(grid_build(&mut head, &mut next, pack.width, pack.height, &xs, &ys, &prop_on), "old grid {round}");
            release(ctrl, cv, 1, worker_count, round * n);
            assert!(grid_build(&mut head, &mut next, pack.width, pack.height, &prop_x, &prop_y, &prop_on), "prop grid {round}");
            release(ctrl, cv, 2, worker_count, round * n);
            round += 1;
        }
        {
            let mut guard = ctrl.lock().unwrap_or_else(|err| err.into_inner());
            guard.phase = 3;
            guard.epoch += 1;
            cv.notify_all();
        }
        clock.elapsed().as_secs_f64() * 1_000.0
    });
    (xs, ys, started)
}

fn release(ctrl: &std::sync::Mutex<Ctrl>, cv: &std::sync::Condvar, phase: u8, workers: usize, base: usize) {
    let mut guard = ctrl.lock().unwrap_or_else(|err| err.into_inner());
    guard.phase = phase;
    guard.base = base;
    guard.arrived = 0;
    guard.epoch += 1;
    cv.notify_all();
    while guard.arrived < workers {
        guard = cv.wait(guard).unwrap_or_else(|err| err.into_inner());
    }
}

/// Kernel milliseconds, wall milliseconds, and the landed positions.
fn gpu_rounds(dev: &RustDevice, pack: &Pack, rounds: u32) -> (f32, f64, Vec<u32>, Vec<u32>) {
    let n = pack.xs.len();
    let rows = (rounds as usize) * n;
    let cand_on = vec![1u32; rows];
    let report = dev
        .rounds(
            &pack.xs,
            &pack.ys,
            &pack.moving,
            &pack.cand_x[..rows],
            &pack.cand_y[..rows],
            &cand_on,
            &pack.ids,
            &pack.mask,
            pack.width,
            pack.height,
            rounds,
        )
        .unwrap_or_else(|err| panic!("rounds {}: {}", err.op, err.code));
    assert_eq!(report.out.error, 0, "device error");
    (report.kernel_ms, report.wall_ms, report.out.xs, report.out.ys)
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
