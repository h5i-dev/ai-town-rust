//! Differential check: PTX `round_phase` against host `tick_round`.
//!
//! Both sides propose from the previous positions and accept a candidate only
//! when no better-priority proposal (lower player id, then lower index) lies
//! inside the collision radius. Chain order on the device may differ. The
//! landed positions must match. `step_batch` remains the sequential baseline.
//! `seek_one` remains a one-row baseline. Skips when the PTX image is absent.
//! `AI_TOWN_REQUIRE_PTX=1` turns the skip into a failure.

use ai_town_gpu::{LayoutError, RustDevice, SeekArena, SeekRow, StepOut, MAX_AGENTS};
use ai_town_kernel::{near_index, scalar_seek, set_blocked, tick_moves, tick_round};
use std::path::PathBuf;
use std::sync::Mutex;

fn ptx_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("AI_TOWN_PTX") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let default = PathBuf::from("/dev/shm/ai-town-rust/nvvm-target/nvptx64-nvidia-cuda/release/ai_town_seek_device.ptx");
    if default.is_file() {
        Some(default)
    } else {
        None
    }
}

fn open_mask(width: u32, height: u32) -> Vec<u8> {
    let cells = (width as usize) * (height as usize);
    vec![0u8; cells.div_ceil(8)]
}

fn cpu_round(
    xs: &[u32],
    ys: &[u32],
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    ids: &[u64],
    mask: &[u8],
    width: u32,
    height: u32,
) -> (Vec<u32>, Vec<u32>) {
    let n = xs.len();
    let cells = (width as usize) * (height as usize);
    let mut head = vec![u32::MAX; cells];
    let mut next = vec![u32::MAX; n];
    let mut prop_x = vec![0u32; n];
    let mut prop_y = vec![0u32; n];
    let mut prop_on = vec![0u32; n];
    let mut xs = xs.to_vec();
    let mut ys = ys.to_vec();
    assert!(tick_round(
        &mut xs,
        &mut ys,
        moving,
        cand_x,
        cand_y,
        ids,
        mask,
        width,
        height,
        &mut head,
        &mut next,
        &mut prop_x,
        &mut prop_y,
        &mut prop_on,
    ));
    (xs, ys)
}

fn assert_round(
    dev: &RustDevice,
    xs: &[u32],
    ys: &[u32],
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    ids: &[u64],
    mask: &[u8],
    width: u32,
    height: u32,
) {
    let (cx, cy) = cpu_round(xs, ys, moving, cand_x, cand_y, ids, mask, width, height);
    let gpu = dev
        .round(xs, ys, moving, cand_x, cand_y, ids, mask, width, height)
        .unwrap_or_else(|err| panic!("{} {}", err.op, err.code));
    assert_eq!(gpu.out.error, 0, "device error");
    assert_eq!(StepOut { xs: gpu.out.xs, ys: gpu.out.ys, error: 0 }, StepOut { xs: cx, ys: cy, error: 0 });
}

