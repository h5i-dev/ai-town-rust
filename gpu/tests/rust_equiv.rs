//! Differential check: PTX compiled from `kernel`'s `seek_one` against `scalar_seek`.
//!
//! Skips when the PTX image is absent, so the cubin tests still pass on a
//! machine that has not run `gpu/nvvm/build-device.sh`. `AI_TOWN_REQUIRE_PTX=1`
//! turns that skip into a failure.

use ai_town_gpu::{Device, LayoutError, RustDevice, SeekArena, SeekOut, SeekRow, MAX_AGENTS};
use ai_town_kernel::{scalar_seek, set_blocked};
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

fn oracle(xs: &[u32], ys: &[u32], txs: &[u32], tys: &[u32], step: u32, mask: &[u8], width: u32, height: u32, n: usize) -> SeekOut {
    let (sx, sy) = scalar_seek(xs, ys, txs, tys, step, mask, width, height);
    let mut out_x = vec![0u32; n];
    let mut out_y = vec![0u32; n];
    let mut errors = vec![0u32; n];
    let mut i = 0;
    while i < n {
        if i >= MAX_AGENTS || i >= xs.len() || i >= ys.len() {
            errors[i] = 1;
        } else {
            out_x[i] = sx[i];
            out_y[i] = sy[i];
        }
        i += 1;
    }
    SeekOut { xs: out_x, ys: out_y, errors }
}

fn columns(n: usize, width: u32) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut txs = Vec::new();
    let mut tys = Vec::new();
    let mut i = 0u32;
    while (i as usize) < n {
        xs.push((i % width) * 1000 + (i % 5) * 10);
        ys.push((i / width) * 1000 + (i % 3) * 20);
        txs.push((i * 3) % width);
        tys.push((i * 5) % width.max(1));
        i += 1;
    }
    (xs, ys, txs, tys)
}

fn assert_gpu(dev: &RustDevice, xs: &[u32], ys: &[u32], txs: &[u32], tys: &[u32], step: u32, mask: &[u8], width: u32, height: u32, n: usize) {
    let cpu = oracle(xs, ys, txs, tys, step, mask, width, height, n);
    let gpu = dev.seek_n(xs, ys, txs, tys, step, mask, width, height, n).unwrap_or_else(|err| panic!("{} {}", err.op, err.code));
    assert_eq!(gpu, cpu, "n={n} step={step}");
}

#[test]
fn ptx_matches_scalar_seek() {
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

    assert_gpu(&dev, &[], &[], &[], &[], 200, &[], 8, 8, 0);

    let width = 64u32;
    let height = 80u32;
    let cells = (width as usize) * (height as usize);
    let open = vec![0u8; (cells - 1) / 8 + 1];
    let mut mask = open.clone();
    set_blocked(&mut mask, width, height, 1, 0);

    for n in [0usize, 1, 127, 128, 129, 1025] {
        let (xs, ys, txs, tys) = columns(n, width);
        for step in [0u32, 12, 200, 5_000] {
            assert_gpu(&dev, &xs, &ys, &txs, &tys, step, &mask, width, height, n);
            assert_gpu(&dev, &xs, &ys, &txs, &tys, step, &open, width, height, n);
        }
        if n > 0 {
            assert_gpu(&dev, &xs[..n - 1], &ys[..n - 1], &txs, &tys, 200, &mask, width, height, n);
        }
    }

    // Landing on the blocked tile (1, 0) is refused. A neighbor inside the
    // 750 milli radius refuses the step; one outside it does not.
    assert_gpu(&dev, &[0], &[0], &[2], &[0], 5_000, &mask, width, height, 1);
    assert_gpu(&dev, &[0, 700], &[0, 0], &[8, 0], &[0, 0], 200, &[], width, height, 2);
    assert_gpu(&dev, &[0, 3000], &[0, 0], &[8, 0], &[0, 0], 200, &[], width, height, 2);

    let (xs, ys, txs, tys) = columns(200, width);
    assert_gpu(&dev, &xs, &ys, &txs, &tys, 180, &mask, width, height, 200);
    let (xs, ys, txs, tys) = columns(5_000, width);
    assert_gpu(&dev, &xs, &ys, &txs, &tys, 180, &[], width, height, 5_000);

    let rows = vec![
        SeekRow { x: 0, y: 0, tx: 4, ty: 1 },
        SeekRow { x: 2000, y: 1000, tx: 0, ty: 2 },
    ];
    let arena = SeekArena::pack(&rows, 2).unwrap();
    assert_eq!(arena.unpack(), rows);
    assert_gpu(&dev, &arena.xs, &arena.ys, &arena.txs, &arena.tys, 250, &[], 16, 16, 2);
    assert_eq!(SeekArena::pack(&rows, MAX_AGENTS + 1).err(), Some(LayoutError::Capacity));

    let too_many = dev.seek_n(&[], &[], &[], &[], 12, &[], 4, 4, MAX_AGENTS + 1);
    assert_eq!(too_many.err().map(|err| err.op), Some("capacity"));

    // Hand-written cubin, on the same inputs, after this context is dropped.
    let (xs, ys, txs, tys) = columns(64, 16);
    let cpu = scalar_seek(&xs, &ys, &txs, &tys, 200, &[], 16, 16);
    let ptx_out = dev.seek(&xs, &ys, &txs, &tys, 200, &[], 16, 16).unwrap_or_else(|err| panic!("{} {}", err.op, err.code));
    drop(dev);
    let cubin = Device::open().unwrap_or_else(|err| panic!("cubin {}: {}", err.op, err.code));
    let cubin_out = cubin.seek(&xs, &ys, &txs, &tys, 200, &[], 16, 16).unwrap_or_else(|err| panic!("cubin seek {}: {}", err.op, err.code));
    assert_eq!(ptx_out.errors, vec![0u32; 64]);
    assert_eq!((ptx_out.xs, ptx_out.ys), cpu);
    assert_eq!(cubin_out, cpu);
}
