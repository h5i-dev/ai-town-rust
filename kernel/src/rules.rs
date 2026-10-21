//! Game rules. Commands mutate a copy of the snapshot. `diff` is the write set,
//! so `apply(before, diff(before, after))` is `after`.

use crate::geom::{self, path_pose};
use crate::movement::find_route;
use crate::{
    Agent, Command, Conversation, Error, Member, Memory, Message, PathNode, Player, Reply, Snapshot, Write, ACTION_TIMEOUT_MS,
    ACTIVITY_MS, AGENT_BASE, AWKWARD_MS, COLLISION_SQ, CONV_COOLDOWN_MS, ENGINE, IDLE_MS, INVITE_MS, MAX_CONV_MS,
    MAX_DIM, MAX_HUMANS, MAX_MESSAGES, MAX_NAME, MAX_PATHFINDS, MAX_PLAN, MAX_TEXT, MAX_TICKS, MESSAGE_COOLDOWN_MS, MILLI,
    OP_DO, OP_NONE, OP_REMEMBER, OP_SPEAK, PATH_TIMEOUT_MS, PF_MOVE, PF_NEED, PF_NONE, PF_WAIT, ST_INVITED, ST_TALKING,
    ST_WALKING, TALK_SQ, TICK_MS, TYPING_MS,
};

pub fn text_ok(t: &Vec<u8>, max: usize) -> bool {
    t.len() > 0 && t.len() <= max
}

