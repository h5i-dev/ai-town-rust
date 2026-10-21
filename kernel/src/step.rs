//! Movement decisions shared by the CPU tick and the PTX image.
//!
//! [`tick_moves`] is the original sequential rule: each agent sees landings
//! already accepted earlier in the tick. [`tick_round`] is the default. Every
//! agent proposes from the previous positions only, then a candidate is kept
//! when no better-priority proposal (lower player id, then lower index) lies
//! inside the collision radius. Rejected agents stay put. Neighbors come from
//! the one-tile spatial grid. Nothing here allocates. The snapshot wrappers
//! are host-only.

use crate::geom::{self, grid_build, grid_hit, grid_yield};
use crate::COLLISION_SQ;

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

/// A candidate pose. `proposed` is false when the agent keeps `(x, y)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub x: u32,
    pub y: u32,
    pub face_dx: u32,
    pub face_dy: u32,
    pub speed: u32,
    pub proposed: bool,
}

/// Propose `cand` from the previous positions. The tile must be open and the
/// spatial grid must show nobody else inside the radius. Otherwise stay.
pub fn propose_one(
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
) -> Proposal {
    let tile = geom::mask_blocked(mask, width, height, cand_x, cand_y);
    let crowded = if tile { true } else { grid_hit(width, height, head, next, xs, ys, cand_x, cand_y, skip) };
    let accept = if crowded { false } else { moving };
    if accept {
        Proposal { x: cand_x, y: cand_y, face_dx: cand_fx, face_dy: cand_fy, speed: cand_speed, proposed: true }
    } else {
        Proposal { x, y, face_dx, face_dy, speed: 0, proposed: false }
    }
}

/// Keep a proposal when no better-priority proposal in the grid is inside the
/// radius. Better means a lower player id, then a lower index.
pub fn resolve_one(
    x: u32,
    y: u32,
    face_dx: u32,
    face_dy: u32,
    cand_x: u32,
    cand_y: u32,
    cand_fx: u32,
    cand_fy: u32,
    cand_speed: u32,
    proposed: bool,
    self_id: u64,
    head: &[u32],
    next: &[u32],
    prop_x: &[u32],
    prop_y: &[u32],
    ids: &[u64],
    skip: u32,
    width: u32,
    height: u32,
) -> Landing {
    if !proposed {
        return Landing { x, y, face_dx, face_dy, speed: 0, moved: false };
    }
    let yielded = grid_yield(width, height, head, next, prop_x, prop_y, ids, cand_x, cand_y, self_id, skip);
    if yielded {
        Landing { x, y, face_dx, face_dy, speed: 0, moved: false }
    } else {
        Landing { x: cand_x, y: cand_y, face_dx: cand_fx, face_dy: cand_fy, speed: cand_speed, moved: true }
    }
}

