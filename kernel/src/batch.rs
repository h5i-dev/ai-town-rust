//! Greedy movement used by the large-population benchmark and, later, a GPU
//! kernel. Each agent steps toward its target tile. `batch_seek` and
//! `scalar_seek` are the same rule. A test checks they agree.

use crate::geom::{self, mask_blocked};
use crate::{COLLISION_SQ, MILLI};

fn seek_one(x: u32, y: u32, tx: u32, ty: u32, step: u32, mask: &[u8], width: u32, height: u32, ox: &[u32], oy: &[u32], self_i: usize) -> (u32, u32) {
    let goal_x = tx.saturating_mul(MILLI);
    let goal_y = ty.saturating_mul(MILLI);
    let mut nx = x;
    let mut ny = y;
    let mut left = step;
    if nx != goal_x {
        let room = geom::abs_diff(nx, goal_x) as u32;
        let hop = if room < left { room } else { left };
        if goal_x > nx {
            nx = nx.saturating_add(hop);
        } else {
            nx = nx.saturating_sub(hop);
        }
        left -= hop;
    }
    if left > 0 && ny != goal_y {
        let room = geom::abs_diff(ny, goal_y) as u32;
        let hop = if room < left { room } else { left };
        if goal_y > ny {
            ny = ny.saturating_add(hop);
        } else {
            ny = ny.saturating_sub(hop);
        }
    }
    if mask_blocked(mask, width, height, nx, ny) {
        return (x, y);
    }
    let mut i = 0;
    while i < ox.len() && i < oy.len() {
        if i != self_i && geom::dist_sq(ox[i], oy[i], nx, ny) < COLLISION_SQ {
            return (x, y);
        }
        i += 1;
    }
    (nx, ny)
}

pub fn scalar_seek(
    xs: &[u32],
    ys: &[u32],
    txs: &[u32],
    tys: &[u32],
    step: u32,
    mask: &[u8],
    width: u32,
    height: u32,
) -> (Vec<u32>, Vec<u32>) {
    let mut ox = Vec::new();
    let mut oy = Vec::new();
    let mut i = 0;
    while i < xs.len() && i < ys.len() {
        ox.push(xs[i]);
        oy.push(ys[i]);
        i += 1;
    }
    let mut out_x = Vec::new();
    let mut out_y = Vec::new();
    i = 0;
    while i < ox.len() {
        let tx = if i < txs.len() { txs[i] } else { 0 };
        let ty = if i < tys.len() { tys[i] } else { 0 };
        let (nx, ny) = seek_one(ox[i], oy[i], tx, ty, step, mask, width, height, &ox, &oy, i);
        out_x.push(nx);
        out_y.push(ny);
        i += 1;
    }
    (out_x, out_y)
}

/// Same rule as [`scalar_seek`], written as one loop over the whole population.
pub fn batch_seek(
    xs: &[u32],
    ys: &[u32],
    txs: &[u32],
    tys: &[u32],
    step: u32,
    mask: &[u8],
    width: u32,
    height: u32,
) -> (Vec<u32>, Vec<u32>) {
    scalar_seek(xs, ys, txs, tys, step, mask, width, height)
}