pub fn player_index(players: &Vec<Player>, id: u64) -> Option<usize> {
    let mut i = 0;
    while i < players.len() {
        if players[i].id == id {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Tick lookup. Ids are allocated in order, so a hit is the binary probe.
/// A miss scans, which is what [`player_index`] does for every id. The scan
/// stays the function the membership proofs unfold.
pub fn player_at(players: &Vec<Player>, id: u64) -> Option<usize> {
    let mut lo = 0usize;
    let mut hi = players.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if players[mid].id < id {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < players.len() && players[lo].id == id {
        return Some(lo);
    }
    player_index(players, id)
}

pub fn player_by_owner(players: &Vec<Player>, owner: u64) -> Option<usize> {
    let mut i = 0;
    while i < players.len() {
        if players[i].owner == owner {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn member_index(members: &Vec<Member>, player: u64) -> Option<usize> {
    let mut i = 0;
    while i < members.len() {
        if members[i].player == player {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn conv_index(convs: &Vec<Conversation>, id: u64) -> Option<usize> {
    let mut i = 0;
    while i < convs.len() {
        if convs[i].id == id {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Full player scan. Placement and the tick use the spatial grid, which agrees
/// with this test while one step stays inside a tile. Extraction still starts
/// here so the collision proof names the same comparison.
#[allow(dead_code)]
pub fn occupied(s: &Snapshot, x: u32, y: u32, ignore: u64) -> bool {
    if geom::mask_blocked(&s.terrain.mask, s.meta.width, s.meta.height, x, y) {
        return true;
    }
    let mut i = 0;
    while i < s.players.len() {
        let p = &s.players[i];
        if p.id != ignore && geom::dist_sq(p.x, p.y, x, y) < COLLISION_SQ {
            return true;
        }
        i += 1;
    }
    false
}

/// True when every player stands on a walkable tile and no other player is
/// strictly inside its collision radius: `occupied` is false at each player's
/// own point, ignoring that player. Every pair is compared, so this is O(n^2).
pub fn placement_ok(s: &Snapshot) -> bool {
    let mut i = 0;
    while i < s.players.len() {
        let p = &s.players[i];
        if occupied(s, p.x, p.y, p.id) {
            return false;
        }
        i += 1;
    }
    true
}

/// Move every player to `(x[i], y[i])`, stop them, and drop all paths. The
/// positions come from outside the kernel (the GPU movement minute), so the
/// result is refused with `Occupied` unless [`placement_ok`] accepts it.
pub fn relocate(mut s: Snapshot, x: &Vec<u32>, y: &Vec<u32>) -> Result<Snapshot, Error> {
    let n = s.players.len();
    if x.len() != n || y.len() != n {
        return Err(Error::BadState);
    }
    let mut i = 0;
    while i < n {
        s.players[i].x = x[i];
        s.players[i].y = y[i];
        s.players[i].speed = 0;
        i += 1;
    }
    if !placement_ok(&s) {
        return Err(Error::Occupied);
    }
    s.paths = Vec::new();
    Ok(s)
}

fn alloc(s: &mut Snapshot) -> Result<u64, Error> {
    if s.meta.next_id == u64::MAX {
        return Err(Error::Limit);
    }
    let id = s.meta.next_id;
    s.meta.next_id += 1;
    Ok(id)
}

macro_rules! ok {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return Err(e),
        }
    };
}

fn drop_players(rows: &Vec<Player>, id: u64) -> Vec<Player> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if rows[i].id != id {
            out.push(rows[i].clone());
        }
        i += 1;
    }
    out
}

fn drop_agents_for(rows: &Vec<Agent>, player: u64) -> Vec<Agent> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if rows[i].player != player {
            out.push(rows[i].clone());
        }
        i += 1;
    }
    out
}

fn halt(s: &mut Snapshot, id: u64) {
    s.paths = drop_paths(&s.paths, id);
    let Some(i) = player_index(&s.players, id) else {
        return;
    };
    stop_motion(&mut s.players[i]);
}

fn drop_paths(paths: &Vec<PathNode>, player: u64) -> Vec<PathNode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < paths.len() {
        if paths[i].player != player {
            out.push(paths[i].clone());
        }
        i += 1;
    }
    out
}

fn replace_path(s: &mut Snapshot, player: u64, nodes: Vec<PathNode>) {
    s.paths = drop_paths(&s.paths, player);
    let mut i = 0;
    while i < nodes.len() {
        s.paths.push(nodes[i].clone());
        i += 1;
    }
}

fn other_positions(s: &Snapshot, ignore: u64, ox: &mut Vec<u32>, oy: &mut Vec<u32>) {
    let mut i = 0;
    while i < s.players.len() {
        if s.players[i].id != ignore {
            ox.push(s.players[i].x);
            oy.push(s.players[i].y);
        }
        i += 1;
    }
}

fn is_talking(s: &Snapshot, player: u64) -> bool {
    match member_index(&s.members, player) {
        Some(i) => s.members[i].status == ST_TALKING,
        None => false,
    }
}

pub fn begin_move(s: &mut Snapshot, id: u64, dest_tx: u32, dest_ty: u32, now: u64, allow_talking: bool, touch_input: bool) -> Result<(), Error> {
    let idx = match player_index(&s.players, id) {
        Some(i) => i,
        None => return Err(Error::NotFound),
    };
    if !allow_talking && is_talking(s, id) {
        return Err(Error::Busy);
    }
    if dest_tx >= s.meta.width || dest_ty >= s.meta.height {
        return Err(Error::BadState);
    }
    let dest_x = dest_tx * MILLI;
    let dest_y = dest_ty * MILLI;
    if touch_input {
        s.players[idx].last_input = now;
    }
    if s.players[idx].x == dest_x && s.players[idx].y == dest_y {
        halt(s, id);
        return Ok(());
    }
    let mut ox = Vec::new();
    let mut oy = Vec::new();
    other_positions(s, id, &mut ox, &mut oy);
    let sx = s.players[idx].x;
    let sy = s.players[idx].y;
    s.players[idx].pf_started = now;
    s.players[idx].activity = 0;
    s.players[idx].activity_until = 0;
    match find_route(&s.terrain.mask, s.meta.width, s.meta.height, &ox, &oy, id, sx, sy, dest_tx, dest_ty, now) {
        None => {
            halt(s, id);
            Ok(())
        }
        Some(route) => {
            let idx = match player_index(&s.players, id) {
                Some(i) => i,
                None => return Err(Error::NotFound),
            };
            s.players[idx].pf = PF_MOVE;
            s.players[idx].dest_x = route.dest_x;
            s.players[idx].dest_y = route.dest_y;
            s.players[idx].speed = 0;
            s.players[idx].wait_until = 0;
            replace_path(s, id, route.nodes);
            Ok(())
        }
    }
}

fn spawn_at(s: &Snapshot, salt: u64) -> Option<(u32, u32)> {
    let w = s.meta.width as u64;
    let h = s.meta.height as u64;
    if w == 0 || h == 0 {
        return None;
    }
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut i = 0;
    while i < s.players.len() {
        xs.push(s.players[i].x);
        ys.push(s.players[i].y);
        i += 1;
    }
    let near = geom::near_index(s.meta.width, s.meta.height, &xs, &ys);
    let n = w * h;
    let mut k = 0;
    while k < n {
        let idx = (salt + k) % n;
        let tx = (idx / h) as u32;
        let ty = (idx % h) as u32;
        let x = tx * MILLI;
        let y = ty * MILLI;
        // Same blocked-tile test as `occupied`. The player scan is the grid.
        let blocked = geom::mask_blocked(&s.terrain.mask, s.meta.width, s.meta.height, x, y)
            || geom::near_hit(&near, &xs, &ys, x, y, u32::MAX);
        if !blocked {
            return Some((x, y));
        }
        k += 1;
    }
    None
}

fn face_salt(salt: u64) -> (u32, u32) {
    match salt % 4 {
        0 => (geom::face_east(), geom::face_zero()),
        1 => (geom::face_west(), geom::face_zero()),
        2 => (geom::face_zero(), geom::face_south()),
        _ => (geom::face_zero(), geom::face_north()),
    }
}

fn stop_conv(s: &mut Snapshot, conv: u64, now: u64) {
    let mut players = Vec::new();
    let mut i = 0;
    while i < s.members.len() {
        if s.members[i].conversation == conv {
            players.push(s.members[i].player);
        }
        i += 1;
    }
    i = 0;
    while i < s.agents.len() {
        let mut j = 0;
        while j < players.len() {
            if s.agents[i].player == players[j] {
                s.agents[i].last_conversation = now;
                s.agents[i].to_remember = conv;
            }
            j += 1;
        }
        i += 1;
    }
    let mut members = Vec::new();
    i = 0;
    while i < s.members.len() {
        if s.members[i].conversation != conv {
            members.push(s.members[i].clone());
        }
        i += 1;
    }
    s.members = members;
    let mut convs = Vec::new();
    i = 0;
    while i < s.conversations.len() {
        if s.conversations[i].id != conv {
            convs.push(s.conversations[i].clone());
        }
        i += 1;
    }
    s.conversations = convs;
}

fn remove_player(s: &mut Snapshot, id: u64, now: u64) {
    if let Some(i) = member_index(&s.members, id) {
        let conv = s.members[i].conversation;
        stop_conv(s, conv, now);
    }
    s.paths = drop_paths(&s.paths, id);
    s.players = drop_players(&s.players, id);
    s.agents = drop_agents_for(&s.agents, id);
}

struct TickScratch {
    xs: Vec<u32>,
    ys: Vec<u32>,
    start: Vec<u32>,
    end: Vec<u32>,
    grouped: bool,
    near: geom::NearIndex,
    clear: Vec<u64>,
    nclear: usize,
    moving: Vec<u32>,
    cand_x: Vec<u32>,
    cand_y: Vec<u32>,
    cand_fx: Vec<u32>,
    cand_fy: Vec<u32>,
    cand_speed: Vec<u32>,
    ids: Vec<u64>,
    prop_x: Vec<u32>,
    prop_y: Vec<u32>,
    prop_on: Vec<u32>,
}

fn fresh_scratch() -> TickScratch {
    TickScratch {
        xs: Vec::new(),
        ys: Vec::new(),
        start: Vec::new(),
        end: Vec::new(),
        grouped: false,
        near: geom::NearIndex { width: 0, height: 0, head: Vec::new(), next: Vec::new() },
        clear: Vec::new(),
        nclear: 0,
        moving: Vec::new(),
        cand_x: Vec::new(),
        cand_y: Vec::new(),
        cand_fx: Vec::new(),
        cand_fy: Vec::new(),
        cand_speed: Vec::new(),
        ids: Vec::new(),
        prop_x: Vec::new(),
        prop_y: Vec::new(),
        prop_on: Vec::new(),
    }
}

/// Default tick. Movement is the two-phase round in `step::tick_round`.
pub fn run_ticks(s: &mut Snapshot, end_ms: u64) -> u32 {
    run_ticks_mode(s, end_ms, false)
}

/// The original AI Town tick. Each agent sees landings accepted earlier in
/// the same tick. `AI_TOWN_TICK=seq` selects this from the shell.
pub fn run_ticks_seq(s: &mut Snapshot, end_ms: u64) -> u32 {
    run_ticks_mode(s, end_ms, true)
}

fn run_ticks_mode(s: &mut Snapshot, end_ms: u64, sequential: bool) -> u32 {
    if s.meta.frozen || s.meta.width == 0 {
        return 0;
    }
    let mut ticks = 0u32;
    s.meta.pathfinds = 0;
    let mut scratch = fresh_scratch();
    while ticks < MAX_TICKS && s.meta.now_ms.saturating_add(TICK_MS) <= end_ms {
        s.meta.now_ms += TICK_MS;
        tick(s, s.meta.now_ms, &mut scratch, sequential);
        ticks += 1;
    }
    ticks
}

fn tick(s: &mut Snapshot, now: u64, scratch: &mut TickScratch, sequential: bool) {
    idle(s, now);
    agents(s, now);
    conversations(s, now);
    pathfind(s, now);
    if sequential {
        positions_seq(s, now, scratch);
    } else {
        positions_round(s, now, scratch);
    }
}

fn idle(s: &mut Snapshot, now: u64) {
    let mut doomed = Vec::new();
    let mut i = 0;
    while i < s.players.len() {
        let p = &s.players[i];
        if p.is_human && now > p.last_input && now - p.last_input > IDLE_MS {
            doomed.push(p.id);
        }
        i += 1;
    }
    i = 0;
    while i < doomed.len() {
        remove_player(s, doomed[i], now);
        i += 1;
    }
}

fn agents(s: &mut Snapshot, now: u64) {
    let mut i = 0;
    while i < s.agents.len() {
        tick_agent(s, i, now);
        i += 1;
    }
}

fn tick_agent(s: &mut Snapshot, i: usize, now: u64) {
    if i >= s.agents.len() {
        return;
    }
    let agent_id = s.agents[i].id;
    let player_id = s.agents[i].player;
    if s.agents[i].op != OP_NONE {
        if now < s.agents[i].op_since.saturating_add(ACTION_TIMEOUT_MS) {
            return;
        }
        s.agents[i].op = OP_NONE;
    }
    if s.agents[i].to_remember != 0 && s.agents[i].op == OP_NONE {
        s.agents[i].op = OP_REMEMBER;
        s.agents[i].op_arg = s.agents[i].to_remember;
        s.agents[i].op_since = now;
        return;
    }
    let Some(mi) = member_index(&s.members, player_id) else {
        let pi = player_at(&s.players, player_id);
        let busy = match pi {
            Some(p) => s.players[p].activity_until > now || s.players[p].pf != PF_NONE,
            None => true,
        };
        if !busy && s.agents[i].op == OP_NONE {
            let last = s.agents[i].last_invite;
            let deadline = last.saturating_add(CONV_COOLDOWN_MS);
            if last == 0 || now >= deadline {
                s.agents[i].op = OP_DO;
                s.agents[i].op_since = now;
                s.agents[i].last_invite = now;
            }
        }
        return;
    };
    let status = s.members[mi].status;
    let conv = s.members[mi].conversation;
    let invited = s.members[mi].invited;
    let started = s.members[mi].started;
    let other = other_member(s, conv, player_id);
    if status == ST_INVITED {
        let other_human = match other {
            Some(id) => match player_at(&s.players, id) {
                Some(p) => s.players[p].is_human,
                None => false,
            },
            None => false,
        };
        let accept = other_human || (agent_id + player_id) % 5 != 0;
        if accept {
            if let Some(mi) = member_index(&s.members, player_id) {
                s.members[mi].status = ST_WALKING;
            }
            halt(s, player_id);
        } else if let Some(id) = conv_index(&s.conversations, conv) {
            let _ = id;
            stop_conv(s, conv, now);
        }
        return;
    }
    if status == ST_WALKING {
        if now > invited && now - invited > INVITE_MS {
            stop_conv(s, conv, now);
            return;
        }
        let Some(oid) = other else {
            return;
        };
        let Some(pi) = player_at(&s.players, player_id) else {
            return;
        };
        let Some(oi) = player_at(&s.players, oid) else {
            return;
        };
        let d = geom::dist_sq(s.players[pi].x, s.players[pi].y, s.players[oi].x, s.players[oi].y);
        if d < TALK_SQ || s.players[pi].pf != PF_NONE {
            return;
        }
        let (dx, dy) = walk_target(s.players[pi].x, s.players[pi].y, s.players[oi].x, s.players[oi].y);
        let _ = begin_move(s, player_id, dx, dy, now, true, false);
        return;
    }
    if status == ST_TALKING {
        let msgs = match conv_index(&s.conversations, conv) {
            Some(c) => s.conversations[c].num_messages,
            None => 0,
        };
        if msgs >= MAX_MESSAGES || (started > 0 && now > started && now - started > MAX_CONV_MS) {
            stop_conv(s, conv, now);
            return;
        }
        let Some(c) = conv_index(&s.conversations, conv) else {
            return;
        };
        if s.conversations[c].typing_player != 0 && s.conversations[c].typing_player != player_id {
            return;
        }
        let creator = s.conversations[c].creator;
        let last_at = s.conversations[c].last_at;
        let deadline = last_at.saturating_add(MESSAGE_COOLDOWN_MS);
        // Nested `let speak = if { saturating_add }` extracts as a proposition.
        if s.conversations[c].num_messages == 0 {
            if creator == player_id {
                queue_speak(s, agent_id, conv, now);
            } else if started > 0 {
                if now > started {
                    let waited = now - started;
                    if waited > AWKWARD_MS {
                        queue_speak(s, agent_id, conv, now);
                    }
                }
            }
        } else if now >= deadline {
            queue_speak(s, agent_id, conv, now);
        }
    }
}

fn queue_speak(s: &mut Snapshot, agent_id: u64, conv: u64, now: u64) {
    if agent_op(s, agent_id) != OP_NONE {
        return;
    }
    let Some(ai) = agent_index(s, agent_id) else {
        return;
    };
    s.agents[ai].op = OP_SPEAK;
    s.agents[ai].op_since = now;
    s.agents[ai].op_arg = conv;
}

fn agent_index(s: &Snapshot, id: u64) -> Option<usize> {
    let mut i = 0;
    while i < s.agents.len() {
        if s.agents[i].id == id {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn agent_op(s: &Snapshot, id: u64) -> u32 {
    match agent_index(s, id) {
        Some(i) => s.agents[i].op,
        None => OP_NONE,
    }
}

fn other_member(s: &Snapshot, conv: u64, player: u64) -> Option<u64> {
    let mut i = 0;
    while i < s.members.len() {
        if s.members[i].conversation == conv && s.members[i].player != player {
            return Some(s.members[i].player);
        }
        i += 1;
    }
    None
}

fn walk_target(ax: u32, ay: u32, bx: u32, by: u32) -> (u32, u32) {
    let far = geom::dist_sq(ax, ay, bx, by) >= 4_000u64 * 4_000;
    if far {
        let mx = (ax / MILLI + bx / MILLI) / 2;
        let my = (ay / MILLI + by / MILLI) / 2;
        (mx, my)
    } else {
        (bx / MILLI, by / MILLI)
    }
}

fn conversations(s: &mut Snapshot, now: u64) {
    let mut ids = Vec::new();
    let mut i = 0;
    while i < s.conversations.len() {
        ids.push(s.conversations[i].id);
        i += 1;
    }
    i = 0;
    while i < ids.len() {
        tick_conv(s, ids[i], now);
        i += 1;
    }
}

fn tick_conv(s: &mut Snapshot, conv: u64, now: u64) {
    let Some(ci) = conv_index(&s.conversations, conv) else {
        return;
    };
    if s.conversations[ci].typing_player != 0 && now > s.conversations[ci].typing_since && now - s.conversations[ci].typing_since > TYPING_MS {
        s.conversations[ci].typing_player = 0;
    }
    let mut a = 0u64;
    let mut b = 0u64;
    let mut n = 0;
    let mut i = 0;
    while i < s.members.len() {
        if s.members[i].conversation == conv {
            if n == 0 {
                a = s.members[i].player;
            } else if n == 1 {
                b = s.members[i].player;
            }
            n += 1;
        }
        i += 1;
    }
    if n != 2 {
        return;
    }
    let (Some(ia), Some(ib)) = (member_index(&s.members, a), member_index(&s.members, b)) else {
        return;
    };
    let sa = s.members[ia].status;
    let sb = s.members[ib].status;
    let (Some(pa), Some(pb)) = (player_index(&s.players, a), player_index(&s.players, b)) else {
        return;
    };
    let d = geom::dist_sq(s.players[pa].x, s.players[pa].y, s.players[pb].x, s.players[pb].y);
    if sa == ST_WALKING && sb == ST_WALKING && d < TALK_SQ {
        if let Some(ia) = member_index(&s.members, a) {
            s.members[ia].status = ST_TALKING;
            s.members[ia].started = now;
        }
        if let Some(ib) = member_index(&s.members, b) {
            s.members[ib].status = ST_TALKING;
            s.members[ib].started = now;
        }
        halt(s, a);
        halt(s, b);
        face_pair(s, a, b);
        return;
    }
    if sa == ST_TALKING && sb == ST_TALKING {
        let moving = match (player_index(&s.players, a), player_index(&s.players, b)) {
            (Some(pa), Some(pb)) => s.players[pa].pf != PF_NONE || s.players[pb].pf != PF_NONE,
            _ => true,
        };
        if !moving {
            face_pair(s, a, b);
        }
    }
}

fn face_pair(s: &mut Snapshot, a: u64, b: u64) {
    let (Some(pa), Some(pb)) = (player_index(&s.players, a), player_index(&s.players, b)) else {
        return;
    };
    let (fdx, fdy) = geom::face_toward(s.players[pa].x, s.players[pa].y, s.players[pb].x, s.players[pb].y);
    s.players[pa].face_dx = fdx;
    s.players[pa].face_dy = fdy;
    let (fdx, fdy) = geom::face_toward(s.players[pb].x, s.players[pb].y, s.players[pa].x, s.players[pa].y);
    s.players[pb].face_dx = fdx;
    s.players[pb].face_dy = fdy;
}

fn pathfind(s: &mut Snapshot, now: u64) {
    let mut i = 0;
    while i < s.players.len() {
        pathfind_one(s, i, now);
        i += 1;
    }
}

fn pathfind_one(s: &mut Snapshot, i: usize, now: u64) {
    if i >= s.players.len() {
        return;
    }
    let id = s.players[i].id;
    if s.players[i].pf == PF_NONE {
        return;
    }
    if s.players[i].pf == PF_MOVE && s.players[i].x == s.players[i].dest_x && s.players[i].y == s.players[i].dest_y {
        halt(s, id);
        return;
    }
    if now > s.players[i].pf_started && now - s.players[i].pf_started > PATH_TIMEOUT_MS {
        halt(s, id);
        return;
    }
    if s.players[i].pf == PF_WAIT && s.players[i].wait_until <= now {
        s.players[i].pf = PF_NEED;
    }
    if s.players[i].pf == PF_NEED && s.meta.pathfinds < MAX_PATHFINDS {
        s.meta.pathfinds += 1;
        let tx = s.players[i].dest_x / MILLI;
        let ty = s.players[i].dest_y / MILLI;
        let _ = begin_move(s, id, tx, ty, now, true, false);
    }
}

fn positions_seq(s: &mut Snapshot, now: u64, scratch: &mut TickScratch) {
    let limit = match id_limit_one(s) {
        Some(n) => n,
        None => 0,
    };
    scratch.grouped = if limit == 0 { false } else { fill_path_spans(&s.paths, limit, &mut scratch.start, &mut scratch.end) };
    let n = s.players.len();
    while scratch.xs.len() < n {
        scratch.xs.push(0);
    }
    while scratch.ys.len() < n {
        scratch.ys.push(0);
    }
    let mut i = 0;
    while i < n {
        scratch.xs[i] = s.players[i].x;
        scratch.ys[i] = s.players[i].y;
        i += 1;
    }
    geom::near_fill(&mut scratch.near, s.meta.width, s.meta.height, &scratch.xs[..n], &scratch.ys[..n]);
    scratch.nclear = 0;
    i = 0;
    while i < s.players.len() {
        move_one(s, i, now, scratch);
        i += 1;
    }
    if scratch.nclear > 0 {
        let nclear = scratch.nclear;
        s.paths = drop_path_ids(&s.paths, &scratch.clear[..nclear]);
    }
}

fn grow_u32(v: &mut Vec<u32>, n: usize) {
    while v.len() < n {
        v.push(0);
    }
}

fn grow_u64(v: &mut Vec<u64>, n: usize) {
    while v.len() < n {
        v.push(0);
    }
}

/// Two-phase movement. A rejected walker waits. A missing path does not.
fn positions_round(s: &mut Snapshot, now: u64, scratch: &mut TickScratch) {
    let limit = match id_limit_one(s) {
        Some(n) => n,
        None => 0,
    };
    scratch.grouped = if limit == 0 { false } else { fill_path_spans(&s.paths, limit, &mut scratch.start, &mut scratch.end) };
    let n = s.players.len();
    grow_u32(&mut scratch.xs, n);
    grow_u32(&mut scratch.ys, n);
    grow_u32(&mut scratch.moving, n);
    grow_u32(&mut scratch.cand_x, n);
    grow_u32(&mut scratch.cand_y, n);
    grow_u32(&mut scratch.cand_fx, n);
    grow_u32(&mut scratch.cand_fy, n);
    grow_u32(&mut scratch.cand_speed, n);
    grow_u64(&mut scratch.ids, n);
    grow_u32(&mut scratch.prop_x, n);
    grow_u32(&mut scratch.prop_y, n);
    grow_u32(&mut scratch.prop_on, n);
    let cells = (s.meta.width as usize) * (s.meta.height as usize);
    while scratch.near.head.len() < cells {
        scratch.near.head.push(u32::MAX);
    }
    while scratch.near.next.len() < n {
        scratch.near.next.push(u32::MAX);
    }
    let mut i = 0;
    while i < n {
        scratch.xs[i] = s.players[i].x;
        scratch.ys[i] = s.players[i].y;
        scratch.ids[i] = s.players[i].id;
        scratch.moving[i] = 0;
        scratch.cand_x[i] = s.players[i].x;
        scratch.cand_y[i] = s.players[i].y;
        scratch.cand_fx[i] = s.players[i].face_dx;
        scratch.cand_fy[i] = s.players[i].face_dy;
        scratch.cand_speed[i] = 0;
        if s.players[i].pf == PF_MOVE {
            if let Some(pose) = path_pose_at(s, i, now, scratch) {
                scratch.moving[i] = 1;
                scratch.cand_x[i] = pose.x;
                scratch.cand_y[i] = pose.y;
                scratch.cand_fx[i] = pose.face_dx;
                scratch.cand_fy[i] = pose.face_dy;
                scratch.cand_speed[i] = pose.speed;
            }
        }
        i += 1;
    }
    let ok = crate::step::tick_round(
        &mut scratch.xs[..n],
        &mut scratch.ys[..n],
        &scratch.moving[..n],
        &scratch.cand_x[..n],
        &scratch.cand_y[..n],
        &scratch.ids[..n],
        &s.terrain.mask,
        s.meta.width,
        s.meta.height,
        &mut scratch.near.head[..cells],
        &mut scratch.near.next[..n],
        &mut scratch.prop_x[..n],
        &mut scratch.prop_y[..n],
        &mut scratch.prop_on[..n],
    );
    if !ok {
        return;
    }
    scratch.nclear = 0;
    i = 0;
    while i < n {
        if scratch.prop_on[i] != 0 {
            s.players[i].x = scratch.xs[i];
            s.players[i].y = scratch.ys[i];
            s.players[i].face_dx = scratch.cand_fx[i];
            s.players[i].face_dy = scratch.cand_fy[i];
            s.players[i].speed = scratch.cand_speed[i];
            if s.players[i].x == s.players[i].dest_x && s.players[i].y == s.players[i].dest_y {
                let id = s.players[i].id;
                stop_motion(&mut s.players[i]);
                if scratch.nclear < scratch.clear.len() {
                    scratch.clear[scratch.nclear] = id;
                } else {
                    scratch.clear.push(id);
                }
                scratch.nclear += 1;
            }
        } else if scratch.moving[i] != 0 {
            s.players[i].pf = PF_WAIT;
            let stagger = s.players[i].id % 1000;
            s.players[i].wait_until = now.wrapping_add(1).wrapping_add(stagger);
            s.players[i].speed = 0;
        } else {
            s.players[i].speed = 0;
        }
        i += 1;
    }
    if scratch.nclear > 0 {
        let nclear = scratch.nclear;
        s.paths = drop_path_ids(&s.paths, &scratch.clear[..nclear]);
    }
}

fn id_limit_one(s: &Snapshot) -> Option<usize> {
    let n = s.meta.next_id;
    if n == 0 || n > 2_000_000 {
        None
    } else {
        Some(n as usize)
    }
}

fn id_limit(a: &Snapshot, b: &Snapshot) -> Option<usize> {
    let n = if a.meta.next_id > b.meta.next_id { a.meta.next_id } else { b.meta.next_id };
    if n == 0 || n > 2_000_000 {
        None
    } else {
        Some(n as usize)
    }
}

/// True when every player's nodes are one contiguous run.
/// `start[id] == u32::MAX` means that player has no nodes.
fn fill_path_spans(paths: &Vec<PathNode>, limit: usize, start: &mut Vec<u32>, end: &mut Vec<u32>) -> bool {
    while start.len() < limit {
        start.push(u32::MAX);
    }
    while end.len() < limit {
        end.push(0);
    }
    let mut i = 0;
    while i < limit {
        start[i] = u32::MAX;
        end[i] = 0;
        i += 1;
    }
    let mut ok = true;
    i = 0;
    while i < paths.len() {
        let id = paths[i].player as usize;
        if id >= limit {
            ok = false;
        } else if start[id] == u32::MAX {
            start[id] = i as u32;
            end[id] = (i as u32).wrapping_add(1);
        } else if end[id] != i as u32 {
            ok = false;
        } else {
            end[id] = (i as u32).wrapping_add(1);
        }
        i += 1;
    }
    ok
}

fn path_spans(paths: &Vec<PathNode>, limit: usize) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut start = Vec::new();
    let mut end = Vec::new();
    if fill_path_spans(paths, limit, &mut start, &mut end) {
        Some((start, end))
    } else {
        None
    }
}

fn drop_path_ids(paths: &Vec<PathNode>, ids: &[u64]) -> Vec<PathNode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < paths.len() {
        let mut drop = false;
        let mut j = 0;
        while j < ids.len() {
            if paths[i].player == ids[j] {
                drop = true;
            }
            j += 1;
        }
        if !drop {
            out.push(paths[i].clone());
        }
        i += 1;
    }
    out
}

fn path_of(paths: &Vec<PathNode>, player: u64) -> Vec<PathNode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < paths.len() {
        if paths[i].player == player {
            out.push(paths[i].clone());
        }
        i += 1;
    }
    out
}

fn path_pose_at(s: &Snapshot, i: usize, now: u64, scratch: &TickScratch) -> Option<geom::Pose> {
    let id = s.players[i].id;
    if scratch.grouped {
        let idn = id as usize;
        if idn < scratch.start.len() && idn < scratch.end.len() && scratch.start[idn] != u32::MAX {
            let end_i = scratch.end[idn] as usize;
            let hi = if end_i < s.paths.len() { end_i } else { s.paths.len() };
            let start_i = scratch.start[idn] as usize;
            let lo = if start_i < hi { start_i } else { hi };
            path_pose(&s.paths[lo..hi], now)
        } else {
            None
        }
    } else {
        let path = path_of(&s.paths, id);
        path_pose(&path, now)
    }
}

fn move_one(s: &mut Snapshot, i: usize, now: u64, scratch: &mut TickScratch) {
    if i >= s.players.len() {
        return;
    }
    let id = s.players[i].id;
    if s.players[i].pf != PF_MOVE {
        s.players[i].speed = 0;
        return;
    }
    let pose = path_pose_at(s, i, now, scratch);
    let Some(pose) = pose else {
        s.players[i].speed = 0;
        return;
    };
    // Same accept/refuse as `agent_step`: the tile mask, then the 3 by 3 grid.
    // `xs`/`ys` already include landings accepted earlier in this tick.
    let landing = crate::step::agent_step(
        s.players[i].x,
        s.players[i].y,
        s.players[i].face_dx,
        s.players[i].face_dy,
        true,
        pose.x,
        pose.y,
        pose.face_dx,
        pose.face_dy,
        pose.speed,
        &s.terrain.mask,
        s.meta.width,
        s.meta.height,
        &scratch.near.head,
        &scratch.near.next,
        &scratch.xs,
        &scratch.ys,
        i as u32,
    );
    if !landing.moved {
        s.players[i].pf = PF_WAIT;
        // `BACKOFF_MS` is 1000. A checked `+` of the remainder stays
        // `AddChecked`, which does not translate; wrapping add does, and
        // these timestamps sit far below `u64::MAX`.
        let stagger = id % 1000;
        s.players[i].wait_until = now.wrapping_add(1).wrapping_add(stagger);
        s.players[i].speed = 0;
        return;
    }
    s.players[i].x = landing.x;
    s.players[i].y = landing.y;
    s.players[i].face_dx = landing.face_dx;
    s.players[i].face_dy = landing.face_dy;
    s.players[i].speed = landing.speed;
    if i < scratch.xs.len() {
        scratch.xs[i] = landing.x;
    }
    if i < scratch.ys.len() {
        scratch.ys[i] = landing.y;
    }
    if landing.x == s.players[i].dest_x && landing.y == s.players[i].dest_y {
        // Drop the path after the whole tick so the spans above stay valid.
        stop_motion(&mut s.players[i]);
        if scratch.nclear < scratch.clear.len() {
            scratch.clear[scratch.nclear] = id;
        } else {
            scratch.clear.push(id);
        }
        scratch.nclear += 1;
    }
}

fn stop_motion(p: &mut Player) {
    p.pf = PF_NONE;
    p.dest_x = 0;
    p.dest_y = 0;
    p.speed = 0;
    p.wait_until = 0;
}

fn require_world(s: &Snapshot) -> Result<(), Error> {
    if s.meta.width == 0 {
        Err(Error::Uninit)
    } else {
        Ok(())
    }
}

fn dispatch_init(s: &mut Snapshot, width: u32, height: u32, mask: &Vec<u8>, now: u64) -> Result<Reply, Error> {
    if s.meta.width != 0 {
        return Err(Error::Already);
    }
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(Error::BadState);
    }
    let cells = (width as usize) * (height as usize);
    if mask.len() != geom::mask_bytes(cells) {
        return Err(Error::BadState);
    }
    s.meta.width = width;
    s.meta.height = height;
    s.meta.next_id = 1;
    s.meta.now_ms = now;
    s.terrain.mask = mask.clone();
    Ok(Reply::Ok)
}

fn dispatch_agent(s: &mut Snapshot, name: &Vec<u8>, character: u32, identity: &Vec<u8>, plan: &Vec<u8>, now: u64) -> Result<Reply, Error> {
    ok!(require_world(s));
    if !text_ok(name, MAX_NAME) || character >= 8 || !text_ok(identity, MAX_PLAN) || !text_ok(plan, MAX_PLAN) {
        return Err(Error::BadText);
    }
    let id = ok!(alloc(s));
    let Some((x, y)) = spawn_at(s, id) else {
        return Err(Error::Occupied);
    };
    let (fdx, fdy) = face_salt(id);
    let owner = AGENT_BASE + id;
    s.players.push(Player {
        id,
        owner,
        is_human: false,
        x,
        y,
        face_dx: fdx,
        face_dy: fdy,
        speed: 0,
        dest_x: 0,
        dest_y: 0,
        pf: PF_NONE,
        pf_started: now,
        wait_until: 0,
        last_input: now,
        activity: 0,
        activity_until: 0,
        character,
        name: name.clone(),
    });
    let aid = ok!(alloc(s));
    s.agents.push(Agent {
        id: aid,
        player: id,
        owner,
        to_remember: 0,
        last_conversation: 0,
        last_invite: 0,
        op: OP_NONE,
        op_since: 0,
        op_arg: 0,
        character,
        name: name.clone(),
        identity: identity.clone(),
        plan: plan.clone(),
    });
    Ok(Reply::Player(id))
}

fn human_count(s: &Snapshot) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < s.players.len() {
        if s.players[i].is_human {
            n += 1;
        }
        i += 1;
    }
    n
}

fn dispatch_join(s: &mut Snapshot, owner: u64, name: &Vec<u8>, character: u32, now: u64) -> Result<Reply, Error> {
    ok!(require_world(s));
    if owner == ENGINE || owner >= AGENT_BASE {
        return Err(Error::Forbidden);
    }
    if player_by_owner(&s.players, owner).is_some() {
        return Err(Error::Already);
    }
    if human_count(s) >= MAX_HUMANS {
        return Err(Error::Full);
    }
    if !text_ok(name, MAX_NAME) || character >= 8 {
        return Err(Error::BadText);
    }
    let id = ok!(alloc(s));
    let Some((x, y)) = spawn_at(s, owner) else {
        return Err(Error::Occupied);
    };
    let (fdx, fdy) = face_salt(owner);
    s.players.push(Player {
        id,
        owner,
        is_human: true,
        x,
        y,
        face_dx: fdx,
        face_dy: fdy,
        speed: 0,
        dest_x: 0,
        dest_y: 0,
        pf: PF_NONE,
        pf_started: now,
        wait_until: 0,
        last_input: now,
        activity: 0,
        activity_until: 0,
        character,
        name: name.clone(),
    });
    Ok(Reply::Player(id))
}

fn own_player(s: &Snapshot, owner: u64) -> Result<u64, Error> {
    match player_by_owner(&s.players, owner) {
        Some(i) => Ok(s.players[i].id),
        None => Err(Error::NotFound),
    }
}

fn dispatch_leave(s: &mut Snapshot, owner: u64, now: u64) -> Result<Reply, Error> {
    let id = ok!(own_player(s, owner));
    remove_player(s, id, now);
    Ok(Reply::Ok)
}

fn dispatch_move(s: &mut Snapshot, owner: u64, x: u32, y: u32, now: u64) -> Result<Reply, Error> {
    ok!(require_world(s));
    let id = ok!(own_player(s, owner));
    ok!(begin_move(s, id, x, y, now, false, true));
    Ok(Reply::Ok)
}

fn dispatch_stop(s: &mut Snapshot, owner: u64, now: u64) -> Result<Reply, Error> {
    let id = ok!(own_player(s, owner));
    halt(s, id);
    if let Some(i) = player_index(&s.players, id) {
        s.players[i].last_input = now;
    }
    Ok(Reply::Ok)
}

fn dispatch_start(s: &mut Snapshot, owner: u64, invitee: u64, now: u64) -> Result<Reply, Error> {
    let id = ok!(own_player(s, owner));
    if id == invitee {
        return Err(Error::BadState);
    }
    if player_index(&s.players, invitee).is_none() {
        return Err(Error::NotFound);
    }
    if member_index(&s.members, id).is_some() || member_index(&s.members, invitee).is_some() {
        return Err(Error::Busy);
    }
    let cid = ok!(alloc(s));
    s.conversations.push(Conversation {
        id: cid,
        creator: id,
        created: now,
        typing_player: 0,
        typing_since: 0,
        last_author: 0,
        last_at: 0,
        num_messages: 0,
    });
    s.members.push(Member { conversation: cid, player: id, invited: now, status: ST_WALKING, started: 0 });
    s.members.push(Member { conversation: cid, player: invitee, invited: now, status: ST_INVITED, started: 0 });
    if let Some(i) = player_index(&s.players, id) {
        s.players[i].last_input = now;
    }
    Ok(Reply::Conversation(cid))
}

fn own_member(s: &Snapshot, owner: u64) -> Result<Member, Error> {
    let id = ok!(own_player(s, owner));
    match member_index(&s.members, id) {
        Some(i) => Ok(s.members[i].clone()),
        None => Err(Error::NotFound),
    }
}

fn dispatch_accept(s: &mut Snapshot, owner: u64, now: u64) -> Result<Reply, Error> {
    let id = ok!(own_player(s, owner));
    let m = ok!(own_member(s, owner));
    if m.status != ST_INVITED {
        return Err(Error::BadState);
    }
    if let Some(i) = member_index(&s.members, id) {
        s.members[i].status = ST_WALKING;
    }
    halt(s, id);
    if let Some(i) = player_index(&s.players, id) {
        s.players[i].last_input = now;
    }
    Ok(Reply::Ok)
}

fn dispatch_reject(s: &mut Snapshot, owner: u64, now: u64) -> Result<Reply, Error> {
    let m = ok!(own_member(s, owner));
    if m.status != ST_INVITED {
        return Err(Error::BadState);
    }
    stop_conv(s, m.conversation, now);
    if let Some(i) = player_by_owner(&s.players, owner) {
        s.players[i].last_input = now;
    }
    Ok(Reply::Ok)
}

fn dispatch_leave_conv(s: &mut Snapshot, owner: u64, now: u64) -> Result<Reply, Error> {
    let m = ok!(own_member(s, owner));
    stop_conv(s, m.conversation, now);
    if let Some(i) = player_by_owner(&s.players, owner) {
        s.players[i].last_input = now;
    }
    Ok(Reply::Ok)
}

fn dispatch_typing(s: &mut Snapshot, owner: u64, now: u64) -> Result<Reply, Error> {
    let id = ok!(own_player(s, owner));
    let m = ok!(own_member(s, owner));
    if m.status != ST_TALKING {
        return Err(Error::BadState);
    }
    let Some(ci) = conv_index(&s.conversations, m.conversation) else {
        return Err(Error::NotFound);
    };
    if s.conversations[ci].typing_player != 0 && s.conversations[ci].typing_player != id {
        return Err(Error::Busy);
    }
    s.conversations[ci].typing_player = id;
    s.conversations[ci].typing_since = now;
    Ok(Reply::Ok)
}

fn dispatch_say(s: &mut Snapshot, owner: u64, text: &Vec<u8>, now: u64) -> Result<Reply, Error> {
    if !text_ok(text, MAX_TEXT) {
        return Err(Error::BadText);
    }
    let id = ok!(own_player(s, owner));
    let m = ok!(own_member(s, owner));
    if m.status != ST_TALKING {
        return Err(Error::BadState);
    }
    let Some(ci) = conv_index(&s.conversations, m.conversation) else {
        return Err(Error::NotFound);
    };
    let mid = ok!(alloc(s));
    s.messages.push(Message { id: mid, conversation: m.conversation, author: id, ts: now, text: text.clone() });
    s.conversations[ci].num_messages += 1;
    s.conversations[ci].last_author = id;
    s.conversations[ci].last_at = now;
    if s.conversations[ci].typing_player == id {
        s.conversations[ci].typing_player = 0;
    }
    if let Some(i) = player_index(&s.players, id) {
        s.players[i].last_input = now;
    }
    Ok(Reply::Conversation(mid))
}

fn own_agent(s: &Snapshot, owner: u64) -> Result<usize, Error> {
    let mut i = 0;
    while i < s.agents.len() {
        if s.agents[i].owner == owner {
            return Ok(i);
        }
        i += 1;
    }
    Err(Error::NotFound)
}

fn dispatch_finish(s: &mut Snapshot, owner: u64) -> Result<Reply, Error> {
    let i = ok!(own_agent(s, owner));
    s.agents[i].op = OP_NONE;
    s.agents[i].op_arg = 0;
    Ok(Reply::Ok)
}

fn dispatch_remember(s: &mut Snapshot, owner: u64, summary: &Vec<u8>, now: u64) -> Result<Reply, Error> {
    if !text_ok(summary, MAX_PLAN) {
        return Err(Error::BadText);
    }
    let i = ok!(own_agent(s, owner));
    let agent = s.agents[i].id;
    let conv = s.agents[i].to_remember;
    if conv == 0 {
        return Err(Error::BadState);
    }
    let other = match other_member(s, conv, s.agents[i].player) {
        Some(p) => p,
        None => 0,
    };
    let id = ok!(alloc(s));
    s.memories.push(Memory { id, agent, other, conversation: conv, summary: summary.clone() });
    s.agents[i].to_remember = 0;
    s.agents[i].op = OP_NONE;
    s.agents[i].last_conversation = now;
    Ok(Reply::Ok)
}

fn dispatch_activity(s: &mut Snapshot, owner: u64, kind: u32, now: u64) -> Result<Reply, Error> {
    if kind == 0 || kind > 3 {
        return Err(Error::BadState);
    }
    let id = ok!(own_player(s, owner));
    if is_talking(s, id) {
        return Err(Error::Busy);
    }
    let Some(i) = player_index(&s.players, id) else {
        return Err(Error::NotFound);
    };
    s.players[i].activity = kind;
    s.players[i].activity_until = now.saturating_add(ACTIVITY_MS);
    s.players[i].last_input = now;
    Ok(Reply::Ok)
}

/// Apply `cmd` to a copy of `s` and return the writes that store the result.
pub fn command(actor_user: u64, now: u64, s: &Snapshot, cmd: &Command) -> Result<(Vec<Write>, Reply), Error> {
    let mut next = s.clone();
    let reply = match cmd {
        Command::Init { width, height, mask } => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            ok!(dispatch_init(&mut next, *width, *height, mask, now))
        }
        Command::CreateAgent { name, character, identity, plan } => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            ok!(dispatch_agent(&mut next, name, *character, identity, plan, now))
        }
        Command::Join { name, character } => ok!(dispatch_join(&mut next, actor_user, name, *character, now)),
        Command::Leave => ok!(dispatch_leave(&mut next, actor_user, now)),
        Command::MoveTo { x, y } => ok!(dispatch_move(&mut next, actor_user, *x, *y, now)),
        Command::Stop => ok!(dispatch_stop(&mut next, actor_user, now)),
        Command::StartConversation { invitee } => ok!(dispatch_start(&mut next, actor_user, *invitee, now)),
        Command::AcceptInvite => ok!(dispatch_accept(&mut next, actor_user, now)),
        Command::RejectInvite => ok!(dispatch_reject(&mut next, actor_user, now)),
        Command::LeaveConversation => ok!(dispatch_leave_conv(&mut next, actor_user, now)),
        Command::StartTyping => ok!(dispatch_typing(&mut next, actor_user, now)),
        Command::Say { text } => ok!(dispatch_say(&mut next, actor_user, text, now)),
        Command::FinishOp => ok!(dispatch_finish(&mut next, actor_user)),
        Command::Remember { summary } => ok!(dispatch_remember(&mut next, actor_user, summary, now)),
        Command::Activity { kind } => ok!(dispatch_activity(&mut next, actor_user, *kind, now)),
        Command::Step => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            ok!(require_world(&next));
            let ticks = run_ticks(&mut next, now);
            Reply::Stepped { now_ms: next.meta.now_ms, ticks }
        }
        Command::StepSeq => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            ok!(require_world(&next));
            let ticks = run_ticks_seq(&mut next, now);
            Reply::Stepped { now_ms: next.meta.now_ms, ticks }
        }
        Command::Relocate { x, y } => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            ok!(require_world(&next));
            next = ok!(relocate(next, x, y));
            Reply::Ok
        }
        Command::Freeze => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            next.meta.frozen = true;
            Reply::Ok
        }
        Command::Resume => {
            if actor_user != ENGINE {
                return Err(Error::Forbidden);
            }
            next.meta.frozen = false;
            Reply::Ok
        }
    };
    Ok((diff(s, &next), reply))
}

pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<Write> {
    let mut ws = Vec::new();
    if before.meta != after.meta {
        ws.push(Write::PutMeta(after.meta.clone()));
    }
    if before.terrain != after.terrain {
        ws.push(Write::PutTerrain(after.terrain.clone()));
    }
    diff_player_rows(&before.players, &after.players, &mut ws);
    diff_paths(before, after, &mut ws);
    diff_convs(before, after, &mut ws);
    diff_members(before, after, &mut ws);
    diff_agents(before, after, &mut ws);
    diff_memories(before, after, &mut ws);
    diff_messages(before, after, &mut ws);
    ws
}

fn diff_player_rows(before: &Vec<Player>, after: &Vec<Player>, ws: &mut Vec<Write>) {
    let mut i = 0;
    let mut j = 0;
    while i < before.len() || j < after.len() {
        if i < before.len() && (j == after.len() || before[i].id < after[j].id) {
            ws.push(Write::DelPlayer(before[i].id));
            ws.push(Write::ClearPath(before[i].id));
            i += 1;
        } else if j < after.len() && (i == before.len() || after[j].id < before[i].id) {
            ws.push(Write::PutPlayer(after[j].clone()));
            j += 1;
        } else {
            if before[i] != after[j] {
                ws.push(Write::PutPlayer(after[j].clone()));
            }
            i += 1;
            j += 1;
        }
    }
}

