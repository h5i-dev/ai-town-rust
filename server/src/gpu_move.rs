//! GPU movement on a thread that owns the CUDA context.
//!
//! One sim second is 60 sequential calls of `tick_moves`. Each round asks
//! `path_pose` for a candidate at `now + 16ms * round`, rebuilds the spatial
//! grid from the latest positions, and launches one device thread. Players
//! who are not already walking stay where they are and stay in the grid.
//! A refused step is retried from the later path pose. The CPU tick would
//! instead enter `PF_WAIT` for about a second. This pass does not advance
//! `now_ms`, conversations, or facing. `Relocate` writes the positions.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{near_index, path_pose, PathNode, Snapshot, PF_MOVE, TICK_MS};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::oneshot;

pub struct MoveReq {
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub moving: Vec<u32>,
    pub paths: Vec<PathNode>,
    pub path_lo: Vec<u32>,
    pub path_hi: Vec<u32>,
    pub now_ms: u64,
    pub mask: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub reply: oneshot::Sender<Option<(Vec<u32>, Vec<u32>)>>,
}

pub fn from_snapshot(snap: &Snapshot, reply: oneshot::Sender<Option<(Vec<u32>, Vec<u32>)>>) -> MoveReq {
    let n = snap.players.len();
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut moving = Vec::with_capacity(n);
    let mut slot = HashMap::with_capacity(n);
    let mut i = 0;
    while i < n {
        let p = &snap.players[i];
        xs.push(p.x);
        ys.push(p.y);
        moving.push(if p.pf == PF_MOVE { 1 } else { 0 });
        slot.insert(p.id, i);
        i += 1;
    }
    let mut grouped = Vec::new();
    i = 0;
    while i < n {
        grouped.push(Vec::new());
        i += 1;
    }
    let mut j = 0;
    while j < snap.paths.len() {
        if let Some(&i) = slot.get(&snap.paths[j].player) {
            grouped[i].push(snap.paths[j].clone());
        }
        j += 1;
    }
    let mut paths = Vec::new();
    let mut path_lo = Vec::with_capacity(n);
    let mut path_hi = Vec::with_capacity(n);
    i = 0;
    while i < n {
        path_lo.push(paths.len() as u32);
        let mut k = 0;
        while k < grouped[i].len() {
            paths.push(grouped[i][k].clone());
            k += 1;
        }
        path_hi.push(paths.len() as u32);
        i += 1;
    }
    MoveReq {
        xs,
        ys,
        moving,
        paths,
        path_lo,
        path_hi,
        now_ms: snap.meta.now_ms,
        mask: snap.terrain.mask.clone(),
        width: snap.meta.width,
        height: snap.meta.height,
        reply,
    }
}

pub fn spawn_mover() -> std::sync::mpsc::Sender<MoveReq> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("gpu-step".into()).spawn(move || mover_loop(rx)).ok();
    tx
}

fn ptx_path() -> PathBuf {
    match std::env::var("AI_TOWN_PTX") {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from("/dev/shm/ai-town-rust/nvvm-target/nvptx64-nvidia-cuda/release/ai_town_seek_device.ptx"),
    }
}

fn mover_loop(rx: std::sync::mpsc::Receiver<MoveReq>) {
    let dev = match RustDevice::open_path(&ptx_path()) {
        Ok(dev) => dev,
        Err(_) => {
            tracing::error!("gpu step unavailable");
            while let Ok(job) = rx.recv() {
                let _ = job.reply.send(None);
            }
            return;
        }
    };
    while let Ok(job) = rx.recv() {
        let MoveReq { xs, ys, moving, paths, path_lo, path_hi, now_ms, mask, width, height, reply } = job;
        let _ = reply.send(step_minute(&dev, xs, ys, moving, paths, path_lo, path_hi, now_ms, &mask, width, height));
    }
}

fn step_minute(
    dev: &RustDevice,
    mut xs: Vec<u32>,
    mut ys: Vec<u32>,
    moving: Vec<u32>,
    paths: Vec<PathNode>,
    path_lo: Vec<u32>,
    path_hi: Vec<u32>,
    now_ms: u64,
    mask: &[u8],
    width: u32,
    height: u32,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let n = xs.len();
    if n == 0 || ys.len() != n || moving.len() != n || path_lo.len() != n || path_hi.len() != n {
        return None;
    }
    let mut cand_x = vec![0u32; n];
    let mut cand_y = vec![0u32; n];
    let mut round = 0u32;
    while round < 60 {
        let t = now_ms.wrapping_add(TICK_MS.wrapping_mul(u64::from(round) + 1));
        let mut i = 0;
        while i < n {
            if moving[i] != 0 {
                let lo = path_lo[i] as usize;
                let hi = path_hi[i] as usize;
                if lo <= hi && hi <= paths.len() {
                    if let Some(pose) = path_pose(&paths[lo..hi], t) {
                        cand_x[i] = pose.x;
                        cand_y[i] = pose.y;
                    } else {
                        cand_x[i] = xs[i];
                        cand_y[i] = ys[i];
                    }
                } else {
                    cand_x[i] = xs[i];
                    cand_y[i] = ys[i];
                }
            } else {
                cand_x[i] = xs[i];
                cand_y[i] = ys[i];
            }
            i += 1;
        }
        let g = near_index(width, height, &xs, &ys);
        let report = dev.step(&xs, &ys, &moving, &cand_x, &cand_y, &mask, width, height, &g.head, &g.next).ok()?;
        if report.out.error != 0 || report.out.xs.len() != n || report.out.ys.len() != n {
            return None;
        }
        xs = report.out.xs;
        ys = report.out.ys;
        round += 1;
    }
    Some((xs, ys))
}
