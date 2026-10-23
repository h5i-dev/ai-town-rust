//! Integer geometry and the min-heap used by pathfinding.
//!
//! Positions are milli-tiles. A tile is [`MILLI`] units. Squared distance
//! avoids a square root on the collision check. The heap is the 1-indexed
//! algorithm from `convex/util/minheap.ts`.

#[cfg(not(target_arch = "nvptx64"))]
use crate::{PathNode, COLLISION_SQ, FACE_BIAS, MILLI, SPEED};
#[cfg(target_arch = "nvptx64")]
use crate::{COLLISION_SQ, MILLI};

pub fn abs_diff(a: u32, b: u32) -> u64 {
    if a >= b {
        (a - b) as u64
    } else {
        (b - a) as u64
    }
}

/// Squared distance. Saturates when a coordinate delta is absurd, so the
/// multiply cannot wrap.
pub fn dist_sq(ax: u32, ay: u32, bx: u32, by: u32) -> u64 {
    let dx = abs_diff(ax, bx);
    let dy = abs_diff(ay, by);
    if dx > 2_000_000 || dy > 2_000_000 {
        return u64::MAX;
    }
    dx * dx + dy * dy
}

pub fn manhattan(ax: u32, ay: u32, bx: u32, by: u32) -> u32 {
    let d = abs_diff(ax, bx) + abs_diff(ay, by);
    if d > u32::MAX as u64 {
        u32::MAX
    } else {
        d as u32
    }
}

