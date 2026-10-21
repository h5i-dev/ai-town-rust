//! Differential check: PTX `step_batch` against host `tick_moves`.
//!
//! Both sides use the spatial grid built at the start of the tick. The device
//! runs that tick on one thread, so a later agent sees an earlier landing.
//! Skips when the PTX image is absent. `AI_TOWN_REQUIRE_PTX=1` turns the skip
//! into a failure. `seek_one` remains a one-row baseline in the same image.

use ai_town_gpu::{LayoutError, RustDevice, SeekArena, SeekRow, StepOut, MAX_AGENTS};
use ai_town_kernel::{near_index, scalar_seek, set_blocked, tick_moves};
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

fn cpu_tick(
    xs: &[u32],
    ys: &[u32],
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    mask: &[u8],
    width: u32,
    height: u32,
) -> (Vec<u32>, Vec<u32>) {
    let g = near_index(width, height, xs, ys);
    let mut xs = xs.to_vec();
    let mut ys = ys.to_vec();
    assert!(tick_moves(&mut xs, &mut ys, moving, cand_x, cand_y, mask, width, height, &g.head, &g.next));
    (xs, ys)
}

fn assert_step(
    dev: &RustDevice,
    xs: &[u32],
    ys: &[u32],
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    mask: &[u8],
    width: u32,
    height: u32,
) {
    let g = near_index(width, height, xs, ys);
    let (cx, cy) = cpu_tick(xs, ys, moving, cand_x, cand_y, mask, width, height);
    let gpu = dev
        .step(xs, ys, moving, cand_x, cand_y, mask, width, height, &g.head, &g.next)
        .unwrap_or_else(|err| panic!("{} {}", err.op, err.code));
    assert_eq!(gpu.out.error, 0, "device error");
    assert_eq!(StepOut { xs: gpu.out.xs, ys: gpu.out.ys, error: 0 }, StepOut { xs: cx, ys: cy, error: 0 });
}

#[test]
fn ptx_matches_tick_moves() {
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

    let empty = dev.step(&[], &[], &[], &[], &[], &[], 8, 8, &[], &[]).unwrap_or_else(|err| panic!("{} {}", err.op, err.code));
    assert_eq!(empty.out, StepOut { xs: Vec::new(), ys: Vec::new(), error: 0 });

    let open = open_mask(8, 8);
    // The second agent wants 800. After the first lands on 200 the gap is 600,
    // inside the radius, so the second stays at 1000.
    assert_step(&dev, &[0, 1000], &[0, 0], &[1, 1], &[200, 800], &[0, 0], &open, 8, 8);

    let mut wall = open_mask(8, 8);
    set_blocked(&mut wall, 8, 8, 1, 0);
    assert_step(&dev, &[0], &[0], &[1], &[1000], &[0], &wall, 8, 8);

    assert_step(&dev, &[0, 1000], &[0, 0], &[1, 0], &[500, 1000], &[0, 0], &open, 8, 8);
    assert_step(&dev, &[0, 3000], &[0, 0], &[1, 0], &[500, 3000], &[0, 0], &open, 8, 8);
    assert_step(&dev, &[0], &[0], &[0], &[500], &[0], &open, 8, 8);

    let n = 128usize;
    let width = 16u32;
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut moving = Vec::new();
    let mut cand_x = Vec::new();
    let mut cand_y = Vec::new();
    let mut i = 0usize;
    while i < n {
        let x = ((i % width as usize) as u32) * 1000;
        let y = ((i / width as usize) as u32) * 1000;
        xs.push(x);
        ys.push(y);
        moving.push(1);
        cand_x.push(x + 12);
        cand_y.push(y);
        i += 1;
    }
    let lattice = open_mask(width, 8);
    assert_step(&dev, &xs, &ys, &moving, &cand_x, &cand_y, &lattice, width, 8);

    let mismatch = dev.step(&[0, 1], &[0], &[1, 1], &[0, 1], &[0, 0], &open, 8, 8, &[], &[]);
    assert_eq!(mismatch.err().map(|err| err.op), Some("shape"));
    let big = vec![0u32; MAX_AGENTS + 1];
    let too_many = dev.step(&big, &big, &big, &big, &big, &[], 4, 4, &[], &[]);
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
}