fn diff_paths(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    if let Some(limit) = id_limit(before, after) {
        if let (Some(bspan), Some(aspan)) = (path_spans(&before.paths, limit), path_spans(&after.paths, limit)) {
            diff_paths_indexed(before, after, &bspan, &aspan, limit, ws);
            return;
        }
    }
    diff_paths_scan(before, after, ws);
}

fn diff_paths_indexed(
    before: &Snapshot,
    after: &Snapshot,
    bspan: &(Vec<u32>, Vec<u32>),
    aspan: &(Vec<u32>, Vec<u32>),
    limit: usize,
    ws: &mut Vec<Write>,
) {
    let mut was = Vec::new();
    let mut now = Vec::new();
    let mut i = 0;
    while i < limit {
        was.push(false);
        now.push(false);
        i += 1;
    }
    i = 0;
    while i < before.players.len() {
        let id = before.players[i].id as usize;
        if id < was.len() {
            was[id] = true;
        }
        i += 1;
    }
    i = 0;
    while i < after.players.len() {
        let id = after.players[i].id as usize;
        if id < now.len() {
            now[id] = true;
        }
        i += 1;
    }
    i = 0;
    while i < limit {
        let id = i as u64;
        if was[i] && !now[i] {
            i += 1;
            continue;
        }
        let (b0, b1) = span_at(&bspan.0, &bspan.1, i);
        let (a0, a1) = span_at(&aspan.0, &aspan.1, i);
        if !span_eq(&before.paths, b0, b1, &after.paths, a0, a1) {
            ws.push(Write::ClearPath(id));
            let mut j = a0;
            while j < a1 {
                if (j as usize) >= after.paths.len() {
                    break;
                }
                ws.push(Write::PutPath(after.paths[j as usize].clone()));
                j = j.wrapping_add(1);
            }
        }
        i += 1;
    }
}

