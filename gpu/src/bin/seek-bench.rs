//! CPU scalar seek versus the CUDA seek. CSV goes under `/dev/shm`.

use ai_town_gpu::Device;
use ai_town_kernel::scalar_seek;
use std::time::Instant;

fn main() {
    let dev = Device::open().unwrap_or_else(|err| panic!("cuda {}: {}", err.op, err.code));
    let dir = std::env::var("AI_TOWN_BENCH_DIR").unwrap_or_else(|_| "/dev/shm/ai-town-rust/bench".into());
    std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("create {dir}: {err}"));
    let mut rows = vec!["agents,cpu_ms,gpu_ms,cpu_seeks_per_s,gpu_seeks_per_s".to_string()];
    for n in [1_000usize, 10_000, 50_000] {
        let (xs, ys, txs, tys, mask, width, height) = world(n);
        let cpu_started = Instant::now();
        let (cx, cy) = scalar_seek(&xs, &ys, &txs, &tys, 200, &mask, width, height);
        let cpu_ms = cpu_started.elapsed().as_secs_f64() * 1_000.0;
        let _ = dev
            .seek(&xs, &ys, &txs, &tys, 200, &mask, width, height)
            .unwrap_or_else(|err| panic!("warmup {}: {}", err.op, err.code));
        let gpu_started = Instant::now();
        let (gx, gy) = dev
            .seek(&xs, &ys, &txs, &tys, 200, &mask, width, height)
            .unwrap_or_else(|err| panic!("seek {}: {}", err.op, err.code));
        let gpu_ms = gpu_started.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(cx, gx, "x mismatch at {n}");
        assert_eq!(cy, gy, "y mismatch at {n}");
        let cpu_rate = n as f64 / (cpu_ms / 1_000.0).max(1e-9);
        let gpu_rate = n as f64 / (gpu_ms / 1_000.0).max(1e-9);
        let row = format!("{n},{cpu_ms:.3},{gpu_ms:.3},{cpu_rate:.1},{gpu_rate:.1}");
        println!("gpu agents={n} cpu_ms={cpu_ms:.2} gpu_ms={gpu_ms:.2} gpu_seeks/s={gpu_rate:.0}");
        rows.push(row);
    }
    let path = format!("{dir}/gpu.csv");
    std::fs::write(&path, rows.join("\n") + "\n").unwrap_or_else(|err| panic!("write {path}: {err}"));
    println!("wrote {path}");
}

fn world(n: usize) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u8>, u32, u32) {
    let side = 224u32;
    let cells = (side as usize) * (side as usize);
    let mut mask = vec![0u8; (cells - 1) / 8 + 1];
    // A stripe of blocked tiles, so some steps are rejected.
    let mut tx = 3u32;
    while tx < side {
        let i = (tx as usize) * (side as usize);
        let byte = i / 8;
        let bit = i % 8;
        if byte < mask.len() {
            mask[byte] |= 1 << bit;
        }
        tx += 17;
    }
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut txs = Vec::with_capacity(n);
    let mut tys = Vec::with_capacity(n);
    let mut i = 0usize;
    while i < n {
        xs.push(((i % side as usize) as u32) * 1000 + 120);
        ys.push(((i / side as usize) as u32) * 1000 + 80);
        txs.push(((i * 3) % side as usize) as u32);
        tys.push(((i * 5) % side as usize) as u32);
        i += 1;
    }
    (xs, ys, txs, tys, mask, side, side)
}