#[test]
fn ptx_matches_tick_round() {
    let path = match ptx_path() {
        Some(path) => path,
        None => {
            if std::env::var("AI_TOWN_REQUIRE_PTX").ok().as_deref() == Some("1") {
                panic!("PTX image missing");
            }
            return;
        }
    };
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let dev = RustDevice::open_path(&path).unwrap_or_else(|err| panic!("open {}: {}", err.op, err.code));

    let empty = dev
        .round(&[], &[], &[], &[], &[], &[], &[], 8, 8)
        .unwrap_or_else(|err| panic!("{} {}", err.op, err.code));
    assert_eq!(empty.out, StepOut { xs: Vec::new(), ys: Vec::new(), error: 0 });

    let open = open_mask(8, 8);
    // Both propose. The lower id keeps 200. The other stays at 1000.
    assert_round(&dev, &[0, 1000], &[0, 0], &[1, 1], &[200, 800], &[0, 0], &[1, 2], &open, 8, 8);
    assert_round(&dev, &[0, 1000], &[0, 0], &[1, 1], &[200, 800], &[0, 0], &[9, 1], &open, 8, 8);

    let mut wall = open_mask(8, 8);
    set_blocked(&mut wall, 8, 8, 1, 0);
    assert_round(&dev, &[0], &[0], &[1], &[1000], &[0], &[1], &wall, 8, 8);

    assert_round(&dev, &[0, 1000], &[0, 0], &[1, 0], &[500, 1000], &[0, 0], &[1, 2], &open, 8, 8);
    assert_round(&dev, &[0, 3000], &[0, 0], &[1, 0], &[500, 3000], &[0, 0], &[1, 2], &open, 8, 8);
    assert_round(&dev, &[0], &[0], &[0], &[500], &[0], &[1], &open, 8, 8);

    // Same cell, gap 762. Each candidate is 750 from the other old position
    // and 738 from the other candidate. The lower id wins.
    assert_round(&dev, &[1000, 1762], &[1000, 1000], &[1, 1], &[1012, 1750], &[1000, 1000], &[1, 5], &open, 8, 8);
    assert_round(&dev, &[1000, 1762], &[1000, 1000], &[1, 1], &[1012, 1750], &[1000, 1000], &[5, 1], &open, 8, 8);

    // Eight agents share one cell and one point. A broken chain would let
    // somebody move. All of them stay.
    let mut pile_x = vec![1000u32; 8];
    let mut pile_y = vec![1000u32; 8];
    let pile_m = vec![1u32; 8];
    let pile_c = vec![1012u32; 8];
    let pile_cy = vec![1000u32; 8];
    let mut pile_ids = Vec::new();
    let mut i = 0u64;
    while i < 8 {
        pile_ids.push(i + 1);
        i += 1;
    }
    assert_round(&dev, &pile_x, &pile_y, &pile_m, &pile_c, &pile_cy, &pile_ids, &open, 8, 8);
    let _ = (&mut pile_x, &mut pile_y);

    // The sequential rule lets B step into the shell A vacated. Two-phase
    // still sees A's previous position, so B stays.
    assert_round(&dev, &[2000, 1248], &[0, 0], &[1, 1], &[2012, 1260], &[0, 0], &[1, 2], &open, 8, 8);
    let g = near_index(8, 8, &[2000, 1248], &[0, 0]);
    let mut seq_x = vec![2000u32, 1248];
    let mut seq_y = vec![0u32, 0];
    assert!(tick_moves(&mut seq_x, &mut seq_y, &[1, 1], &[2012, 1260], &[0, 0], &open, 8, 8, &g.head, &g.next));
    let seq = dev
        .step(&[2000, 1248], &[0, 0], &[1, 1], &[2012, 1260], &[0, 0], &open, 8, 8, &g.head, &g.next)
        .unwrap_or_else(|err| panic!("step {} {}", err.op, err.code));
    assert_eq!(seq.out.xs, seq_x);
    assert_eq!(seq.out.ys, seq_y);
    assert_eq!(seq_x, vec![2012, 1260]);

    let n = 128usize;
    let width = 16u32;
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut moving = Vec::new();
    let mut cand_x = Vec::new();
    let mut cand_y = Vec::new();
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i < n {
        let x = ((i % width as usize) as u32) * 1000;
        let y = ((i / width as usize) as u32) * 1000;
        xs.push(x);
        ys.push(y);
        moving.push(1);
        cand_x.push(x + 12);
        cand_y.push(y);
        ids.push((i as u64) + 1);
        i += 1;
    }
    let lattice = open_mask(width, 8);
    assert_round(&dev, &xs, &ys, &moving, &cand_x, &cand_y, &ids, &lattice, width, 8);

    // Round 2 must see the landing from round 1. A stale grid would let B
    // step to 900, which is clear of A's old tile and inside A's new one.
    let cand_x2 = vec![200u32, 1000, 200, 900];
    let cand_y2 = vec![0u32, 0, 0, 0];
    let cand_on = vec![1u32, 1, 1, 1];
    let gpu2 = dev
        .rounds(&[0, 1000], &[0, 0], &[1, 1], &cand_x2, &cand_y2, &cand_on, &[1, 2], &open, 8, 8, 2)
        .unwrap_or_else(|err| panic!("rounds {} {}", err.op, err.code));
    let (mut hx, mut hy) = cpu_round(&[0, 1000], &[0, 0], &[1, 1], &[200, 1000], &[0, 0], &[1, 2], &open, 8, 8);
    let (hx2, hy2) = cpu_round(&hx, &hy, &[1, 1], &[200, 900], &[0, 0], &[1, 2], &open, 8, 8);
    hx = hx2;
    hy = hy2;
    assert_eq!(gpu2.out.xs, hx);
    assert_eq!(gpu2.out.ys, hy);
    assert_eq!(hx, vec![200, 1000]);

    let mismatch = dev.round(&[0, 1], &[0], &[1, 1], &[0, 1], &[0, 0], &[1, 2], &open, 8, 8);
    assert_eq!(mismatch.err().map(|err| err.op), Some("shape"));
    let big = vec![0u32; MAX_AGENTS + 1];
    let big_ids = vec![0u64; MAX_AGENTS + 1];
    let too_many = dev.round(&big, &big, &big, &big, &big, &big_ids, &[], 4, 4);
    assert_eq!(too_many.err().map(|err| err.op), Some("capacity"));

    let rows = vec![
        SeekRow { x: 0, y: 0, tx: 4, ty: 1 },
        SeekRow { x: 2000, y: 1000, tx: 0, ty: 2 },
    ];
    let arena = SeekArena::pack(&rows, 2).unwrap();
    assert_eq!(arena.unpack(), rows);
    assert_eq!(SeekArena::pack(&rows, MAX_AGENTS + 1).err(), Some(LayoutError::Capacity));

    let (sx, sy) = scalar_seek(&[0], &[0], &[3], &[0], 100, &open, 8, 8);
    let seek = dev.seek(&[0], &[0], &[3], &[0], 100, &open, 8, 8).unwrap_or_else(|err| panic!("seek {} {}", err.op, err.code));
    assert_eq!((seek.xs, seek.ys), (sx, sy));
    assert_eq!(seek.errors, vec![0]);
    // The same device workspace has now served round, step and seek layouts.
    // Grow it again, then shrink with a different mask to expose stale slots.
    assert_round(&dev, &[0, 3000, 6000], &[0, 0, 0], &[1, 1, 0],
        &[200, 3200, 6000], &[0, 0, 0], &[1, 2, 3], &open, 8, 8);
    assert_round(&dev, &[0], &[0], &[1], &[1000], &[0], &[4], &wall, 8, 8);
}