pub fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    // Start above sqrt(n), within a factor of two. Starting at n wastes
    // integer divisions halving the estimate on every facing/path query.
    // x <= 2^32 also keeps x + n/x in range for every u64 input.
    let mut bits = n;
    let mut x = 1u64;
    while bits > 0 {
        bits >>= 2;
        x <<= 1;
    }
    let mut y = (x + n / x) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn face_zero() -> u32 {
    FACE_BIAS
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn face_east() -> u32 {
    FACE_BIAS + 1000
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn face_west() -> u32 {
    FACE_BIAS - 1000
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn face_south() -> u32 {
    FACE_BIAS + 1000
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn face_north() -> u32 {
    FACE_BIAS - 1000
}

/// Unit facing from `a` to `b`, stored as component + [`FACE_BIAS`].
#[cfg(not(target_arch = "nvptx64"))]
pub fn face_toward(ax: u32, ay: u32, bx: u32, by: u32) -> (u32, u32) {
    let dx = abs_diff(ax, bx);
    let dy = abs_diff(ay, by);
    let len = isqrt(dx * dx + dy * dy);
    if len == 0 {
        return (face_zero(), face_zero());
    }
    let mx = (dx * 1000 / len) as u32;
    let my = (dy * 1000 / len) as u32;
    let fdx = if bx >= ax { FACE_BIAS + mx } else { FACE_BIAS - mx };
    let fdy = if by >= ay { FACE_BIAS + my } else { FACE_BIAS - my };
    (fdx, fdy)
}

pub fn tile_of(milli: u32) -> u32 {
    milli / MILLI
}

/// Milliseconds to travel `milli` at [`SPEED`] milli-tiles per second.
#[cfg(not(target_arch = "nvptx64"))]
pub fn travel_ms(milli: u32) -> u64 {
    (milli as u64) * 1000 / SPEED as u64
}

pub fn lerp(a: u32, b: u32, u: u64, dt: u64) -> u32 {
    if dt == 0 || a == b {
        return a;
    }
    if b >= a {
        (a as u64 + (b - a) as u64 * u / dt) as u32
    } else {
        (a as u64 - (a - b) as u64 * u / dt) as u32
    }
}

#[cfg(not(target_arch = "nvptx64"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pose {
    pub x: u32,
    pub y: u32,
    pub face_dx: u32,
    pub face_dy: u32,
    pub speed: u32,
}

/// Position along a path at `now`. `None` when the path has fewer than two points.
#[cfg(not(target_arch = "nvptx64"))]
pub fn path_pose(path: &[PathNode], now: u64) -> Option<Pose> {
    if path.len() < 2 {
        return None;
    }
    let first = &path[0];
    if now < first.t {
        return Some(Pose { x: first.x, y: first.y, face_dx: first.face_dx, face_dy: first.face_dy, speed: 0 });
    }
    let last = &path[path.len() - 1];
    if now > last.t {
        return Some(Pose { x: last.x, y: last.y, face_dx: last.face_dx, face_dy: last.face_dy, speed: 0 });
    }
    let mut i = 0;
    while i + 1 < path.len() {
        let a = &path[i];
        let b = &path[i + 1];
        if a.t <= now && now <= b.t {
            let dt = b.t - a.t;
            let u = now - a.t;
            let x = lerp(a.x, b.x, u, dt);
            let y = lerp(a.y, b.y, u, dt);
            let speed = if dt == 0 {
                0
            } else {
                // Most town paths follow grid axes. Their exact integer length needs
                // no square root (and no repeated integer divisions).
                let d = if a.x == b.x { abs_diff(a.y, b.y) }
                    else if a.y == b.y { abs_diff(a.x, b.x) }
                    else { isqrt(dist_sq(a.x, a.y, b.x, b.y)) };
                (d * 1000 / dt) as u32
            };
            return Some(Pose { x, y, face_dx: a.face_dx, face_dy: a.face_dy, speed });
        }
        i += 1;
    }
    None
}

#[cfg(not(target_arch = "nvptx64"))]
fn between(a: u32, b: u32, c: u32) -> bool {
    if a <= c {
        a <= b && b <= c
    } else {
        c <= b && b <= a
    }
}

/// `b - a` points the same way as `c - a`, or `b` sits on `a`.
/// A boolean `==` of two comparisons extracts as a proposition, which Lean
/// then refuses as an `if` condition, so the branches are written out.
#[cfg(not(target_arch = "nvptx64"))]
fn axis_same(a: u32, b: u32, c: u32) -> bool {
    if b == a {
        return true;
    }
    if b > a {
        c >= a
    } else {
        c < a
    }
}

/// True when `b` lies on the segment `a`–`c` and the facing does not change.
#[cfg(not(target_arch = "nvptx64"))]
pub fn colinear(a: &PathNode, b: &PathNode, c: &PathNode) -> bool {
    if a.face_dx != b.face_dx || a.face_dy != b.face_dy || b.face_dx != c.face_dx || b.face_dy != c.face_dy {
        return false;
    }
    let abx = abs_diff(a.x, b.x);
    let aby = abs_diff(a.y, b.y);
    let acx = abs_diff(a.x, c.x);
    let acy = abs_diff(a.y, c.y);
    if abx * acy != aby * acx {
        return false;
    }
    // Same direction: each axis of `b - a` agrees with `c - a`.
    if !axis_same(a.x, b.x, c.x) {
        return false;
    }
    if !axis_same(a.y, b.y, c.y) {
        return false;
    }
    between(a.x, b.x, c.x) && between(a.y, b.y, c.y)
}

/// Drop waypoints that interpolate between their neighbors. Same idea as
/// `compressPath` in the original geometry module.
///
/// The scan keeps an anchor and one candidate index. A candidate that is
/// colinear with the anchor and the next point is skipped. The loop is a
/// single index so Aeneas can find a fixed point (no `Option`, no `continue`).
#[cfg(not(target_arch = "nvptx64"))]
pub fn compress(path: &Vec<PathNode>) -> Vec<PathNode> {
    if path.len() <= 2 {
        return path.clone();
    }
    let mut out = Vec::new();
    out.push(path[0].clone());
    let mut last = 0;
    let mut candidate = 1;
    let mut i = 2;
    while i < path.len() {
        if colinear(&path[last], &path[candidate], &path[i]) {
            candidate = i;
        } else {
            out.push(path[candidate].clone());
            last = candidate;
            candidate = i;
        }
        i += 1;
    }
    out.push(path[candidate].clone());
    out
}

/// Remove the last element by copying the prefix. `Vec::pop` is not in the
/// Aeneas subset this toolchain translates, and a heap pop is the only caller.
#[cfg(not(target_arch = "nvptx64"))]
fn drop_last_u32(tree: &mut Vec<u32>) -> Option<u32> {
    let n = tree.len();
    if n == 0 {
        return None;
    }
    let last = tree[n - 1];
    let mut kept = Vec::new();
    let mut i = 0;
    while i + 1 < n {
        kept.push(tree[i]);
        i += 1;
    }
    *tree = kept;
    Some(last)
}

#[cfg(not(target_arch = "nvptx64"))]
fn drop_last_u64(tree: &mut Vec<u64>) -> Option<u64> {
    let n = tree.len();
    if n == 0 {
        return None;
    }
    let last = tree[n - 1];
    let mut kept = Vec::new();
    let mut i = 0;
    while i + 1 < n {
        kept.push(tree[i]);
        i += 1;
    }
    *tree = kept;
    Some(last)
}

/// 1-indexed binary heap of `u64`, smallest on top. `compare(a, b)` in the
/// original is `a > b`.
#[cfg(not(target_arch = "nvptx64"))]
#[derive(Clone, Debug)]
pub struct MinHeap {
    tree: Vec<u64>,
}

#[cfg(not(target_arch = "nvptx64"))]
impl MinHeap {
    pub fn new() -> Self {
        let mut tree = Vec::new();
        tree.push(0);
        MinHeap { tree }
    }

    pub fn len(&self) -> usize {
        self.tree.len() - 1
    }

    pub fn peek(&self) -> Option<u64> {
        if self.tree.len() <= 1 {
            None
        } else {
            Some(self.tree[1])
        }
    }

    pub fn push(&mut self, new_value: u64) {
        self.tree.push(new_value);
        let mut destination = self.tree.len() - 1;
        while destination > 1 {
            let parent = destination >> 1;
            let existing = self.tree[parent];
            if new_value > existing {
                break;
            }
            self.tree[destination] = existing;
            destination = parent;
        }
        self.tree[destination] = new_value;
    }

    pub fn pop(&mut self) -> Option<u64> {
        if self.tree.len() <= 1 {
            return None;
        }
        let value = self.tree[1];
        let last = match drop_last_u64(&mut self.tree) {
            Some(v) => v,
            None => return Some(value),
        };
        if self.tree.len() == 1 {
            return Some(value);
        }
        let last_value = last;
        let mut destination = 1;
        let end = self.tree.len();
        while (destination << 1) < end {
            let mut child = destination << 1;
            if child + 1 < end && self.tree[child] > self.tree[child + 1] {
                child += 1;
            }
            let existing = self.tree[child];
            if existing > last_value {
                break;
            }
            self.tree[destination] = existing;
            destination = child;
        }
        self.tree[destination] = last_value;
        Some(value)
    }
}

/// Heap of search-node indices. `cost[node]` decides the order. Index 0 of
/// `tree` is unused.
#[cfg(not(target_arch = "nvptx64"))]
pub fn idx_push(tree: &mut Vec<u32>, cost: &[u32], node: u32) {
    tree.push(node);
    let mut destination = tree.len() - 1;
    while destination > 1 {
        let parent = destination >> 1;
        let existing = tree[parent];
        if cost[node as usize] > cost[existing as usize] {
            break;
        }
        tree[destination] = existing;
        destination = parent;
    }
    tree[destination] = node;
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn idx_pop(tree: &mut Vec<u32>, cost: &[u32]) -> Option<u32> {
    if tree.len() <= 1 {
        return None;
    }
    let value = tree[1];
    let last = match drop_last_u32(tree) {
        Some(v) => v,
        None => return None,
    };
    if tree.len() == 1 {
        return Some(value);
    }
    let last_value = last;
    let mut destination = 1;
    let end = tree.len();
    while (destination << 1) < end {
        let mut child = destination << 1;
        if child + 1 < end && cost[tree[child] as usize] > cost[tree[child + 1] as usize] {
            child += 1;
        }
        let existing = tree[child];
        if cost[existing as usize] > cost[last_value as usize] {
            break;
        }
        tree[destination] = existing;
        destination = child;
    }
    tree[destination] = last_value;
    Some(value)
}

/// Bit `tx * height + ty` of the walk mask. Set means blocked.
pub fn mask_blocked(mask: &[u8], width: u32, height: u32, x: u32, y: u32) -> bool {
    if width == 0 || height == 0 {
        return true;
    }
    let tx = tile_of(x);
    let ty = tile_of(y);
    if tx >= width || ty >= height {
        return true;
    }
    // u64 so the index cannot overflow on a 32-bit usize: tx < width and
    // ty < height are both below 2^32. Same bit as `tx * height + ty`.
    let i = (tx as u64) * (height as u64) + (ty as u64);
    let byte = i / 8;
    let bit = i % 8;
    if byte >= mask.len() as u64 {
        return true;
    }
    ((mask[byte as usize] >> bit) & 1) == 1
}

/// Bytes in a bit mask of `cells` bits. Same value as `usize::div_ceil(8)`,
/// written with division so extraction does not pull in `core::num::div_ceil`.
pub fn mask_bytes(cells: usize) -> usize {
    if cells == 0 {
        0
    } else {
        (cells - 1) / 8 + 1
    }
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn set_blocked(mask: &mut Vec<u8>, width: u32, height: u32, tx: u32, ty: u32) {
    let cells = (width as usize) * (height as usize);
    let bytes = mask_bytes(cells);
    while mask.len() < bytes {
        mask.push(0);
    }
    if tx >= width || ty >= height {
        return;
    }
    let i = (tx as usize) * (height as usize) + (ty as usize);
    let byte = i / 8;
    let bit = i % 8;
    mask[byte] |= 1 << bit;
}

/// Buckets of point indices, one tile wide. A collision radius below one tile
/// only needs the 3 by 3 neighborhood. `head[cell]` and `next[i]` are `u32::MAX`
/// when the chain ends. Callers that move a point by less than a tile between
/// checks may leave it in the old cell; the 3 by 3 neighborhood still reaches it
/// while the radius plus the step stays under one tile.
#[cfg(not(target_arch = "nvptx64"))]
pub struct NearIndex {
    pub width: u32,
    pub height: u32,
    pub head: Vec<u32>,
    pub next: Vec<u32>,
}

#[cfg(not(target_arch = "nvptx64"))]
fn grow_max(v: &mut Vec<u32>, n: usize) {
    while v.len() < n {
        v.push(u32::MAX);
    }
}

/// Fill `index`, reusing its buffers when the caller steps many ticks.
#[cfg(not(target_arch = "nvptx64"))]
pub fn near_fill(index: &mut NearIndex, width: u32, height: u32, xs: &[u32], ys: &[u32]) {
    let cells = (width as usize) * (height as usize);
    let n = if xs.len() < ys.len() { xs.len() } else { ys.len() };
    grow_max(&mut index.head, cells);
    grow_max(&mut index.next, n);
    let mut i = 0;
    while i < cells {
        index.head[i] = u32::MAX;
        i += 1;
    }
    i = 0;
    while i < n {
        index.next[i] = u32::MAX;
        i += 1;
    }
    index.width = width;
    index.height = height;
    i = 0;
    while i < n {
        let tx = xs[i] / MILLI;
        let ty = ys[i] / MILLI;
        if height > 0 && tx < width && ty < height {
            let c = (tx as usize) * (height as usize) + (ty as usize);
            if c < cells {
                index.next[i] = index.head[c];
                index.head[c] = i as u32;
            }
        }
        i += 1;
    }
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn near_index(width: u32, height: u32, xs: &[u32], ys: &[u32]) -> NearIndex {
    let mut index = NearIndex { width: 0, height: 0, head: Vec::new(), next: Vec::new() };
    near_fill(&mut index, width, height, xs, ys);
    index
}

/// True when some indexed point other than `skip` is strictly inside the
/// collision radius. `head` and `next` are the one-tile chains from
/// [`near_fill`]. The walk has no `return`, so extraction keeps one loop.
pub fn grid_hit(width: u32, height: u32, head: &[u32], next: &[u32], xs: &[u32], ys: &[u32], x: u32, y: u32, skip: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let cx = x / MILLI;
    let cy = y / MILLI;
    if cx >= width || cy >= height {
        return false;
    }
    let x0 = if cx == 0 { 0 } else { cx - 1 };
    let y0 = if cy == 0 { 0 } else { cy - 1 };
    let x1 = if cx + 1 >= width { width - 1 } else { cx + 1 };
    let y1 = if cy + 1 >= height { height - 1 } else { cy + 1 };
    let mut hit = false;
    let mut tx = x0;
    while tx <= x1 {
        let mut ty = y0;
        while ty <= y1 {
            if !hit {
                let c = cell_index(tx, ty, height);
                if c < head.len() {
                    let mut p = head[c];
                    let mut guard = 0u32;
                    while p != u32::MAX {
                        if (p as usize) >= next.len() || (p as usize) >= xs.len() || (p as usize) >= ys.len() {
                            p = u32::MAX;
                        } else {
                            let i = p as usize;
                            if (i as u32) != skip && dist_sq(xs[i], ys[i], x, y) < COLLISION_SQ {
                                hit = true;
                                p = u32::MAX;
                            } else {
                                let nxt = next[i];
                                if nxt == p {
                                    p = u32::MAX;
                                } else {
                                    p = nxt;
                                    guard = guard.wrapping_add(1);
                                    if guard > next.len() as u32 {
                                        p = u32::MAX;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ty += 1;
        }
        tx += 1;
    }
    hit
}

/// True when some point other than `skip` is strictly inside the collision radius.
#[cfg(not(target_arch = "nvptx64"))]
pub fn near_hit(index: &NearIndex, xs: &[u32], ys: &[u32], x: u32, y: u32, skip: u32) -> bool {
    grid_hit(index.width, index.height, &index.head, &index.next, xs, ys, x, y, skip)
}

/// Index of tile `(tx, ty)` in a column-major grid of `height` rows. The CPU
/// grid, the walk in [`grid_hit`] and [`grid_yield`], and the PTX inserts all
/// use this one formula.
pub fn cell_index(tx: u32, ty: u32, height: u32) -> usize {
    (tx as usize) * (height as usize) + (ty as usize)
}

/// The cell holding milli-tile point `(x, y)`, or `usize::MAX` off the map.
pub fn cell_of(x: u32, y: u32, width: u32, height: u32) -> usize {
    let tx = x / MILLI;
    let ty = y / MILLI;
    if width == 0 || height == 0 || tx >= width || ty >= height {
        usize::MAX
    } else {
        cell_index(tx, ty, height)
    }
}

/// Rebuild one-tile chains. `on[i] != 0` inserts agent `i` at `(xs[i], ys[i])`.
/// Links run from the newest insert toward older ones. Returns false when the
/// buffers are short or the map is empty, and then leaves the slices untouched.
pub fn grid_build(head: &mut [u32], next: &mut [u32], width: u32, height: u32, xs: &[u32], ys: &[u32], on: &[u32]) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let cells = (width as usize) * (height as usize);
    let n = xs.len();
    if ys.len() < n || on.len() < n || next.len() < n || head.len() < cells {
        return false;
    }
    let mut c = 0usize;
    while c < cells {
        head[c] = u32::MAX;
        c += 1;
    }
    let mut i = 0usize;
    while i < n {
        next[i] = u32::MAX;
        i += 1;
    }
    i = 0;
    while i < n {
        if on[i] != 0 {
            let cell = cell_of(xs[i], ys[i], width, height);
            if cell < cells && i < next.len() {
                next[i] = head[cell];
                head[cell] = i as u32;
            }
        }
        i += 1;
    }
    true
}

/// True when `head`/`next` hold exactly the agents with `on[i] != 0` whose
/// point is on the map, each once, in the chain of its own [`cell_of`], with
/// no cycle. Chain order is free: [`grid_build`] and the PTX atomic inserts
/// order a cell differently, and [`grid_hit`] / [`grid_yield`] only look at
/// membership. `seen` is scratch of at least `xs.len()` words. O(cells + n).
pub fn grid_wellformed(head: &[u32], next: &[u32], width: u32, height: u32, xs: &[u32], ys: &[u32], on: &[u32], seen: &mut [u32]) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let cells = (width as usize) * (height as usize);
    let n = xs.len();
    if ys.len() < n || on.len() < n || next.len() < n || seen.len() < n || head.len() < cells {
        return false;
    }
    let mut i = 0usize;
    while i < n {
        seen[i] = 0;
        i += 1;
    }
    let mut ok = true;
    let mut c = 0usize;
    while c < cells {
        let mut p = head[c];
        while ok && p != u32::MAX {
            let k = p as usize;
            if k >= n || on[k] == 0 || seen[k] != 0 || cell_of(xs[k], ys[k], width, height) != c {
                ok = false;
            } else {
                seen[k] = 1;
                p = next[k];
            }
        }
        c += 1;
    }
    i = 0;
    while ok && i < n {
        let placed = on[i] != 0 && cell_of(xs[i], ys[i], width, height) < cells;
        if placed != (seen[i] != 0) {
            ok = false;
        }
        i += 1;
    }
    ok
}

/// True when a chained proposal with a better priority is strictly inside the
/// collision radius. Better means a lower player id, then a lower index.
/// `skip` is this agent and is ignored. The walk has no `return`.
pub fn grid_yield(
    width: u32,
    height: u32,
    head: &[u32],
    next: &[u32],
    xs: &[u32],
    ys: &[u32],
    ids: &[u64],
    x: u32,
    y: u32,
    self_id: u64,
    skip: u32,
) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let cx = x / MILLI;
    let cy = y / MILLI;
    if cx >= width || cy >= height {
        return false;
    }
    let x0 = if cx == 0 { 0 } else { cx - 1 };
    let y0 = if cy == 0 { 0 } else { cy - 1 };
    let x1 = if cx + 1 >= width { width - 1 } else { cx + 1 };
    let y1 = if cy + 1 >= height { height - 1 } else { cy + 1 };
    let mut hit = false;
    let mut tx = x0;
    while tx <= x1 {
        let mut ty = y0;
        while ty <= y1 {
            if !hit {
                let c = cell_index(tx, ty, height);
                if c < head.len() {
                    let mut p = head[c];
                    let mut guard = 0u32;
                    while p != u32::MAX {
                        if (p as usize) >= next.len() || (p as usize) >= xs.len() || (p as usize) >= ys.len() {
                            p = u32::MAX;
                        } else {
                            let i = p as usize;
                            let better = if i >= ids.len() {
                                false
                            } else if ids[i] < self_id {
                                true
                            } else if ids[i] == self_id && (i as u32) < skip {
                                true
                            } else {
                                false
                            };
                            if (i as u32) != skip && better && dist_sq(xs[i], ys[i], x, y) < COLLISION_SQ {
                                hit = true;
                                p = u32::MAX;
                            } else {
                                let nxt = next[i];
                                if nxt == p {
                                    p = u32::MAX;
                                } else {
                                    p = nxt;
                                    guard = guard.wrapping_add(1);
                                    if guard > next.len() as u32 {
                                        p = u32::MAX;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ty += 1;
        }
        tx += 1;
    }
    hit
}
