//! Device image for the movement decision in `kernel/src/step.rs`.
//!
//! This crate is `no_std` and is built only for `nvptx64-nvidia-cuda` with
//! `rustc_codegen_nvvm`. It includes the kernel sources with `#[path]`.
//! `round_phase` runs the two-phase tick: one thread per agent proposes from
//! the previous positions, then one thread per agent accepts or yields.
//! The host rebuilds the one-tile grid between those launches. `step_batch`
//! keeps the sequential `tick_moves` baseline. `seek_batch` stays the greedy
//! full-scan baseline.

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

/// Swap `*addr` with `val` and return the previous word.
///
/// `cuda_std` in this pin only exposes floating-point atomics. The exchange
/// is a device-wide atomic, so concurrent pushes into one cell stay a list.
/// The thread that owns index `i` is the only writer of `next[i]`.
unsafe fn atomic_exch_u32(addr: *mut u32, val: u32) -> u32 {
    let mut out: u32;
    unsafe {
        core::arch::asm!(
            "atom.global.exch.b32 {}, [{}], {};",
            out(reg32) out,
            in(reg64) addr,
            in(reg32) val,
        );
    }
    out
}

/// Push agent `i` onto the cell that contains `(xs[i], ys[i])`.
///
/// Out-of-map agents stay unlinked. The chain order differs from the
/// sequential `grid_build` inserts. Membership matches, and `grid_hit` /
/// `grid_yield` only care about the set.
unsafe fn insert_cell(head: *mut u32, next: *mut u32, xs: *const u32, ys: *const u32, i: usize, width: u32, height: u32) {
    let x = unsafe { *xs.add(i) };
    let y = unsafe { *ys.add(i) };
    let tx = x / MILLI;
    let ty = y / MILLI;
    if tx >= width || ty >= height {
        return;
    }
    let cell = (tx as usize) * (height as usize) + (ty as usize);
    let cells = (width as usize) * (height as usize);
    if cell >= cells {
        return;
    }
    let old = unsafe { atomic_exch_u32(head.add(cell), i as u32) };
    unsafe {
        *next.add(i) = old;
    }
}

/// One phase of a two-phase round. `phase` 0 inserts every agent at `(xs, ys)`.
/// Phase 1 writes proposals from the previous positions. Phase 2 inserts agents
/// whose `prop_on` is set, at the proposal. Phase 3 accepts or yields.
/// `cand_*` point at this round's rows. A zero `cand_on` uses the current position.
///
/// Launches in one stream are the barrier between phases. There is no
/// software grid barrier.
#[kernel]
pub unsafe fn round_phase(
    xs: *mut u32,
    ys: *mut u32,
    moving: *const u32,
    cand_x: *const u32,
    cand_y: *const u32,
    cand_on: *const u32,
    ids: *const u64,
    mask: *const u8,
    mask_len: usize,
    head: *mut u32,
    next: *mut u32,
    prop_x: *mut u32,
    prop_y: *mut u32,
    prop_on: *mut u32,
    width: u32,
    height: u32,
    phase: u32,
    n: usize,
) {
    let i = thread::index_1d() as usize;
    if i >= n || n > MAX_AGENTS || width == 0 || height == 0 {
        return;
    }
    let cells = (width as usize) * (height as usize);
    unsafe {
        if phase == 0 {
            insert_cell(head, next, xs, ys, i, width, height);
            return;
        }
        if phase == 2 {
            if *prop_on.add(i) != 0 {
                insert_cell(head, next, prop_x, prop_y, i, width, height);
            }
            return;
        }
        if phase == 1 {
            let xs_s = core::slice::from_raw_parts(xs as *const u32, n);
            let ys_s = core::slice::from_raw_parts(ys as *const u32, n);
            let head_s = core::slice::from_raw_parts(head as *const u32, cells);
            let next_s = core::slice::from_raw_parts(next as *const u32, n);
            let mask_s = core::slice::from_raw_parts(mask, mask_len);
            let use_cand = *cand_on.add(i) != 0;
            let cx = if use_cand { *cand_x.add(i) } else { xs_s[i] };
            let cy = if use_cand { *cand_y.add(i) } else { ys_s[i] };
            let proposal = step::propose_one(
                xs_s[i],
                ys_s[i],
                0,
                0,
                *moving.add(i) != 0,
                cx,
                cy,
                0,
                0,
                0,
                mask_s,
                width,
                height,
                head_s,
                next_s,
                xs_s,
                ys_s,
                i as u32,
            );
            *prop_x.add(i) = proposal.x;
            *prop_y.add(i) = proposal.y;
            *prop_on.add(i) = if proposal.proposed { 1 } else { 0 };
            return;
        }
        if phase == 3 {
            let x = *xs.add(i);
            let y = *ys.add(i);
            let prop_x_s = core::slice::from_raw_parts(prop_x as *const u32, n);
            let prop_y_s = core::slice::from_raw_parts(prop_y as *const u32, n);
            let ids_s = core::slice::from_raw_parts(ids, n);
            let head_s = core::slice::from_raw_parts(head as *const u32, cells);
            let next_s = core::slice::from_raw_parts(next as *const u32, n);
            let land = step::resolve_one(
                x,
                y,
                0,
                0,
                *prop_x.add(i),
                *prop_y.add(i),
                0,
                0,
                0,
                *prop_on.add(i) != 0,
                *ids.add(i),
                head_s,
                next_s,
                prop_x_s,
                prop_y_s,
                ids_s,
                i as u32,
                width,
                height,
            );
            *xs.add(i) = land.x;
            *ys.add(i) = land.y;
            *prop_on.add(i) = if land.moved { 1 } else { 0 };
        }
    }
}
