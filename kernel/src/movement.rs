//! Grid pathfinding. Destinations are whole tiles. A search walks tile centers
//! and, when the player is between centers, the first step snaps to one.

use crate::geom::{self, idx_pop, idx_push};
use crate::{PathNode, MILLI};

struct Node {
    x: u32,
    y: u32,
    g: u32,
    parent: u32,
}

pub struct Route {
    pub nodes: Vec<PathNode>,
    pub dest_x: u32,
    pub dest_y: u32,
}

fn cell_of(width: u32, height: u32, x: u32, y: u32) -> Option<usize> {
    if width == 0 || height == 0 || x % MILLI != 0 || y % MILLI != 0 {
        return None;
    }
    let tx = x / MILLI;
    let ty = y / MILLI;
    if tx >= width || ty >= height {
        return None;
    }
    Some((tx as usize) * (height as usize) + (ty as usize))
}

fn point_blocked(mask: &[u8], width: u32, height: u32, near: &geom::NearIndex, ox: &[u32], oy: &[u32], x: u32, y: u32) -> bool {
    if geom::mask_blocked(mask, width, height, x, y) {
        return true;
    }
    geom::near_hit(near, ox, oy, x, y, u32::MAX)
}

fn push_neighbor(xs: &mut [u32; 4], ys: &mut [u32; 4], n: &mut usize, x: u32, y: u32) {
    if *n < 4 {
        xs[*n] = x;
        ys[*n] = y;
        *n += 1;
    }
}

fn neighbors(x: u32, y: u32, xs: &mut [u32; 4], ys: &mut [u32; 4]) -> usize {
    let mut n = 0;
    if x % MILLI != 0 {
        let floor = (x / MILLI) * MILLI;
        push_neighbor(xs, ys, &mut n, floor, y);
        push_neighbor(xs, ys, &mut n, floor + MILLI, y);
    }
    if y % MILLI != 0 {
        let floor = (y / MILLI) * MILLI;
        push_neighbor(xs, ys, &mut n, x, floor);
        push_neighbor(xs, ys, &mut n, x, floor + MILLI);
    }
    if x % MILLI == 0 && y % MILLI == 0 {
        push_neighbor(xs, ys, &mut n, x + MILLI, y);
        if x >= MILLI {
            push_neighbor(xs, ys, &mut n, x - MILLI, y);
        }
        push_neighbor(xs, ys, &mut n, x, y + MILLI);
        if y >= MILLI {
            push_neighbor(xs, ys, &mut n, x, y - MILLI);
        }
    }
    n
}

