//! Device image for the movement decision in `kernel/src/step.rs`.
//!
//! This crate is `no_std` and is built only for `nvptx64-nvidia-cuda` with
//! `rustc_codegen_nvvm`. It includes the kernel sources with `#[path]`.
//! `step_batch` runs `step::tick_moves` on thread 0, the same sequential
//! accept/refuse the CPU tick uses. `seek_batch` stays as the greedy baseline.

#![cfg(feature = "gpu")]
#![allow(improper_ctypes_definitions)]
#![allow(dead_code)]
#![allow(unused_imports)]
// `build-device.sh` passes `-Zcrate-attr=no_std`. A second `#![no_std]` here warns.

pub const MILLI: u32 = 1000;
pub const COLLISION_SQ: u64 = 750 * 750;
pub const FACE_BIAS: u32 = 1000;
pub const SPEED: u32 = 750;

#[path = "../../../../kernel/src/geom.rs"]
mod geom;
#[path = "../../../../kernel/src/batch.rs"]
mod batch;
#[path = "../../../../kernel/src/step.rs"]
mod step;
#[path = "../../../src/layout.rs"]
mod layout;

use cuda_std::{kernel, thread};
use layout::MAX_AGENTS;

/// One thread per agent. `xs`/`ys` are both the current positions and the
/// original positions `seek_one` tests collisions against.
///
/// Error 0 is success. Error 1 means the row is missing or past [`MAX_AGENTS`].
/// A successful step that stays put still writes error 0.
#[kernel]
pub unsafe fn seek_batch(
    xs: &[u32],
    ys: &[u32],
    txs: &[u32],
    tys: &[u32],
    mask: &[u8],
    step: u32,
    width: u32,
    height: u32,
    out_x: *mut u32,
    out_y: *mut u32,
    errors: *mut u32,
    n: usize,
) {
    let i = thread::index_1d() as usize;
    if i < n {
        let (nx, ny, error) = if i >= MAX_AGENTS || i >= xs.len() || i >= ys.len() {
            (0, 0, 1u32)
        } else {
            let tx = if i < txs.len() { txs[i] } else { 0 };
            let ty = if i < tys.len() { tys[i] } else { 0 };
            let (nx, ny) = batch::seek_one(xs[i], ys[i], tx, ty, step, mask, width, height, xs, ys, i);
            (nx, ny, 0u32)
        };
        unsafe {
            *out_x.add(i) = nx;
            *out_y.add(i) = ny;
            *errors.add(i) = error;
        }
    }
}

/// One thread runs the whole tick, in index order, so a later agent sees an
/// earlier landing. Error 0 is success. Error 1 means `n` is past
/// [`MAX_AGENTS`] or a position slice is shorter than `n`.
#[kernel]
pub unsafe fn step_batch(
    xs: *mut u32,
    xs_len: usize,
    ys: *mut u32,
    ys_len: usize,
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    mask: &[u8],
    head: &[u32],
    next: &[u32],
    width: u32,
    height: u32,
    error: *mut u32,
    n: usize,
) {
    if thread::index_1d() == 0 {
        let short = n > MAX_AGENTS || xs_len < n || ys_len < n || moving.len() < n || cand_x.len() < n || cand_y.len() < n;
        let code = if short {
            1u32
        } else {
            let xs_all = unsafe { core::slice::from_raw_parts_mut(xs, xs_len) };
            let ys_all = unsafe { core::slice::from_raw_parts_mut(ys, ys_len) };
            let ok = step::tick_moves(
                &mut xs_all[..n],
                &mut ys_all[..n],
                &moving[..n],
                &cand_x[..n],
                &cand_y[..n],
                mask,
                width,
                height,
                head,
                next,
            );
            if ok { 0 } else { 1 }
        };
        unsafe {
            *error = code;
        }
    }
}
