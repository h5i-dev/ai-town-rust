//! Device image for the seek decision in `kernel/src/batch.rs`.
//!
//! This crate is `no_std` and is built only for `nvptx64-nvidia-cuda` with
//! `rustc_codegen_nvvm`. It includes the kernel sources with `#[path]` so the
//! GPU runs the same `seek_one` the host extracts to Lean.

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