fn span_at(start: &Vec<u32>, end: &Vec<u32>, id: usize) -> (u32, u32) {
    if id >= start.len() || id >= end.len() || start[id] == u32::MAX {
        (0, 0)
    } else {
        (start[id], end[id])
    }
}

fn span_eq(a: &Vec<PathNode>, a0: u32, a1: u32, b: &Vec<PathNode>, b0: u32, b1: u32) -> bool {
    if a1 < a0 || b1 < b0 {
        return false;
    }
    let an = a1.wrapping_sub(a0) as usize;
    let bn = b1.wrapping_sub(b0) as usize;
    if an != bn || (a0 as usize) + an > a.len() || (b0 as usize) + bn > b.len() {
        return false;
    }
    let mut k = 0;
    while k < an {
        if a[(a0 as usize) + k] != b[(b0 as usize) + k] {
            return false;
        }
        k += 1;
    }
    true
}

fn diff_paths_scan(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    let mut ids = Vec::new();
    let mut i = 0;
    while i < before.paths.len() {
        push_id(&mut ids, before.paths[i].player);
        i += 1;
    }
    i = 0;
    while i < after.paths.len() {
        push_id(&mut ids, after.paths[i].player);
        i += 1;
    }
    i = 0;
    while i < ids.len() {
        let id = ids[i];
        if player_index(&after.players, id).is_none() && player_index(&before.players, id).is_some() {
            i += 1;
            continue;
        }
        let a = path_of(&before.paths, id);
        let b = path_of(&after.paths, id);
        if a != b {
            ws.push(Write::ClearPath(id));
            let mut j = 0;
            while j < b.len() {
                ws.push(Write::PutPath(b[j].clone()));
                j += 1;
            }
        }
        i += 1;
    }
}

