//! Two-phase movement: one CPU thread, several CPU threads, and the GPU.
//!
//! `cpu_tick_ms` is the product `run_ticks` (path pose, wait, and the round).
//! `cpu_round_ms` is 60 calls of `tick_round` on one thread. `cpu_par<T>_ms`
//! is the same rounds on a pool of `T` threads, started once (thread counts
//! from `AI_TOWN_CPU_THREADS`, default `8,16,32,64`). The workers also clear and
//! fill the grid, with atomic pushes like the PTX inserts, so no phase is
//! serial. The GPU number is those 60 rounds resident on the device:
//! one copy in, one copy out. `gpu_checked_ms` includes preparation and the
//! proven Relocate check. AI_TOWN_GPU_VERIFY=1 also checks every device grid.
//! Samples start from identical states, after one unrecorded warmup.
//!
//! `AI_TOWN_SCENARIO` picks the crowd. `east` (default): every agent walks
//! east in step one tile from its neighbors, so no landing is ever contested.
//! `headon`: each row alternates eastbound and westbound agents 1.5 tiles
//! apart, so every pair meets within the batch and keeps proposing into a
//! refused landing. `refused` counts (round, agent) proposals that did not
//! land, from the single-thread run.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{
    cell_of, path_pose, propose_one, resolve_one, run_ticks, run_ticks_seq, placement_ok, relocate, tick_round,
    PathNode, Player, Snapshot, FACE_BIAS, MILLI, PF_MOVE, TICK_MS,
};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let ptx = std::env::var("AI_TOWN_PTX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/dev/shm/ai-town-rust/nvvm-target/nvptx64-nvidia-cuda/release/ai_town_seek_device.ptx"));
    let mut dev = RustDevice::open_path(&ptx).unwrap_or_else(|err| panic!("cuda {}: {}", err.op, err.code));
    let verify_grid = std::env::var("AI_TOWN_GPU_VERIFY").ok().as_deref() == Some("1");
    dev.set_verify_grid(verify_grid);
    let dir = std::env::var("AI_TOWN_BENCH_DIR").unwrap_or_else(|_| "/dev/shm/ai-town-rust/bench".into());
    std::fs::create_dir_all(&dir).expect("benchmark directory");
    let available = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let thread_counts: Vec<usize> = std::env::var("AI_TOWN_CPU_THREADS")
        .unwrap_or_else(|_| "8,16,32,64".into())
        .split(',')
        .map(|t| t.trim().parse::<usize>().expect("AI_TOWN_CPU_THREADS").max(1).min(available))
        .collect();
    let sizes: Vec<usize> = std::env::var("AI_TOWN_BENCH_AGENTS")
        .unwrap_or_else(|_| "1000,10000,50000,100000".into())
        .split(',').map(|n| n.trim().parse().expect("AI_TOWN_BENCH_AGENTS")).collect();
    let samples: usize = std::env::var("AI_TOWN_BENCH_SAMPLES")
        .unwrap_or_else(|_| "5".into()).parse().expect("AI_TOWN_BENCH_SAMPLES");
    assert!(samples > 0 && sizes.iter().all(|&n| n > 0));
    let scenario = std::env::var("AI_TOWN_SCENARIO").unwrap_or_else(|_| "east".into());
    let headon = match scenario.as_str() {
        "east" => false,
        "headon" => true,
        other => panic!("AI_TOWN_SCENARIO: {other}"),
    };
    println!("scenario={scenario} verify_grid={verify_grid} cpu_threads={thread_counts:?} samples={samples}");
    let par_cols: Vec<String> = thread_counts.iter().map(|t| format!("cpu_par{t}_ms")).collect();
    let path = format!("{dir}/fair-{scenario}.csv");
    let mut csv = std::fs::File::create(&path).expect("CSV");
    use std::io::Write;
    writeln!(csv, "players,sample,verify_grid,refused,cpu_tick_ms,cpu_seq_ms,cpu_round_ms,{},gpu_60_kernel_ms,gpu_60_wall_ms,prepare_ms,relocate_ms,gpu_checked_ms", par_cols.join(",")).expect("CSV header");
    for n in sizes {
        let world = world(n, headon);
        assert!(placement_ok(&world), "initial placement at {n}");
        let end = world.meta.now_ms + 60 * TICK_MS;
        // Every sample starts from the same state; sample 0 is an unrecorded warmup.
        for sample in 0..=samples {
            let mut timed = world.clone();
            let cpu_started = Instant::now();
            let ticks = run_ticks(&mut timed, end);
            let cpu_tick_ms = cpu_started.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(ticks, 60);
            assert!(placement_ok(&timed));
            assert_ne!(timed.players[0].x, world.players[0].x);

            let mut sequential = world.clone();
            let cpu_started = Instant::now();
            assert_eq!(run_ticks_seq(&mut sequential, end), 60);
            let cpu_seq_ms = cpu_started.elapsed().as_secs_f64() * 1_000.0;
            assert!(placement_ok(&sequential));

            let pack = pack_world(&world);
            let serial = drive(&pack);
            if headon { assert!(serial.3 > n); }
            let mut par_ms = Vec::new();
            for &threads in &thread_counts {
                let parallel = drive_parallel(&pack, threads);
                assert_eq!(serial.0, parallel.0, "parallel x at {n} with {threads}");
                assert_eq!(serial.1, parallel.1, "parallel y at {n} with {threads}");
                par_ms.push(parallel.2);
            }

            // Include candidate generation, flags, copies, launches, snapshot
            // cloning, and the actual proven Relocate acceptance check. Database
            // persistence and server/RPC overhead belong to the pipe benchmark.
            let pipeline = Instant::now();
            let gpu_pack = pack_world(&world);
            let cand_on = vec![1u32; n * 60];
            let prepare_ms = pipeline.elapsed().as_secs_f64() * 1_000.0;
            let report = dev.rounds(
                &gpu_pack.xs, &gpu_pack.ys, &gpu_pack.moving,
                &gpu_pack.cand_x, &gpu_pack.cand_y, &cand_on, &gpu_pack.ids,
                &gpu_pack.mask, gpu_pack.width, gpu_pack.height, 60,
            ).unwrap_or_else(|err| panic!("rounds {}: {}", err.op, err.code));
            assert_eq!(report.out.error, 0);
            let checked = Instant::now();
            let landed = relocate(world.clone(), &report.out.xs, &report.out.ys).expect("GPU Relocate accepted");
            let relocate_ms = checked.elapsed().as_secs_f64() * 1_000.0;
            let gpu_checked_ms = pipeline.elapsed().as_secs_f64() * 1_000.0;
            assert!(placement_ok(&landed));
            assert_eq!(report.out.xs, serial.0, "gpu x at {n}");
            assert_eq!(report.out.ys, serial.1, "gpu y at {n}");
            if !headon {
                for i in 0..n {
                    assert_eq!(timed.players[i].x, serial.0[i]);
                    assert_eq!(timed.players[i].y, serial.1[i]);
                    assert_eq!(sequential.players[i].x, serial.0[i]);
                    assert_eq!(sequential.players[i].y, serial.1[i]);
                    assert_eq!(serial.0[i], world.players[i].x + 720);
                }
            }
            if sample == 0 { continue; }
            let par_csv: Vec<String> = par_ms.iter().map(|ms| format!("{ms:.3}")).collect();
            writeln!(csv, "{n},{},{},{},{cpu_tick_ms:.3},{cpu_seq_ms:.3},{:.3},{},{:.3},{:.3},{prepare_ms:.3},{relocate_ms:.3},{gpu_checked_ms:.3}",
                sample - 1, u8::from(verify_grid), serial.3, serial.2, par_csv.join(","), report.kernel_ms, report.wall_ms).expect("CSV row");
            csv.flush().expect("CSV flush");
            println!("fair {scenario} players={n} sample={} cpu_tick_ms={cpu_tick_ms:.3} cpu_seq_ms={cpu_seq_ms:.3} gpu_checked_ms={gpu_checked_ms:.3}", sample - 1);
        }
    }
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