/// Shortest axis-aligned route that stays on walkable tiles and out of other
/// players. `None` when the player cannot leave the start. When the exact
/// destination is blocked, the route ends at the closest reachable center.
pub fn find_route(
    mask: &[u8],
    width: u32,
    height: u32,
    ox: &[u32],
    oy: &[u32],
    player: u64,
    sx: u32,
    sy: u32,
    dest_tx: u32,
    dest_ty: u32,
    now: u64,
) -> Option<Route> {
    if width == 0 || height == 0 || dest_tx >= width || dest_ty >= height {
        return None;
    }
    let dest_x = dest_tx * MILLI;
    let dest_y = dest_ty * MILLI;
    if sx == dest_x && sy == dest_y {
        return None;
    }
    let near = geom::near_index(width, height, ox, oy);
    let cells = (width as usize) * (height as usize);
    let mut best = Vec::new();
    let mut i = 0;
    while i < cells {
        best.push(u32::MAX);
        i += 1;
    }
    let mut nodes = Vec::new();
    nodes.push(Node { x: sx, y: sy, g: 0, parent: u32::MAX });
    let mut cost = Vec::new();
    cost.push(geom::manhattan(sx, sy, dest_x, dest_y));
    if let Some(c) = cell_of(width, height, sx, sy) {
        best[c] = 0;
    }
    let mut heap = Vec::new();
    heap.push(0);
    idx_push(&mut heap, &cost, 0);

    let mut best_i: u32 = 0;
    let mut found: Option<u32> = None;
    let mut expanded: u32 = 0;
    while heap.len() > 1 && expanded < 4096 && nodes.len() < 8192 {
        let ni = match idx_pop(&mut heap, &cost) {
            Some(v) => v,
            None => break,
        };
        if ni as usize >= nodes.len() {
            continue;
        }
        let node_x = nodes[ni as usize].x;
        let node_y = nodes[ni as usize].y;
        let node_g = nodes[ni as usize].g;
        if let Some(c) = cell_of(width, height, node_x, node_y) {
            if best[c] < node_g {
                continue;
            }
        }
        expanded += 1;
        if node_x == dest_x && node_y == dest_y {
            found = Some(ni);
            break;
        }
        let here = geom::manhattan(node_x, node_y, dest_x, dest_y);
        let so_far = geom::manhattan(nodes[best_i as usize].x, nodes[best_i as usize].y, dest_x, dest_y);
        if here < so_far {
            best_i = ni;
        }
        let mut nxs = [0u32; 4];
        let mut nys = [0u32; 4];
        let n = neighbors(node_x, node_y, &mut nxs, &mut nys);
        let mut k = 0;
        while k < n {
            let nx = nxs[k];
            let ny = nys[k];
            k += 1;
            if nx == node_x && ny == node_y {
                continue;
            }
            if point_blocked(mask, width, height, &near, ox, oy, nx, ny) {
                continue;
            }
            let step = geom::manhattan(node_x, node_y, nx, ny);
            let g = node_g.saturating_add(step);
            if let Some(c) = cell_of(width, height, nx, ny) {
                if best[c] <= g {
                    continue;
                }
                best[c] = g;
            }
            let idx = nodes.len() as u32;
            nodes.push(Node { x: nx, y: ny, g, parent: ni });
            let h = geom::manhattan(nx, ny, dest_x, dest_y);
            cost.push(g.saturating_add(h));
            idx_push(&mut heap, &cost, idx);
        }
    }

    let end = match found {
        Some(i) => i,
        None => {
            if nodes[best_i as usize].g == 0 {
                return None;
            }
            best_i
        }
    };

    let mut order = Vec::new();
    let mut cur = end;
    let mut guard = 0;
    loop {
        order.push(cur);
        let parent = nodes[cur as usize].parent;
        if parent == u32::MAX {
            break;
        }
        cur = parent;
        guard += 1;
        if guard > 4096 {
            break;
        }
    }

    let reached_x = nodes[end as usize].x;
    let reached_y = nodes[end as usize].y;
    let mut dense = Vec::new();
    let mut i = order.len();
    while i > 0 {
        i -= 1;
        let n = &nodes[order[i] as usize];
        dense.push(PathNode {
            player,
            seq: 0,
            x: n.x,
            y: n.y,
            face_dx: geom::face_zero(),
            face_dy: geom::face_zero(),
            t: now + geom::travel_ms(n.g),
        });
    }
    // `order` was destination-first and we walked it backward, so `dense` is
    // start-first. Facing on point i is the step toward i+1.
    let mut i = 0;
    while i + 1 < dense.len() {
        let (fdx, fdy) = geom::face_toward(dense[i].x, dense[i].y, dense[i + 1].x, dense[i + 1].y);
        dense[i].face_dx = fdx;
        dense[i].face_dy = fdy;
        i += 1;
    }
    if dense.len() >= 2 {
        let last = dense.len() - 1;
        dense[last].face_dx = dense[last - 1].face_dx;
        dense[last].face_dy = dense[last - 1].face_dy;
    }
    let packed = geom::compress(&dense);
    if packed.len() < 2 {
        return None;
    }
    let mut nodes = packed;
    let mut i = 0;
    while i < nodes.len() {
        nodes[i].seq = i as u64;
        nodes[i].player = player;
        i += 1;
    }
    Some(Route { nodes, dest_x: reached_x, dest_y: reached_y })
}
