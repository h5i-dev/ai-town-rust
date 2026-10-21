//! GPU seek on a thread that owns the CUDA context.
//!
//! One sim second is 60 calls of `seek_one` with a 12 milli-tile step, the
//! same per-tick budget as `SPEED * TICK_MS / 1000`. Players who are not
//! already walking keep their positions and stay in the collision set.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{Snapshot, MILLI, PF_MOVE};
use std::path::PathBuf;
use tokio::sync::oneshot;

pub struct MoveReq {
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub txs: Vec<u32>,
    pub tys: Vec<u32>,
    pub moving: Vec<bool>,
    pub mask: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub reply: oneshot::Sender<Option<(Vec<u32>, Vec<u32>)>>,
}

pub fn spawn_mover() -> std::sync::mpsc::Sender<MoveReq> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("gpu-seek".into()).spawn(move || mover_loop(rx)).ok();
    tx
}

pub fn columns(snap: &Snapshot) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<bool>) {
    let n = snap.players.len();
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut txs = Vec::with_capacity(n);
    let mut tys = Vec::with_capacity(n);
    let mut moving = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let p = &snap.players[i];
        xs.push(p.x);
        ys.push(p.y);
        txs.push(p.dest_x / MILLI);
        tys.push(p.dest_y / MILLI);
        moving.push(p.pf == PF_MOVE);
        i += 1;
    }
    (xs, ys, txs, tys, moving)
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
            tracing::error!("gpu seek unavailable");
            while let Ok(job) = rx.recv() {
                let _ = job.reply.send(None);
            }
            return;
        }
    };
    while let Ok(job) = rx.recv() {
        let MoveReq { xs, ys, txs, tys, moving, mask, width, height, reply } = job;
        let _ = reply.send(seek_minute(&dev, xs, ys, txs, tys, moving, &mask, width, height));
    }
}

fn seek_minute(
    dev: &RustDevice,
    mut xs: Vec<u32>,
    mut ys: Vec<u32>,
    txs: Vec<u32>,
    tys: Vec<u32>,
    moving: Vec<bool>,
    mask: &[u8],
    width: u32,
    height: u32,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let n = xs.len();
    if n == 0 || ys.len() != n || moving.len() != n {
        return None;
    }
    let mut round = 0u32;
    while round < 60 {
        let out = dev.seek(&xs, &ys, &txs, &tys, 12, mask, width, height).ok()?;
        if out.errors.len() != n || out.xs.len() != n || out.ys.len() != n {
            return None;
        }
        let mut i = 0;
        while i < n {
            if out.errors[i] != 0 {
                return None;
            }
            if moving[i] {
                xs[i] = out.xs[i];
                ys[i] = out.ys[i];
            }
            i += 1;
        }
        round += 1;
    }
    Some((xs, ys))
}
