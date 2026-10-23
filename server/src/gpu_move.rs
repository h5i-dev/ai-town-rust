//! GPU movement on a thread that owns the CUDA context.
//!
//! One sim second is 60 two-phase rounds. Candidates come from `path_pose` and
//! are copied once. The device rebuilds the spatial grid from the positions of
//! the previous round and runs one thread per agent for propose and for
//! resolve. Players who are not already walking stay where they are. A refused
//! step is retried from the later path pose. The CPU tick would instead enter
//! `PF_WAIT` for about a second. This pass does not advance `now_ms`,
//! conversations, or facing. `Relocate` writes the positions and clears paths.
//!
//! Paths are stamped with the command's wall clock. `meta.now_ms` lags that
//! clock because this pass does not advance it. Sampling only the lagging
//! clock stays before the path, every pose is the start point, and `Relocate`
//! then deletes the path. The minute is therefore anchored so its last sample
//! is the wall clock when that is later than `meta.now_ms`.

use ai_town_gpu::RustDevice;
use ai_town_kernel::{path_pose, PathNode, Snapshot, PF_MOVE, TICK_MS};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::oneshot;

pub struct MoveReq {
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub moving: Vec<u32>,
    pub ids: Vec<u64>,
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
    let mut ids = Vec::with_capacity(n);
    let mut slot = HashMap::with_capacity(n);
    let mut i = 0;
    while i < n {
        let p = &snap.players[i];
        xs.push(p.x);
        ys.push(p.y);
        moving.push(if p.pf == PF_MOVE { 1 } else { 0 });
        ids.push(p.id);
        slot.insert(p.id, i);
        i += 1;
    }
    // Count and prefix-pack once; avoid one heap allocation per resident and
    // the second clone of every path row. Preserve original order per player.
    let mut counts = vec![0usize; n];
    for node in &snap.paths {
        if let Some(&i) = slot.get(&node.player) { counts[i] += 1; }
    }
    let mut path_lo = Vec::with_capacity(n);
    let mut path_hi = Vec::with_capacity(n);
    let mut offsets = Vec::with_capacity(n);
    let mut total = 0;
    for count in counts {
        offsets.push(total);
        path_lo.push(total as u32);
        total += count;
        path_hi.push(total as u32);
    }
    let mut ordered = vec![None; total];
    for node in &snap.paths {
        if let Some(&i) = slot.get(&node.player) {
            ordered[offsets[i]] = Some(node);
            offsets[i] += 1;
        }
    }
    let paths = ordered.into_iter().map(|node| node.unwrap().clone()).collect();
    MoveReq {
        xs,
        ys,
        moving,
        ids,
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
    let mut dev = match RustDevice::open_path(&ptx_path()) {
        Ok(dev) => dev,
        Err(_) => {
            tracing::error!("gpu step unavailable");
            while let Ok(job) = rx.recv() {
                let _ = job.reply.send(None);
            }
            return;
        }
    };
    // `AI_TOWN_GPU_VERIFY=1` checks every device-built grid with the kernel's
    // `grid_wellformed`. A refused grid fails the batch like any device error.
    dev.set_verify_grid(std::env::var("AI_TOWN_GPU_VERIFY").ok().as_deref() == Some("1"));
    while let Ok(job) = rx.recv() {
        let MoveReq { xs, ys, moving, ids, paths, path_lo, path_hi, now_ms, mask, width, height, reply } = job;
        let _ = reply.send(step_minute(&dev, xs, ys, moving, ids, paths, path_lo, path_hi, now_ms, &mask, width, height));
    }
}

fn step_minute(
    dev: &RustDevice,
    xs: Vec<u32>,
    ys: Vec<u32>,
    moving: Vec<u32>,
    ids: Vec<u64>,
    paths: Vec<PathNode>,
    path_lo: Vec<u32>,
    path_hi: Vec<u32>,
    now_ms: u64,
    mask: &[u8],
    width: u32,
    height: u32,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let n = xs.len();
    if n == 0 || ys.len() != n || moving.len() != n || ids.len() != n || path_lo.len() != n || path_hi.len() != n {
        return None;
    }
    let rounds = 60usize;
    let origin = minute_origin(now_ms);
    let mut cand_x = vec![0u32; n * rounds];
    let mut cand_y = vec![0u32; n * rounds];
    let mut cand_on = vec![0u32; n * rounds];
    let mut round = 0usize;
    while round < rounds {
        let t = origin.wrapping_add(TICK_MS.wrapping_mul((round as u64).wrapping_add(1)));
        let base = round * n;
        let mut i = 0;
        while i < n {
            if moving[i] != 0 {
                let lo = path_lo[i] as usize;
                let hi = path_hi[i] as usize;
                if lo <= hi && hi <= paths.len() {
                    if let Some(pose) = path_pose(&paths[lo..hi], t) {
                        cand_x[base + i] = pose.x;
                        cand_y[base + i] = pose.y;
                        cand_on[base + i] = 1;
                    }
                }
            } else {
                cand_x[base + i] = xs[i];
                cand_y[base + i] = ys[i];
                cand_on[base + i] = 1;
            }
            i += 1;
        }
        round += 1;
    }
    let report = dev.rounds(&xs, &ys, &moving, &cand_x, &cand_y, &cand_on, &ids, mask, width, height, 60).ok()?;
    if report.out.error != 0 || report.out.xs.len() != n || report.out.ys.len() != n {
        return None;
    }
    Some((report.out.xs, report.out.ys))
}

/// Base time for the 60 samples. The last sample is this value plus 960ms.
/// `meta.now_ms` wins when it is already later than the wall-clock window.
fn minute_origin(now_ms: u64) -> u64 {
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(now_ms))
        .unwrap_or(now_ms);
    let window = wall.saturating_sub(TICK_MS.saturating_mul(60));
    if now_ms > window { now_ms } else { window }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_packing_preserves_resident_and_node_order() {
        let mut snap = Snapshot::default();
        let principal = ai_town_kernel::Principal { org: 1, user: 0, now_ms: 1000 };
        let (writes, _) = ai_town_kernel::transition(&principal, &snap, &ai_town_kernel::Command::Init {
            width: 8, height: 8, mask: vec![0; 8],
        }).unwrap();
        snap = ai_town_kernel::apply(&snap, &writes);
        for user in [10, 20, 30] {
            let actor = ai_town_kernel::Principal { user, ..principal };
            let (writes, _) = ai_town_kernel::transition(&actor, &snap, &ai_town_kernel::Command::Join { name: vec![b'x'], character: 0 }).unwrap();
            snap = ai_town_kernel::apply(&snap, &writes);
        }
        let ids: Vec<_> = snap.players.iter().map(|p| p.id).collect();
        let node = |player, seq| PathNode { player, seq, x: 0, y: 0, face_dx: 1000, face_dy: 1000, t: seq };
        snap.paths = vec![node(ids[1], 3), node(ids[0], 8), node(9999, 0), node(ids[1], 1)];
        let (reply, _) = oneshot::channel();
        let packed = from_snapshot(&snap, reply);
        assert_eq!(packed.path_lo, vec![0, 1, 3]);
        assert_eq!(packed.path_hi, vec![1, 3, 3]);
        assert_eq!(packed.paths, vec![node(ids[0], 8), node(ids[1], 3), node(ids[1], 1)]);
    }
}