/// A crowd that all walks toward the middle of a walled map, so most rounds
/// have contested landings. Every device grid is checked with the kernel's
/// `grid_wellformed`, and every round must land where `tick_round` lands.
#[test]
fn ptx_matches_tick_round_in_a_crowd_with_checked_grids() {
    let path = match ptx_path() {
        Some(path) => path,
        None => {
            if std::env::var("AI_TOWN_REQUIRE_PTX").ok().as_deref() == Some("1") {
                panic!("PTX image missing");
            }
            return;
        }
    };
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let mut dev = RustDevice::open_path(&path).unwrap_or_else(|err| panic!("open {}: {}", err.op, err.code));
    dev.set_verify_grid(true);

    let (w, h) = (40u32, 40u32);
    let mut mask = open_mask(w, h);
    let mut seed = 0x5eed_u64;
    let mut rand = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_f491_4f6c_dd1d)
    };
    for _ in 0..120 {
        set_blocked(&mut mask, w, h, (rand() % w as u64) as u32, (rand() % h as u64) as u32);
    }
    // One agent per open tile on a sparse lattice, so the start is collision-free.
    let (mut xs, mut ys, mut ids) = (Vec::new(), Vec::new(), Vec::new());
    for tx in (0..w).step_by(2) {
        for ty in (0..h).step_by(2) {
            if !ai_town_kernel::mask_blocked(&mask, w, h, tx * 1000 + 500, ty * 1000 + 500) {
                xs.push(tx * 1000 + 500);
                ys.push(ty * 1000 + 500);
                ids.push(rand() % 50);
            }
        }
    }
    let n = xs.len();
    let moving = vec![1u32; n];
    let toward = |a: u32, b: u32| if a < b { a + (b - a).min(300) } else { a - (a - b).min(300) };
    let mut contested = 0;
    for round in 0..40 {
        let cand_x: Vec<u32> = xs.iter().map(|&x| toward(x, 20_000)).collect();
        let cand_y: Vec<u32> = ys.iter().map(|&y| toward(y, 20_000)).collect();
        let (cx, cy) = cpu_round(&xs, &ys, &moving, &cand_x, &cand_y, &ids, &mask, w, h);
        let gpu = dev
            .round(&xs, &ys, &moving, &cand_x, &cand_y, &ids, &mask, w, h)
            .unwrap_or_else(|err| panic!("round {round}: {} {}", err.op, err.code));
        assert_eq!(gpu.out.error, 0, "round {round}");
        assert_eq!((&gpu.out.xs, &gpu.out.ys), (&cx, &cy), "round {round}");
        contested += (0..n).filter(|&i| (cx[i], cy[i]) == (xs[i], ys[i]) && (cand_x[i], cand_y[i]) != (xs[i], ys[i])).count();
        xs = cx;
        ys = cy;
    }
    assert!(n > 300, "crowd of {n}");
    assert!(contested > 1000, "only {contested} refused moves; the crowd is not contested");
}