fn pack_world(world: &Snapshot) -> Pack {
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

/// Landed x, landed y, milliseconds, and refused proposals. Candidates are
/// already filled. The refusal count is taken outside the timed rounds.
fn drive(pack: &Pack) -> (Vec<u32>, Vec<u32>, f64, usize) {
    let n = pack.xs.len();
    let mut xs = pack.xs.clone();
    let mut ys = pack.ys.clone();
    let cells = (pack.width as usize) * (pack.height as usize);
    let mut head = vec![u32::MAX; cells];
    let mut next = vec![u32::MAX; n];
    let mut prop_x = vec![0u32; n];
    let mut prop_y = vec![0u32; n];
    let mut prop_on = vec![0u32; n];
    let mut refused = 0usize;
    let mut elapsed = 0f64;
    let mut round = 0usize;
    while round < 60 {
        let base = round * n;
        let before = (xs.clone(), ys.clone());
        let started = Instant::now();
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
        elapsed += started.elapsed().as_secs_f64() * 1_000.0;
        assert!(ok, "round {round}");
        let mut i = 0;
        while i < n {
            let wants = pack.moving[i] != 0 && (pack.cand_x[base + i], pack.cand_y[base + i]) != (before.0[i], before.1[i]);
            if wants && prop_on[i] == 0 {
                refused += 1;
            }
            i += 1;
        }
        round += 1;
    }
    (xs, ys, elapsed, refused)
}

/// Same decisions as `tick_round`. Workers are started once and each owns an
/// agent range and a cell range. Each round runs six phases behind a barrier:
/// clear, insert positions, propose, clear, insert proposals, resolve. Inserts
/// push with an atomic swap, so chain order differs from `grid_build`; the
/// grid answers do not depend on it (`grid_answers_do_not_depend_on_chain_order`).
fn drive_parallel(pack: &Pack, threads: usize) -> (Vec<u32>, Vec<u32>, f64) {
    use std::sync::atomic::{AtomicU32, Ordering};
    let n = pack.xs.len();
    let threads = threads.max(1).min(n.max(1));
    let mut xs = pack.xs.clone();
    let mut ys = pack.ys.clone();
    let cells = (pack.width as usize) * (pack.height as usize);
    let head: Vec<AtomicU32> = (0..cells).map(|_| AtomicU32::new(u32::MAX)).collect();
    let mut next = vec![u32::MAX; n];
    let mut prop_x = vec![0u32; n];
    let mut prop_y = vec![0u32; n];
    let mut prop_on = vec![0u32; n];
    let ctrl = Spin::default();
    let xs_p = xs.as_mut_ptr() as usize;
    let ys_p = ys.as_mut_ptr() as usize;
    let px_p = prop_x.as_mut_ptr() as usize;
    let py_p = prop_y.as_mut_ptr() as usize;
    let po_p = prop_on.as_mut_ptr() as usize;
    let next_p = next.as_mut_ptr() as usize;
    let head = &head;
    let chunk = (n + threads - 1) / threads;
    let worker_count = (n + chunk - 1) / chunk;
    let cell_chunk = (cells + worker_count - 1) / worker_count;
    let ctrl = &ctrl;
    let started = std::thread::scope(|scope| {
        let mut w = 0usize;
        while w < worker_count {
            let range = w * chunk;
            let take = chunk.min(n - range);
            let c_lo = (w * cell_chunk).min(cells);
            let c_hi = ((w + 1) * cell_chunk).min(cells);
            scope.spawn(move || {
                let mut seen = 0usize;
                loop {
                    let (phase, base) = ctrl.wait(seen);
                    seen += 1;
                    if phase == PH_EXIT {
                        break;
                    }
                    // Each worker writes only its own agent range of the plain
                    // buffers and its own cell range of `head` when clearing.
                    // Every other `head` write is an atomic swap. The barrier
                    // orders the phases. Addresses are `usize` so the closure
                    // can move to a thread.
                    unsafe {
                        let xs_p = xs_p as *mut u32;
                        let ys_p = ys_p as *mut u32;
                        let px_p = px_p as *mut u32;
                        let py_p = py_p as *mut u32;
                        let po_p = po_p as *mut u32;
                        let next_p = next_p as *mut u32;
                        match phase {
                            PH_CLEAR => {
                                let mut c = c_lo;
                                while c < c_hi {
                                    head[c].store(u32::MAX, Ordering::Relaxed);
                                    c += 1;
                                }
                            }
                            PH_INSERT_POS | PH_INSERT_PROP => {
                                let (px, py) = if phase == PH_INSERT_POS { (xs_p, ys_p) } else { (px_p, py_p) };
                                let mut k = 0;
                                while k < take {
                                    let i = range + k;
                                    let on = phase == PH_INSERT_POS || *po_p.add(i) != 0;
                                    let cell = cell_of(*px.add(i), *py.add(i), pack.width, pack.height);
                                    *next_p.add(i) = if on && cell < cells { head[cell].swap(i as u32, Ordering::Relaxed) } else { u32::MAX };
                                    k += 1;
                                }
                            }
                            PH_PROPOSE => {
                                let xs_s = std::slice::from_raw_parts(xs_p, n);
                                let ys_s = std::slice::from_raw_parts(ys_p, n);
                                let head_s = std::slice::from_raw_parts(head.as_ptr() as *const u32, cells);
                                let next_s = std::slice::from_raw_parts(next_p, n);
                                let mut k = 0;
                                while k < take {
                                    let i = range + k;
                                    let proposal = propose_one(
                                        xs_s[i], ys_s[i], 0, 0, pack.moving[i] != 0, pack.cand_x[base + i], pack.cand_y[base + i], 0, 0, 0, &pack.mask, pack.width, pack.height, head_s, next_s, xs_s, ys_s, i as u32,
                                    );
                                    *px_p.add(i) = proposal.x;
                                    *py_p.add(i) = proposal.y;
                                    *po_p.add(i) = if proposal.proposed { 1 } else { 0 };
                                    k += 1;
                                }
                            }
                            _ => {
                                let head_s = std::slice::from_raw_parts(head.as_ptr() as *const u32, cells);
                                let next_s = std::slice::from_raw_parts(next_p, n);
                                let px = std::slice::from_raw_parts(px_p, n);
                                let py = std::slice::from_raw_parts(py_p, n);
                                let mut k = 0;
                                while k < take {
                                    let i = range + k;
                                    let land = resolve_one(
                                        *xs_p.add(i), *ys_p.add(i), 0, 0, px[i], py[i], 0, 0, 0, *po_p.add(i) != 0, pack.ids[i], head_s, next_s, px, py, &pack.ids, i as u32, pack.width, pack.height,
                                    );
                                    *xs_p.add(i) = land.x;
                                    *ys_p.add(i) = land.y;
                                    *po_p.add(i) = if land.moved { 1 } else { 0 };
                                    k += 1;
                                }
                            }
                        }
                    }
                    ctrl.arrived.fetch_add(1, Ordering::AcqRel);
                }
            });
            w += 1;
        }
        let clock = Instant::now();
        let mut round = 0usize;
        while round < 60 {
            let base = round * n;
            for phase in [PH_CLEAR, PH_INSERT_POS, PH_PROPOSE, PH_CLEAR, PH_INSERT_PROP, PH_RESOLVE] {
                ctrl.release(phase, base, worker_count);
            }
            round += 1;
        }
        ctrl.release_exit();
        clock.elapsed().as_secs_f64() * 1_000.0
    });
    (xs, ys, started)
}

const PH_CLEAR: u8 = 0;
const PH_INSERT_POS: u8 = 1;
const PH_PROPOSE: u8 = 2;
const PH_INSERT_PROP: u8 = 3;
const PH_RESOLVE: u8 = 4;
const PH_EXIT: u8 = 5;

/// A spinning barrier. The main thread publishes a phase by bumping `epoch`,
/// workers spin until they see it, run the phase, and bump `arrived`. A
/// condvar costs about a millisecond per phase at 128 threads, which is more
/// than a whole round of work.
#[derive(Default)]
struct Spin {
    epoch: std::sync::atomic::AtomicUsize,
    phase: std::sync::atomic::AtomicU8,
    base: std::sync::atomic::AtomicUsize,
    arrived: std::sync::atomic::AtomicUsize,
}

impl Spin {
    /// Wait for the phase after `seen`, then return it and the candidate base.
    fn wait(&self, seen: usize) -> (u8, usize) {
        use std::sync::atomic::Ordering;
        while self.epoch.load(Ordering::Acquire) == seen {
            std::hint::spin_loop();
        }
        (self.phase.load(Ordering::Relaxed), self.base.load(Ordering::Relaxed))
    }

    /// Run one phase on every worker and wait for all of them.
    fn release(&self, phase: u8, base: usize, workers: usize) {
        use std::sync::atomic::Ordering;
        self.arrived.store(0, Ordering::Relaxed);
        self.phase.store(phase, Ordering::Relaxed);
        self.base.store(base, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Release);
        while self.arrived.load(Ordering::Acquire) < workers {
            std::hint::spin_loop();
        }
    }

    fn release_exit(&self) {
        use std::sync::atomic::Ordering;
        self.phase.store(PH_EXIT, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Release);
    }
}


fn side_for(n: usize) -> u32 {
    let mut side = 48u32;
    while (side as usize) * (side as usize) < n {
        side = side.saturating_mul(2);
    }
    side
}

fn world(n: usize, headon: bool) -> Snapshot {
    let side = if headon { side_for(n * 3 / 2) } else { side_for(n) };
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
        let (x, y, dest_x) = if headon {
            // Rows one tile apart. Along a row, agents sit 1.5 tiles apart and
            // alternate east and west, so each eastbound agent closes on the
            // westbound one ahead of it at 1.5 tiles per second.
            let per_row = (side as usize) * 2 / 3;
            let col = (i % per_row) as u32;
            let row = (i / per_row) as u32;
            let x = MILLI + col * (3 * MILLI / 2);
            let y = row * MILLI;
            let dest_x = if col % 2 == 0 { x + 3 * MILLI } else { x - 3 * MILLI / 2 };
            (x, y, dest_x)
        } else {
            let tx = (i % side as usize) as u32;
            let ty = (i / side as usize) as u32;
            // Every agent walks east by the same amount. A neighbor who stayed
            // would fall inside the collision radius within 60 ticks, and the
            // CPU tick would then wait while the device kept retrying.
            (tx * MILLI, ty * MILLI, tx * MILLI + 3 * MILLI)
        };
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