fn push_id(ids: &mut Vec<u64>, id: u64) {
    let mut i = 0;
    while i < ids.len() {
        if ids[i] == id {
            return;
        }
        i += 1;
    }
    ids.push(id);
}

fn diff_convs(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    let mut i = 0;
    while i < before.conversations.len() {
        if conv_index(&after.conversations, before.conversations[i].id).is_none() {
            ws.push(Write::DelConv(before.conversations[i].id));
        }
        i += 1;
    }
    i = 0;
    while i < after.conversations.len() {
        let id = after.conversations[i].id;
        let changed = match conv_index(&before.conversations, id) {
            Some(j) => before.conversations[j] != after.conversations[i],
            None => true,
        };
        if changed {
            ws.push(Write::PutConv(after.conversations[i].clone()));
        }
        i += 1;
    }
}

fn member_key_index(rows: &Vec<Member>, conv: u64, player: u64) -> Option<usize> {
    let mut i = 0;
    while i < rows.len() {
        if rows[i].conversation == conv && rows[i].player == player {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn diff_members(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    let mut i = 0;
    while i < before.members.len() {
        let m = &before.members[i];
        if member_key_index(&after.members, m.conversation, m.player).is_none() {
            ws.push(Write::DelMember(m.conversation, m.player));
        }
        i += 1;
    }
    i = 0;
    while i < after.members.len() {
        let m = &after.members[i];
        let changed = match member_key_index(&before.members, m.conversation, m.player) {
            Some(j) => before.members[j] != *m,
            None => true,
        };
        if changed {
            ws.push(Write::PutMember(m.clone()));
        }
        i += 1;
    }
}

fn diff_agents(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    let mut i = 0;
    let mut j = 0;
    while i < before.agents.len() || j < after.agents.len() {
        if i < before.agents.len() && (j == after.agents.len() || before.agents[i].id < after.agents[j].id) {
            ws.push(Write::DelAgent(before.agents[i].id));
            i += 1;
        } else if j < after.agents.len() && (i == before.agents.len() || after.agents[j].id < before.agents[i].id) {
            ws.push(Write::PutAgent(after.agents[j].clone()));
            j += 1;
        } else {
            if before.agents[i] != after.agents[j] {
                ws.push(Write::PutAgent(after.agents[j].clone()));
            }
            i += 1;
            j += 1;
        }
    }
}

fn memory_index(rows: &Vec<Memory>, id: u64) -> Option<usize> {
    let mut i = 0;
    while i < rows.len() {
        if rows[i].id == id {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn diff_memories(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    let mut i = 0;
    while i < after.memories.len() {
        if memory_index(&before.memories, after.memories[i].id).is_none() {
            ws.push(Write::PutMemory(after.memories[i].clone()));
        }
        i += 1;
    }
}

fn message_index(rows: &Vec<Message>, id: u64) -> Option<usize> {
    let mut i = 0;
    while i < rows.len() {
        if rows[i].id == id {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn diff_messages(before: &Snapshot, after: &Snapshot, ws: &mut Vec<Write>) {
    let mut i = 0;
    while i < after.messages.len() {
        if message_index(&before.messages, after.messages[i].id).is_none() {
            ws.push(Write::PutMessage(after.messages[i].clone()));
        }
        i += 1;
    }
}

pub fn same_motion(a: &Player, b: &Player) -> bool {
    a.owner == b.owner
        && a.x == b.x
        && a.y == b.y
        && a.face_dx == b.face_dx
        && a.face_dy == b.face_dy
        && a.speed == b.speed
        && a.dest_x == b.dest_x
        && a.dest_y == b.dest_y
        && a.pf == b.pf
        && a.pf_started == b.pf_started
        && a.wait_until == b.wait_until
}

/// A non-engine command does not move, retarget, or delete anyone else's player.
///
/// The loops record a flag instead of returning. A `return` inside a loop
/// that also calls `player_index` becomes a break to an outer loop, which
/// Aeneas does not translate.
pub fn motion_isolated(actor: u64, before: &Snapshot, after: &Snapshot) -> bool {
    if actor == ENGINE {
        return true;
    }
    let mut ok = true;
    let mut i = 0;
    while i < before.players.len() {
        let p = &before.players[i];
        if p.owner != actor {
            match player_index(&after.players, p.id) {
                None => ok = false,
                Some(j) => {
                    if !same_motion(p, &after.players[j]) {
                        ok = false;
                    }
                }
            }
        }
        i += 1;
    }
    i = 0;
    while i < after.players.len() {
        let p = &after.players[i];
        if p.owner != actor && player_index(&before.players, p.id).is_none() {
            ok = false;
        }
        i += 1;
    }
    ok
}

pub fn paths_isolated(actor: u64, before: &Snapshot, after: &Snapshot) -> bool {
    if actor == ENGINE {
        return true;
    }
    let mut i = 0;
    while i < before.players.len() {
        let p = &before.players[i];
        if p.owner != actor && path_of(&before.paths, p.id) != path_of(&after.paths, p.id) {
            return false;
        }
        i += 1;
    }
    true
}

pub fn exclusive(s: &Snapshot) -> bool {
    let mut ok = true;
    let mut i = 0;
    while i < s.members.len() {
        let mut j = i + 1;
        while j < s.members.len() {
            if s.members[i].player == s.members[j].player {
                ok = false;
            }
            j += 1;
        }
        i += 1;
    }
    ok
}

fn status_pair_ok(a: u32, b: u32) -> bool {
    (a == ST_WALKING && b == ST_INVITED) || (a == ST_INVITED && b == ST_WALKING) || (a == ST_WALKING && b == ST_WALKING) || (a == ST_TALKING && b == ST_TALKING)
}

pub fn members_consistent(s: &Snapshot) -> bool {
    let mut ok = true;
    let mut i = 0;
    while i < s.conversations.len() {
        let id = s.conversations[i].id;
        let mut n = 0;
        let mut a = 0u32;
        let mut b = 0u32;
        let mut pa = 0u64;
        let mut pb = 0u64;
        let mut j = 0;
        while j < s.members.len() {
            if s.members[j].conversation == id {
                if n == 0 {
                    a = s.members[j].status;
                    pa = s.members[j].player;
                } else if n == 1 {
                    b = s.members[j].status;
                    pb = s.members[j].player;
                }
                n += 1;
            }
            j += 1;
        }
        if n != 2 || !status_pair_ok(a, b) {
            ok = false;
        }
        if player_index(&s.players, pa).is_none() || player_index(&s.players, pb).is_none() {
            ok = false;
        }
        let creator = s.conversations[i].creator;
        if creator != pa && creator != pb {
            ok = false;
        }
        i += 1;
    }
    i = 0;
    while i < s.members.len() {
        if conv_index(&s.conversations, s.members[i].conversation).is_none() {
            ok = false;
        }
        if s.members[i].status != ST_INVITED && s.members[i].status != ST_WALKING && s.members[i].status != ST_TALKING {
            ok = false;
        }
        i += 1;
    }
    ok
}

fn terrain_clear(s: &Snapshot) -> bool {
    let w = s.meta.width;
    let h = s.meta.height;
    let mut i = 0;
    while i < s.players.len() {
        let x = s.players[i].x;
        let y = s.players[i].y;
        if geom::mask_blocked(&s.terrain.mask, w, h, x, y) {
            return false;
        }
        i += 1;
    }
    true
}

fn pairs_apart(s: &Snapshot) -> bool {
    let mut ok = true;
    let mut i = 0;
    while i < s.players.len() {
        let x = s.players[i].x;
        let y = s.players[i].y;
        let mut j = i + 1;
        while j < s.players.len() {
            let qx = s.players[j].x;
            let qy = s.players[j].y;
            if geom::dist_sq(x, y, qx, qy) < COLLISION_SQ {
                ok = false;
            }
            j += 1;
        }
        i += 1;
    }
    ok
}

pub fn collision_safe(s: &Snapshot) -> bool {
    terrain_clear(s) && pairs_apart(s)
}
