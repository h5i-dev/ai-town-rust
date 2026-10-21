//! Per-agent movement decision shared by the CPU tick and the PTX image.
//!
//! The tick asks [`path_pose`] for a candidate, then accepts it here.
//! Neighbors come from the one-tile spatial grid, not a scan of every agent.
//! Nothing in this module allocates. `tick_players` is host-only: it copies
//! positions into slices, runs [`tick_moves`], and writes the landings back.

use crate::geom::{self, grid_hit};

/// Where one agent stands after the decision. `moved` is false when the agent
/// keeps `(x, y)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landing {
    pub x: u32,
    pub y: u32,
    pub face_dx: u32,
    pub face_dy: u32,
    pub speed: u32,
    pub moved: bool,
}

/// Accept `cand` when `moving` is set, the landing tile is walkable, and the
/// spatial grid has no other agent inside the collision radius. Otherwise stay.
pub fn agent_step(
    x: u32,
    y: u32,
    face_dx: u32,
    face_dy: u32,
    moving: bool,
    cand_x: u32,
    cand_y: u32,
    cand_fx: u32,
    cand_fy: u32,
    cand_speed: u32,
    mask: &[u8],
    width: u32,
    height: u32,
    head: &[u32],
    next: &[u32],
    xs: &[u32],
    ys: &[u32],
    skip: u32,
) -> Landing {
    let tile = geom::mask_blocked(mask, width, height, cand_x, cand_y);
    let crowded = if tile { true } else { grid_hit(width, height, head, next, xs, ys, cand_x, cand_y, skip) };
    let accept = if crowded { false } else { moving };
    if accept {
        Landing { x: cand_x, y: cand_y, face_dx: cand_fx, face_dy: cand_fy, speed: cand_speed, moved: true }
    } else {
        Landing { x, y, face_dx, face_dy, speed: 0, moved: false }
    }
}

/// Apply [`agent_step`] in index order. Each accepted landing is visible to
/// later agents. `head` and `next` stay the chains built at the start of the
/// tick. Returns false when the slices disagree in length, and then writes
/// nothing.
pub fn tick_moves(
    xs: &mut [u32],
    ys: &mut [u32],
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    mask: &[u8],
    width: u32,
    height: u32,
    head: &[u32],
    next: &[u32],
) -> bool {
    let n = xs.len();
    let ready = ys.len() == n && moving.len() == n && cand_x.len() == n && cand_y.len() == n;
    if ready {
        let mut i = 0;
        while i < n {
            let land = agent_step(
                xs[i],
                ys[i],
                0,
                0,
                moving[i] != 0,
                cand_x[i],
                cand_y[i],
                0,
                0,
                0,
                mask,
                width,
                height,
                head,
                next,
                xs,
                ys,
                i as u32,
            );
            xs[i] = land.x;
            ys[i] = land.y;
            i += 1;
        }
    }
    ready
}

/// One movement pass over a snapshot. Members, conversations, ids, and owners
/// stay. Positions follow [`tick_moves`].
#[cfg(not(target_arch = "nvptx64"))]
pub fn tick_players(mut s: crate::Snapshot, moving: &[u32], cand_x: &[u32], cand_y: &[u32], head: &[u32], next: &[u32]) -> crate::Snapshot {
    let n = s.players.len();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut i = 0;
    while i < n {
        xs.push(s.players[i].x);
        ys.push(s.players[i].y);
        i += 1;
    }
    let _ = tick_moves(&mut xs, &mut ys, moving, cand_x, cand_y, &s.terrain.mask, s.meta.width, s.meta.height, head, next);
    i = 0;
    while i < n && i < xs.len() && i < ys.len() {
        s.players[i].x = xs[i];
        s.players[i].y = ys[i];
        i += 1;
    }
    s
}