/// One parallel-friendly round. Proposals read `xs`/`ys` only. `prop_on` is 1
/// after the call when that agent moved. Returns false when the slices or the
/// grid buffers disagree, and then writes nothing.
pub fn tick_round(
    xs: &mut [u32],
    ys: &mut [u32],
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    ids: &[u64],
    mask: &[u8],
    width: u32,
    height: u32,
    head: &mut [u32],
    next: &mut [u32],
    prop_x: &mut [u32],
    prop_y: &mut [u32],
    prop_on: &mut [u32],
) -> bool {
    let n = xs.len();
    if width == 0 || height == 0 {
        return false;
    }
    let cells = (width as usize) * (height as usize);
    let ready = ys.len() == n
        && moving.len() == n
        && cand_x.len() == n
        && cand_y.len() == n
        && ids.len() == n
        && prop_x.len() >= n
        && prop_y.len() >= n
        && prop_on.len() >= n
        && next.len() >= n
        && head.len() >= cells;
    if !ready {
        return false;
    }
    let mut i = 0;
    while i < n {
        prop_on[i] = 1;
        i += 1;
    }
    if !grid_build(head, next, width, height, xs, ys, prop_on) {
        return false;
    }
    i = 0;
    while i < n {
        let proposal = propose_one(
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
        prop_x[i] = proposal.x;
        prop_y[i] = proposal.y;
        prop_on[i] = if proposal.proposed { 1 } else { 0 };
        i += 1;
    }
    if !grid_build(head, next, width, height, prop_x, prop_y, prop_on) {
        return false;
    }
    i = 0;
    while i < n {
        let land = resolve_one(
            xs[i],
            ys[i],
            0,
            0,
            prop_x[i],
            prop_y[i],
            0,
            0,
            0,
            prop_on[i] != 0,
            ids[i],
            head,
            next,
            prop_x,
            prop_y,
            ids,
            i as u32,
            width,
            height,
        );
        xs[i] = land.x;
        ys[i] = land.y;
        prop_on[i] = if land.moved { 1 } else { 0 };
        i += 1;
    }
    true
}

/// Full scan of the radius. The grid walk agrees with this on a grid that
/// [`grid_build`] just filled with every in-map agent.
pub fn scan_hit(xs: &[u32], ys: &[u32], x: u32, y: u32, skip: u32) -> bool {
    let mut hit = false;
    let n = xs.len();
    let mut i = 0usize;
    while i < n {
        if i < ys.len() && (i as u32) != skip && geom::dist_sq(xs[i], ys[i], x, y) < COLLISION_SQ {
            hit = true;
        }
        i += 1;
    }
    hit
}

/// Full scan of better-priority proposals inside the radius. `on[i] != 0`
/// marks a proposal. The same predicate as [`grid_yield`] on a grid of those
/// proposals.
pub fn scan_yield(xs: &[u32], ys: &[u32], ids: &[u64], on: &[u32], x: u32, y: u32, self_id: u64, skip: u32) -> bool {
    let mut hit = false;
    let n = xs.len();
    let mut i = 0usize;
    while i < n {
        let present = i < ys.len() && i < ids.len() && i < on.len() && on[i] != 0 && (i as u32) != skip;
        let better = if !present {
            false
        } else if ids[i] < self_id {
            true
        } else if ids[i] == self_id && (i as u32) < skip {
            true
        } else {
            false
        };
        if present && better && geom::dist_sq(xs[i], ys[i], x, y) < COLLISION_SQ {
            hit = true;
        }
        i += 1;
    }
    hit
}

/// One two-phase pass over a snapshot. Members, conversations, ids, and owners
/// stay. Positions follow [`tick_round`].
#[cfg(not(target_arch = "nvptx64"))]
pub fn tick_players_round(
    mut s: crate::Snapshot,
    moving: &[u32],
    cand_x: &[u32],
    cand_y: &[u32],
    ids: &[u64],
) -> crate::Snapshot {
    let n = s.players.len();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut prop_x = Vec::new();
    let mut prop_y = Vec::new();
    let mut prop_on = Vec::new();
    let mut next = Vec::new();
    let mut i = 0;
    while i < n {
        xs.push(s.players[i].x);
        ys.push(s.players[i].y);
        prop_x.push(0);
        prop_y.push(0);
        prop_on.push(0);
        next.push(0);
        i += 1;
    }
    let cells = (s.meta.width as usize) * (s.meta.height as usize);
    let mut head = Vec::new();
    i = 0;
    while i < cells {
        head.push(0);
        i += 1;
    }
    let _ = tick_round(
        &mut xs,
        &mut ys,
        moving,
        cand_x,
        cand_y,
        ids,
        &s.terrain.mask,
        s.meta.width,
        s.meta.height,
        &mut head,
        &mut next,
        &mut prop_x,
        &mut prop_y,
        &mut prop_on,
    );
    i = 0;
    while i < n && i < xs.len() && i < ys.len() {
        s.players[i].x = xs[i];
        s.players[i].y = ys[i];
        i += 1;
    }
    s
}
