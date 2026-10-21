//! Kernel tests: the ported heap, integer geometry, and the four invariants.

use super::*;

fn actor(user: u64, now: u64) -> Principal {
    Principal { org: 1, user, now_ms: now }
}

fn run(s: &Snapshot, user: u64, now: u64, cmd: Command) -> (Snapshot, Result<Reply, Error>) {
    match transition(&actor(user, now), s, &cmd) {
        Ok((ws, reply)) => {
            let next = apply(s, &ws);
            (next, Ok(reply))
        }
        Err(e) => (s.clone(), Err(e)),
    }
}

fn mask(width: u32, height: u32) -> Vec<u8> {
    vec![0u8; ((width * height) as usize).div_ceil(8)]
}

fn world(now: u64) -> Snapshot {
    let (s, r) = run(&Snapshot::default(), ENGINE, now, Command::Init { width: 8, height: 8, mask: mask(8, 8) });
    assert_eq!(r, Ok(Reply::Ok));
    assert!(collision_safe(&s) && exclusive(&s) && members_consistent(&s));
    s
}

fn check(before: &Snapshot, after: &Snapshot, user: u64) {
    assert!(exclusive(after), "exclusivity");
    assert!(members_consistent(after), "membership");
    assert!(collision_safe(after), "collision");
    assert!(motion_isolated(user, before, after), "motion");
    assert!(paths_isolated(user, before, after), "paths");
}

#[test]
fn min_heap_matches_the_typescript_suite() {
    let mut heap = MinHeap::new();
    assert_eq!(heap.len(), 0);
    assert_eq!(heap.peek(), None);
    assert_eq!(heap.pop(), None);

    heap.push(3);
    heap.push(1);
    heap.push(4);
    heap.push(2);
    assert_eq!(heap.peek(), Some(1));
    assert_eq!(heap.len(), 4);
    assert_eq!(heap.pop(), Some(1));
    assert_eq!(heap.peek(), Some(2));
    assert_eq!(heap.pop(), Some(2));
    assert_eq!(heap.peek(), Some(3));
    assert_eq!(heap.pop(), Some(3));
    assert_eq!(heap.pop(), Some(4));
    assert_eq!(heap.pop(), None);
    assert_eq!(heap.len(), 0);
}

#[test]
fn integer_distance_and_path_pose() {
    assert_eq!(dist_sq(0, 0, 3000, 4000), 25_000_000);
    assert_eq!(isqrt(25_000_000), 5000);
    assert_eq!(manhattan(1000, 0, 1000, 2000), 2000);
    assert_eq!(isqrt(0), 0);

    assert_eq!(geom::tile_of(3500), 3);
    let path = vec![
        PathNode { player: 1, seq: 0, x: 0, y: 0, face_dx: 2000, face_dy: 1000, t: 1000 },
        PathNode { player: 1, seq: 1, x: 1000, y: 0, face_dx: 2000, face_dy: 1000, t: 2000 },
    ];
    let pose = geom::path_pose(&path, 1500).unwrap();
    assert_eq!(pose.x, 500);
    assert_eq!(pose.y, 0);
    // 1000 milli over 1000 ms is 1000 milli-tiles per second.
    assert_eq!(pose.speed, 1000);
    // Before the first timestamp the pose stays put with speed 0, matching geometry.ts.
    let before = geom::path_pose(&path, 500).unwrap();
    assert_eq!(before.x, 0);
    assert_eq!(before.speed, 0);
}

#[test]
fn compress_keeps_turns() {
    let facing_n = geom::face_north();
    let facing_e = geom::face_east();
    let mut dense = Vec::new();
    let mut y = 0;
    while y <= 4 {
        dense.push(PathNode {
            player: 1,
            seq: y as u64,
            x: 0,
            y: y * MILLI,
            face_dx: geom::face_zero(),
            face_dy: if y == 4 { facing_e } else { facing_n },
            t: y as u64,
        });
        y += 1;
    }
    // The last vertical point faces east, so the line compresses up to it.
    let packed = geom::compress(&dense);
    assert_eq!(packed.len(), 3);
    assert_eq!(packed[0].y, 0);
    assert_eq!(packed[1].y, 3 * MILLI);
    assert_eq!(packed[2].y, 4 * MILLI);
}

#[test]
fn compress_straight_line_and_turn() {
    let north = geom::face_north();
    let east = geom::face_east();
    let zero = geom::face_zero();
    let mut line = Vec::new();
    let mut y = 0;
    while y <= 4 {
        line.push(PathNode {
            player: 1,
            seq: y as u64,
            x: 0,
            y: y * MILLI,
            face_dx: zero,
            face_dy: north,
            t: y as u64,
        });
        y += 1;
    }
    let packed = geom::compress(&line);
    assert_eq!(packed.len(), 2);
    assert_eq!((packed[0].y, packed[1].y), (0, 4 * MILLI));

    // The corner keeps the incoming facing, so it is colinear with the leg
    // behind it and stays. The first east-facing step is a new anchor.
    let mut turned = line;
    turned[2].face_dx = zero;
    turned[2].face_dy = north;
    turned[3].x = MILLI;
    turned[3].y = 2 * MILLI;
    turned[3].face_dx = east;
    turned[3].face_dy = zero;
    turned[4].x = 2 * MILLI;
    turned[4].y = 2 * MILLI;
    turned[4].face_dx = east;
    turned[4].face_dy = zero;
    let packed = geom::compress(&turned);
    assert_eq!(packed.len(), 4);
    assert_eq!((packed[1].x, packed[1].y), (0, 2 * MILLI));
    assert_eq!((packed[2].x, packed[2].y), (MILLI, 2 * MILLI));
    assert_eq!((packed[3].x, packed[3].y), (2 * MILLI, 2 * MILLI));
}

#[test]
fn join_move_and_isolation() {
    let s = world(1_000);
    let (s, r) = run(&s, 1, 1_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    assert!(matches!(r, Ok(Reply::Player(_))));
    let (s, r) = run(&s, 2, 1_000, Command::Join { name: b"Bea".to_vec(), character: 1 });
    assert!(matches!(r, Ok(Reply::Player(_))));
    let before = s.clone();
    let (s, r) = run(&s, 1, 1_100, Command::MoveTo { x: 7, y: 0 });
    assert_eq!(r, Ok(Reply::Ok));
    check(&before, &s, 1);
    assert_ne!(s.players[0].pf, PF_NONE);
    // The other player did not start walking.
    assert_eq!(s.players[1].pf, PF_NONE);
    assert!(run(&s, 1, 1_100, Command::MoveTo { x: 80, y: 0 }).1.is_err());
}

#[test]
fn conversation_exclusivity_and_membership() {
    let s = world(2_000);
    let (s, _) = run(&s, 1, 2_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    let (s, _) = run(&s, 2, 2_000, Command::Join { name: b"Bea".to_vec(), character: 1 });
    let (s, _) = run(&s, 3, 2_000, Command::Join { name: b"Cam".to_vec(), character: 2 });
    let a = s.players.iter().find(|p| p.owner == 1).unwrap().id;
    let b = s.players.iter().find(|p| p.owner == 2).unwrap().id;
    let before = s.clone();
    let (s, r) = run(&s, 1, 2_100, Command::StartConversation { invitee: b });
    assert!(matches!(r, Ok(Reply::Conversation(_))));
    check(&before, &s, 1);
    assert_eq!(s.members.len(), 2);
    assert!(exclusive(&s));
    // A second conversation for the same player is refused and changes nothing.
    let (s2, r) = run(&s, 1, 2_200, Command::StartConversation { invitee: s.players.iter().find(|p| p.owner == 3).unwrap().id });
    assert_eq!(r, Err(Error::Busy));
    assert_eq!(s2.members.len(), 2);
    let before = s.clone();
    let (s, r) = run(&s, 2, 2_200, Command::AcceptInvite);
    assert_eq!(r, Ok(Reply::Ok));
    check(&before, &s, 2);
    assert!(s.members.iter().all(|m| m.status == ST_WALKING));
    let (s, r) = run(&s, 1, 2_300, Command::LeaveConversation);
    assert_eq!(r, Ok(Reply::Ok));
    assert!(s.conversations.is_empty() && s.members.is_empty());
    let _ = a;
}

#[test]
fn collision_holds_across_a_step() {
    let mut mask = mask(8, 8);
    // Wall down the middle, leaving a gap at y = 0.
    let mut y = 1;
    while y < 8 {
        set_blocked(&mut mask, 8, 8, 3, y);
        y += 1;
    }
    let (s, _) = run(&Snapshot::default(), ENGINE, 5_000, Command::Init { width: 8, height: 8, mask });
    let (s, _) = run(&s, 1, 5_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    // Put Ada on the left if spawn did not.
    let (s, _) = run(&s, 1, 5_000, Command::MoveTo { x: 0, y: 0 });
    let mut now = 5_000;
    let mut s = s;
    let mut steps = 0;
    while steps < 40 {
        now += 1000;
        let before = s.clone();
        let (next, r) = run(&s, ENGINE, now, Command::Step);
        assert!(r.is_ok());
        assert!(collision_safe(&next), "step {steps}");
        assert!(exclusive(&next) && members_consistent(&next));
        // Engine steps may move the caller. A human command in between must not.
        let (next, _) = run(&next, 1, now, Command::Stop);
        assert!(motion_isolated(1, &before, &next) || next.players.iter().any(|p| p.owner == 1));
        s = next;
        steps += 1;
        if s.players.iter().any(|p| p.owner == 1 && p.pf == PF_NONE && p.x == 0) && steps > 5 {
            break;
        }
    }
    assert!(collision_safe(&s));
}

#[test]
fn a_player_cannot_step_the_world_or_move_someone_else() {
    let s = world(9_000);
    let (s, _) = run(&s, 1, 9_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    assert_eq!(run(&s, 1, 10_000, Command::Step).1, Err(Error::Forbidden));
    assert_eq!(run(&s, 1, 10_000, Command::Freeze).1, Err(Error::Forbidden));
    assert_eq!(run(&s, 1, 10_000, Command::CreateAgent { name: b"Zed".to_vec(), character: 1, identity: b"z".to_vec(), plan: b"p".to_vec() }).1, Err(Error::Forbidden));
}

#[test]
fn blocked_tile_is_not_entered() {
    let mut mask = mask(4, 4);
    set_blocked(&mut mask, 4, 4, 1, 0);
    let (s, _) = run(&Snapshot::default(), ENGINE, 1_000, Command::Init { width: 4, height: 4, mask });
    let (mut s, _) = run(&s, 1, 1_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    // Force the player onto (0,0) through a move, then step toward the wall.
    let (s2, _) = run(&s, 1, 1_000, Command::MoveTo { x: 0, y: 0 });
    s = s2;
    let mut now = 1_000;
    let mut i = 0;
    while i < 30 {
        now += 1000;
        let (next, _) = run(&s, ENGINE, now, Command::Step);
        s = next;
        i += 1;
    }
    let (mut s, _) = run(&s, 1, now, Command::MoveTo { x: 3, y: 0 });
    i = 0;
    while i < 40 {
        now += 1000;
        let (next, _) = run(&s, ENGINE, now, Command::Step);
        assert!(collision_safe(&next));
        for p in &next.players {
            assert!(!mask_blocked(&next.terrain.mask, 4, 4, p.x, p.y));
        }
        s = next;
        i += 1;
    }
}

#[test]
fn batch_seek_matches_scalar_seek() {
    let mask = mask(16, 16);
    let xs = vec![0, 1000, 2000, 5000];
    let ys = vec![0, 0, 1000, 4000];
    let txs = vec![5, 5, 0, 1];
    let tys = vec![0, 2, 4, 1];
    let scalar = scalar_seek(&xs, &ys, &txs, &tys, 400, &mask, 16, 16);
    let batch = batch_seek(&xs, &ys, &txs, &tys, 400, &mask, 16, 16);
    assert_eq!(scalar, batch);
    // A step toward the target moves, and never onto a blocked tile.
    let mut mask = mask;
    set_blocked(&mut mask, 16, 16, 1, 0);
    let (xs, ys) = scalar_seek(&[0], &[0], &[3], &[0], 5000, &mask, 16, 16);
    assert!(!mask_blocked(&mask, 16, 16, xs[0], ys[0]));
}

#[test]
fn engine_relocate_writes_positions_and_drops_paths() {
    let s = world(5_000);
    let (s, r) = run(&s, ENGINE, 5_000, Command::CreateAgent { name: b"A".to_vec(), character: 0, identity: b"a".to_vec(), plan: b"p".to_vec() });
    assert!(r.is_ok());
    let owner = s.players[0].owner;
    let (s, _) = run(&s, owner, 5_100, Command::MoveTo { x: 4, y: 4 });
    assert!(!s.paths.is_empty());
    let (_denied, denied) = run(&s, 1, 5_200, Command::Relocate { x: vec![100], y: vec![100] });
    assert_eq!(denied, Err(Error::Forbidden));
    let (_bad, bad) = run(&s, ENGINE, 5_200, Command::Relocate { x: Vec::new(), y: Vec::new() });
    assert_eq!(bad, Err(Error::BadState));
    let dest_x = s.players[0].dest_x;
    let pf = s.players[0].pf;
    let (s2, r) = run(&s, ENGINE, 5_200, Command::Relocate { x: vec![2_000], y: vec![3_000] });
    assert_eq!(r, Ok(Reply::Ok));
    assert_eq!(s2.players[0].x, 2_000);
    assert_eq!(s2.players[0].y, 3_000);
    assert_eq!(s2.players[0].dest_x, dest_x);
    assert_eq!(s2.players[0].pf, pf);
    assert_eq!(s2.players[0].speed, 0);
    assert!(s2.paths.is_empty());
    check(&s, &s2, ENGINE);
}

#[test]
fn diff_roundtrip_matches_apply() {
    let s = world(3_000);
    let (s, _) = run(&s, 1, 3_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    let cmd = Command::MoveTo { x: 5, y: 5 };
    let (ws, _) = transition(&actor(1, 3_100), &s, &cmd).unwrap();
    let next = apply(&s, &ws);
    let again = rules::diff(&s, &next);
    assert_eq!(apply(&s, &again), next);
}

#[test]
fn player_index_finds_sorted_ids_and_an_unsorted_row() {
    let (s, _) = run(&Snapshot::default(), ENGINE, 1_000, Command::Init { width: 8, height: 8, mask: mask(8, 8) });
    let (mut s, _) = run(&s, ENGINE, 1_000, Command::CreateAgent { name: b"A".to_vec(), character: 0, identity: b"a".to_vec(), plan: b"p".to_vec() });
    let (s2, _) = run(&s, ENGINE, 1_000, Command::CreateAgent { name: b"B".to_vec(), character: 1, identity: b"b".to_vec(), plan: b"p".to_vec() });
    s = s2;
    let (s3, _) = run(&s, ENGINE, 1_000, Command::CreateAgent { name: b"C".to_vec(), character: 2, identity: b"c".to_vec(), plan: b"p".to_vec() });
    s = s3;
    assert!(s.players.len() >= 3);
    let mut i = 0;
    while i < s.players.len() {
        let id = s.players[i].id;
        assert_eq!(rules::player_index(&s.players, id), Some(i));
        i += 1;
    }
    assert_eq!(rules::player_index(&s.players, u64::MAX), None);
    assert_eq!(rules::player_at(&s.players, s.players[1].id), Some(1));
    let last = s.players.len() - 1;
    let moved = s.players[last].id;
    s.players.swap(0, last);
    let found = rules::player_at(&s.players, moved).expect("unsorted id");
    assert_eq!(s.players[found].id, moved);
    let first = rules::player_index(&s.players, moved).expect("scan");
    assert_eq!(s.players[first].id, moved);
}

#[test]
fn step_plus_collision_radius_fits_in_one_tile() {
    assert!(u64::from(SPEED) * TICK_MS / 1000 + 750 < u64::from(MILLI));
}

#[test]
fn near_grid_matches_full_scan_including_a_short_move() {
    let w = 16u32;
    let h = 16u32;
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut i = 0u32;
    while i < 80 {
        xs.push((i % 16) * 1000 + (i % 5) * 20);
        ys.push((i / 16) * 1000 + (i % 3) * 30);
        i += 1;
    }
    let g = geom::near_index(w, h, &xs, &ys);
    let mut q = 0u32;
    while q < 40 {
        let x = (q * 370) % (w * 1000);
        let y = (q * 910) % (h * 1000);
        let mut slow = false;
        let mut k = 0;
        while k < xs.len() {
            if k != 3 && dist_sq(xs[k], ys[k], x, y) < COLLISION_SQ {
                slow = true;
            }
            k += 1;
        }
        assert_eq!(geom::near_hit(&g, &xs, &ys, x, y, 3), slow);
        q += 1;
    }
    xs[5] = 990;
    ys[5] = 500;
    let g = geom::near_index(w, h, &xs, &ys);
    xs[5] = 1010;
    let mut q = 0u32;
    while q < 40 {
        let x = (q * 410) % (w * 1000);
        let y = (q * 640) % (h * 1000);
        let mut slow = false;
        let mut k = 0;
        while k < xs.len() {
            if dist_sq(xs[k], ys[k], x, y) < COLLISION_SQ {
                slow = true;
            }
            k += 1;
        }
        assert_eq!(geom::near_hit(&g, &xs, &ys, x, y, u32::MAX), slow);
        q += 1;
    }
}

#[test]
fn agent_step_refuses_a_blocked_tile_and_a_neighbor() {
    let w = 8u32;
    let h = 8u32;
    let mut mask = mask(w, h);
    set_blocked(&mut mask, w, h, 1, 0);
    let xs = vec![0u32, 1000];
    let ys = vec![0u32, 0];
    let g = near_index(w, h, &xs, &ys);
    let blocked = agent_step(0, 0, 3, 4, true, 1000, 0, 9, 8, 7, &mask, w, h, &g.head, &g.next, &xs, &ys, 0);
    assert!(!blocked.moved);
    assert_eq!((blocked.x, blocked.y, blocked.speed), (0, 0, 0));
    assert!(mask_blocked(&mask, w, h, 1000, 0));
    let crowded = agent_step(0, 0, 3, 4, true, 500, 0, 9, 8, 7, &mask, w, h, &g.head, &g.next, &xs, &ys, 0);
    assert!(!crowded.moved);
    assert_eq!((crowded.x, crowded.y), (0, 0));
    assert!(dist_sq(500, 0, xs[1], ys[1]) < COLLISION_SQ);
    let clear = agent_step(0, 0, 3, 4, true, 0, 0, 9, 8, 7, &mask, w, h, &g.head, &g.next, &xs, &ys, 0);
    assert!(clear.moved);
    assert_eq!((clear.x, clear.y, clear.face_dx, clear.face_dy, clear.speed), (0, 0, 9, 8, 7));
    let still = agent_step(0, 0, 3, 4, false, 0, 0, 9, 8, 7, &mask, w, h, &g.head, &g.next, &xs, &ys, 0);
    assert!(!still.moved);
    assert_eq!((still.x, still.y, still.face_dx, still.speed), (0, 0, 3, 0));
}

#[test]
fn tick_moves_lets_the_next_agent_see_the_landing() {
    let w = 8u32;
    let h = 8u32;
    let mask = mask(w, h);
    let mut xs = vec![0u32, 1000];
    let mut ys = vec![0u32, 0];
    let g = near_index(w, h, &xs, &ys);
    let moving = [1u32, 1];
    let cand_x = [200u32, 800];
    let cand_y = [0u32, 0];
    assert!(tick_moves(&mut xs, &mut ys, &moving, &cand_x, &cand_y, &mask, w, h, &g.head, &g.next));
    // 800 - 200 = 600, inside the radius, so the second agent stays.
    assert_eq!(xs, vec![200, 1000]);
    assert_eq!(ys, vec![0, 0]);
    let mut xs = vec![0u32, 1000];
    let mut ys = vec![0u32];
    assert!(!tick_moves(&mut xs, &mut ys, &moving, &cand_x, &cand_y, &mask, w, h, &g.head, &g.next));
    assert_eq!(xs, vec![0, 1000]);
}

#[test]
fn tick_players_matches_one_engine_tick() {
    let (s, _) = run(&Snapshot::default(), ENGINE, 1_000_000, Command::Init { width: 8, height: 8, mask: mask(8, 8) });
    let (s, _) = run(&s, 1, 1_000_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    let (mut s, _) = run(&s, 2, 1_000_000, Command::Join { name: b"Bea".to_vec(), character: 1 });
    assert_eq!(s.players.len(), 2);
    let id0 = s.players[0].id;
    let id1 = s.players[1].id;
    let owner0 = s.players[0].owner;
    let owner1 = s.players[1].owner;
    s.players[0].x = 0;
    s.players[0].y = 0;
    s.players[0].pf = PF_MOVE;
    s.players[0].dest_x = 5_000;
    s.players[0].dest_y = 0;
    s.players[0].pf_started = 1_000_000;
    s.players[0].last_input = 1_000_000;
    s.players[1].x = 1_000;
    s.players[1].y = 0;
    s.players[1].pf = PF_MOVE;
    s.players[1].dest_x = 0;
    s.players[1].dest_y = 0;
    s.players[1].pf_started = 1_000_000;
    s.players[1].last_input = 1_000_000;
    s.meta.now_ms = 1_000_000;
    let t0 = 1_000_000u64;
    let t1 = t0 + 80;
    s.paths = vec![
        PathNode { player: id0, seq: 0, x: 0, y: 0, face_dx: 1000, face_dy: 0, t: t0 },
        PathNode { player: id0, seq: 1, x: 1000, y: 0, face_dx: 1000, face_dy: 0, t: t1 },
        PathNode { player: id1, seq: 0, x: 1000, y: 0, face_dx: 1000, face_dy: 0, t: t0 },
        PathNode { player: id1, seq: 1, x: 0, y: 0, face_dx: 1000, face_dy: 0, t: t1 },
    ];
    s.members.push(Member { conversation: 7, player: id0, invited: 1, status: ST_INVITED, started: 0 });
    let before = s.clone();
    assert_eq!(run_ticks(&mut s, t0 + TICK_MS), 1);
    assert_eq!((s.players[0].x, s.players[0].y), (200, 0));
    assert_eq!((s.players[1].x, s.players[1].y), (1000, 0));
    assert!(collision_safe(&s));
    let mut moving = Vec::new();
    let mut cand_x = Vec::new();
    let mut cand_y = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut i = 0;
    while i < before.players.len() {
        let p = &before.players[i];
        xs.push(p.x);
        ys.push(p.y);
        let path = before.paths.iter().filter(|n| n.player == p.id).cloned().collect::<Vec<_>>();
        let pose = path_pose(&path, before.meta.now_ms + TICK_MS).unwrap();
        moving.push(if p.pf == PF_MOVE { 1 } else { 0 });
        cand_x.push(pose.x);
        cand_y.push(pose.y);
        i += 1;
    }
    let g = near_index(before.meta.width, before.meta.height, &xs, &ys);
    let stepped = tick_players(before.clone(), &moving, &cand_x, &cand_y, &g.head, &g.next);
    assert_eq!(stepped.players[0].x, s.players[0].x);
    assert_eq!(stepped.players[1].x, s.players[1].x);
    assert_eq!(stepped.players[0].y, s.players[0].y);
    assert_eq!(stepped.players[1].y, s.players[1].y);
    assert_eq!(stepped.players[0].id, id0);
    assert_eq!(stepped.players[1].id, id1);
    assert_eq!(stepped.players[0].owner, owner0);
    assert_eq!(stepped.players[1].owner, owner1);
    assert_eq!(stepped.members, before.members);
    assert_eq!(stepped.conversations, before.conversations);
}

fn round_move(xs: &mut [u32], ys: &mut [u32], moving: &[u32], cand_x: &[u32], cand_y: &[u32], ids: &[u64], mask: &[u8], w: u32, h: u32) -> Vec<u32> {
    let n = xs.len();
    let cells = (w as usize) * (h as usize);
    let mut head = vec![u32::MAX; cells];
    let mut next = vec![u32::MAX; n];
    let mut prop_x = vec![0u32; n];
    let mut prop_y = vec![0u32; n];
    let mut prop_on = vec![0u32; n];
    assert!(tick_round(xs, ys, moving, cand_x, cand_y, ids, mask, w, h, &mut head, &mut next, &mut prop_x, &mut prop_y, &mut prop_on));
    prop_on
}

fn pairs_clear(xs: &[u32], ys: &[u32]) -> bool {
    let mut i = 0;
    while i < xs.len() {
        let mut j = i + 1;
        while j < xs.len() && j < ys.len() {
            if dist_sq(xs[i], ys[i], xs[j], ys[j]) < COLLISION_SQ {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

#[test]
fn tick_round_uses_priority_and_previous_positions() {
    let w = 8u32;
    let h = 8u32;
    let mask = mask(w, h);
    let mut xs = vec![0u32, 1000];
    let mut ys = vec![0u32, 0];
    let moved = round_move(&mut xs, &mut ys, &[1, 1], &[200, 800], &[0, 0], &[1, 2], &mask, w, h);
    assert_eq!(xs, vec![200, 1000]);
    assert_eq!(moved, vec![1, 0]);
    assert!(pairs_clear(&xs, &ys));

    let mut xs = vec![0u32, 1000];
    let mut ys = vec![0u32, 0];
    let moved = round_move(&mut xs, &mut ys, &[1, 1], &[200, 800], &[0, 0], &[9, 1], &mask, w, h);
    assert_eq!(xs, vec![0, 800]);
    assert_eq!(moved, vec![0, 1]);
    assert!(pairs_clear(&xs, &ys));

    let mut xs = vec![2000u32, 1248];
    let mut ys = vec![0u32, 0];
    let mut seq_x = xs.clone();
    let mut seq_y = ys.clone();
    let g = near_index(w, h, &xs, &ys);
    assert!(tick_moves(&mut seq_x, &mut seq_y, &[1, 1], &[2012, 1260], &[0, 0], &mask, w, h, &g.head, &g.next));
    assert_eq!(seq_x, vec![2012, 1260]);
    let moved = round_move(&mut xs, &mut ys, &[1, 1], &[2012, 1260], &[0, 0], &[1, 2], &mask, w, h);
    assert_eq!(xs, vec![2012, 1248]);
    assert_eq!(moved, vec![1, 0]);
    assert!(pairs_clear(&xs, &ys));
}

#[test]
fn tick_round_refuses_a_blocked_tile_and_stays_apart() {
    let w = 8u32;
    let h = 8u32;
    let mut blocked = mask(w, h);
    set_blocked(&mut blocked, w, h, 1, 0);
    let mut xs = vec![0u32];
    let mut ys = vec![0u32];
    let moved = round_move(&mut xs, &mut ys, &[1], &[1000], &[0], &[1], &blocked, w, h);
    assert_eq!(xs, vec![0]);
    assert_eq!(moved, vec![0]);

    let open = mask(16, 8);
    let n = 128usize;
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut moving = Vec::new();
    let mut cand_x = Vec::new();
    let mut cand_y = Vec::new();
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i < n {
        let x = ((i % 16) as u32) * 1000;
        let y = ((i / 16) as u32) * 1000;
        xs.push(x);
        ys.push(y);
        moving.push(1);
        cand_x.push(x + 12);
        cand_y.push(y);
        ids.push((i as u64) + 1);
        i += 1;
    }
    assert!(pairs_clear(&xs, &ys));
    let moved = round_move(&mut xs, &mut ys, &moving, &cand_x, &cand_y, &ids, &open, 16, 8);
    assert!(moved.iter().all(|m| *m == 1));
    assert_eq!(xs[0], 12);
    assert!(pairs_clear(&xs, &ys));
    i = 0;
    while i < n {
        assert!(!mask_blocked(&open, 16, 8, xs[i], ys[i]));
        i += 1;
    }
}

#[test]
fn grid_yield_matches_the_full_scan() {
    let w = 8u32;
    let h = 8u32;
    let xs = vec![0u32, 1000, 2000, 500, 3000];
    let ys = vec![0u32, 0, 0, 400, 1000];
    let ids = vec![4u64, 1, 8, 2, 3];
    let on = vec![1u32, 1, 0, 1, 1];
    let mut head = vec![u32::MAX; (w * h) as usize];
    let mut next = vec![u32::MAX; xs.len()];
    assert!(grid_build(&mut head, &mut next, w, h, &xs, &ys, &on));
    let ones = vec![1u32; xs.len()];
    let mut i = 0usize;
    while i < xs.len() {
        assert!(grid_build(&mut head, &mut next, w, h, &xs, &ys, &on));
        let grid = grid_yield(w, h, &head, &next, &xs, &ys, &ids, xs[i], ys[i], ids[i], i as u32);
        let scan = scan_yield(&xs, &ys, &ids, &on, xs[i], ys[i], ids[i], i as u32);
        assert_eq!(grid, scan, "agent {i}");
        assert!(grid_build(&mut head, &mut next, w, h, &xs, &ys, &ones));
        let hit = grid_hit(w, h, &head, &next, &xs, &ys, xs[i], ys[i], i as u32);
        assert_eq!(hit, scan_hit(&xs, &ys, xs[i], ys[i], i as u32), "hit {i}");
        i += 1;
    }
}

#[test]
fn sequential_tick_still_sees_the_earlier_landing() {
    let (s, _) = run(&Snapshot::default(), ENGINE, 1_000_000, Command::Init { width: 8, height: 8, mask: mask(8, 8) });
    let (mut s, _) = run(&s, ENGINE, 1_000_000, Command::CreateAgent { name: b"A".to_vec(), character: 0, identity: b"a".to_vec(), plan: b"p".to_vec() });
    let (s2, _) = run(&s, ENGINE, 1_000_000, Command::CreateAgent { name: b"B".to_vec(), character: 1, identity: b"b".to_vec(), plan: b"p".to_vec() });
    s = s2;
    assert_eq!(s.players.len(), 2);
    let id0 = s.players[0].id;
    let id1 = s.players[1].id;
    s.players[0].x = 2000;
    s.players[0].y = 0;
    s.players[0].dest_x = 4000;
    s.players[0].dest_y = 0;
    s.players[0].pf = PF_MOVE;
    s.players[0].pf_started = 1_000_000;
    s.players[1].x = 1248;
    s.players[1].y = 0;
    s.players[1].dest_x = 4000;
    s.players[1].dest_y = 0;
    s.players[1].pf = PF_MOVE;
    s.players[1].pf_started = 1_000_000;
    s.meta.now_ms = 1_000_000;
    let t0 = 1_000_000u64;
    let t1 = t0 + TICK_MS;
    s.paths = vec![
        PathNode { player: id0, seq: 0, x: 2000, y: 0, face_dx: 2000, face_dy: 1000, t: t0 },
        PathNode { player: id0, seq: 1, x: 2012, y: 0, face_dx: 2000, face_dy: 1000, t: t1 },
        PathNode { player: id1, seq: 0, x: 1248, y: 0, face_dx: 2000, face_dy: 1000, t: t0 },
        PathNode { player: id1, seq: 1, x: 1260, y: 0, face_dx: 2000, face_dy: 1000, t: t1 },
    ];
    let mut parallel = s.clone();
    let mut sequential = s.clone();
    assert_eq!(run_ticks(&mut parallel, t0 + TICK_MS), 1);
    assert_eq!(run_ticks_seq(&mut sequential, t0 + TICK_MS), 1);
    assert_eq!((parallel.players[0].x, parallel.players[1].x), (2012, 1248));
    assert_eq!((sequential.players[0].x, sequential.players[1].x), (2012, 1260));
    assert!(collision_safe(&parallel));
    assert!(collision_safe(&sequential));
    assert_eq!(run(&s, 1, t0 + TICK_MS, Command::StepSeq).1, Err(Error::Forbidden));
}

/// xorshift64*, enough to scatter test points without a dependency.
fn rand_next(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// Build the grid inserting agents in `order`, the way the PTX atomic pushes
/// can interleave. Same cells as `grid_build`, different chain order.
fn grid_build_in(order: &[usize], head: &mut [u32], next: &mut [u32], w: u32, h: u32, xs: &[u32], ys: &[u32], on: &[u32]) {
    head.fill(u32::MAX);
    next.fill(u32::MAX);
    for &i in order {
        let cell = cell_of(xs[i], ys[i], w, h);
        if on[i] != 0 && cell < head.len() {
            next[i] = head[cell];
            head[cell] = i as u32;
        }
    }
}

#[test]
fn grid_answers_do_not_depend_on_chain_order() {
    let (w, h) = (12u32, 9u32);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut trial = 0;
    while trial < 200 {
        let n = 1 + (rand_next(&mut seed) % 60) as usize;
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let mut ids = Vec::new();
        let mut on = Vec::new();
        for _ in 0..n {
            // A few points fall off the map, and ids repeat, to reach every branch.
            xs.push((rand_next(&mut seed) % (w as u64 * 1000 + 800)) as u32);
            ys.push((rand_next(&mut seed) % (h as u64 * 1000 + 800)) as u32);
            ids.push(rand_next(&mut seed) % 20);
            on.push((rand_next(&mut seed) % 4 != 0) as u32);
        }
        let mut order: Vec<usize> = (0..n).collect();
        let mut k = n;
        while k > 1 {
            let j = (rand_next(&mut seed) % k as u64) as usize;
            order.swap(k - 1, j);
            k -= 1;
        }
        let mut head = vec![0u32; (w * h) as usize];
        let mut next = vec![0u32; n];
        let mut seen = vec![0u32; n];
        let ones = vec![1u32; n];
        grid_build_in(&order, &mut head, &mut next, w, h, &xs, &ys, &ones);
        assert!(grid_wellformed(&head, &next, w, h, &xs, &ys, &ones, &mut seen), "trial {trial} position grid");
        let mut q = 0;
        while q < 20 {
            let x = (rand_next(&mut seed) % (w as u64 * 1000)) as u32;
            let y = (rand_next(&mut seed) % (h as u64 * 1000)) as u32;
            let skip = (rand_next(&mut seed) % (n as u64 + 1)) as u32;
            // Off-map agents are not in the grid, so compare against in-map ones.
            let inside: Vec<bool> = (0..n).map(|i| cell_of(xs[i], ys[i], w, h) != usize::MAX).collect();
            let scan = (0..n).any(|i| inside[i] && i as u32 != skip && dist_sq(xs[i], ys[i], x, y) < COLLISION_SQ);
            assert_eq!(grid_hit(w, h, &head, &next, &xs, &ys, x, y, skip), scan, "trial {trial} hit");
            q += 1;
        }
        grid_build_in(&order, &mut head, &mut next, w, h, &xs, &ys, &on);
        assert!(grid_wellformed(&head, &next, w, h, &xs, &ys, &on, &mut seen), "trial {trial} proposal grid");
        let mut i = 0;
        while i < n {
            let on_map: Vec<u32> = (0..n).map(|j| (on[j] != 0 && cell_of(xs[j], ys[j], w, h) != usize::MAX) as u32).collect();
            if cell_of(xs[i], ys[i], w, h) != usize::MAX {
                let grid = grid_yield(w, h, &head, &next, &xs, &ys, &ids, xs[i], ys[i], ids[i], i as u32);
                assert_eq!(grid, scan_yield(&xs, &ys, &ids, &on_map, xs[i], ys[i], ids[i], i as u32), "trial {trial} yield {i}");
            }
            i += 1;
        }
        trial += 1;
    }
}

#[test]
fn grid_wellformed_rejects_broken_chains() {
    let (w, h) = (4u32, 4u32);
    let xs = vec![500u32, 600, 2500, 9000];
    let ys = vec![500u32, 700, 1500, 500];
    let on = vec![1u32, 1, 1, 1];
    let mut head = vec![0u32; 16];
    let mut next = vec![0u32; 4];
    let mut seen = vec![0u32; 4];
    assert!(grid_build(&mut head, &mut next, w, h, &xs, &ys, &on));
    assert!(grid_wellformed(&head, &next, w, h, &xs, &ys, &on, &mut seen));
    let c0 = cell_of(500, 500, w, h);
    let c2 = cell_of(2500, 1500, w, h);
    // Agent 3 is off the map: linking it is wrong.
    let (mut h2, mut n2) = (head.clone(), next.clone());
    n2[3] = h2[c2];
    h2[c2] = 3;
    assert!(!grid_wellformed(&h2, &n2, w, h, &xs, &ys, &on, &mut seen), "off-map agent linked");
    // Dropping agent 2 loses a member.
    let mut h2 = head.clone();
    h2[c2] = u32::MAX;
    assert!(!grid_wellformed(&h2, &next, w, h, &xs, &ys, &on, &mut seen), "missing agent");
    // Agent 2 filed under agent 0's cell.
    let (mut h2, mut n2) = (head.clone(), next.clone());
    h2[c2] = u32::MAX;
    n2[2] = h2[c0];
    h2[c0] = 2;
    assert!(!grid_wellformed(&h2, &n2, w, h, &xs, &ys, &on, &mut seen), "wrong cell");
    // A cycle in cell 0.
    let mut n2 = next.clone();
    let first = head[c0] as usize;
    let second = next[first] as usize;
    n2[second] = first as u32;
    assert!(!grid_wellformed(&head, &n2, w, h, &xs, &ys, &on, &mut seen), "cycle");
    // An agent switched off but still linked.
    let off = vec![1u32, 1, 0, 1];
    assert!(!grid_wellformed(&head, &next, w, h, &xs, &ys, &off, &mut seen), "off agent linked");
}

#[test]
fn relocate_refuses_overlap_and_blocked_tiles() {
    let s = world(5_000);
    let (s, _) = run(&s, ENGINE, 5_000, Command::CreateAgent { name: b"A".to_vec(), character: 0, identity: b"a".to_vec(), plan: b"p".to_vec() });
    let (s, _) = run(&s, ENGINE, 5_000, Command::CreateAgent { name: b"B".to_vec(), character: 0, identity: b"b".to_vec(), plan: b"p".to_vec() });
    assert_eq!(s.players.len(), 2);
    assert!(placement_ok(&s));
    // 749 apart is inside the radius; 750 is not.
    let (_s2, r) = run(&s, ENGINE, 5_100, Command::Relocate { x: vec![2_000, 2_749], y: vec![3_000, 3_000] });
    assert_eq!(r, Err(Error::Occupied));
    let (s2, r) = run(&s, ENGINE, 5_100, Command::Relocate { x: vec![2_000, 2_750], y: vec![3_000, 3_000] });
    assert_eq!(r, Ok(Reply::Ok));
    assert!(placement_ok(&s2));
    // Off the map counts as blocked.
    let far = s.meta.width * MILLI + 10;
    let (_s3, r) = run(&s, ENGINE, 5_100, Command::Relocate { x: vec![2_000, far], y: vec![3_000, 3_000] });
    assert_eq!(r, Err(Error::Occupied));
    // A blocked tile.
    let mut walled = s.clone();
    set_blocked(&mut walled.terrain.mask, walled.meta.width, walled.meta.height, 5, 5);
    let (_s4, r) = run(&walled, ENGINE, 5_100, Command::Relocate { x: vec![2_000, 5_500], y: vec![3_000, 5_500] });
    assert_eq!(r, Err(Error::Occupied));
}

#[test]
fn bucketed_placement_matches_the_all_pairs_scan() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut agree = [0usize; 2];
    for trial in 0..400 {
        let (w, h) = (2 + (rand_next(&mut seed) % 10) as u32, 2 + (rand_next(&mut seed) % 10) as u32);
        let mut s = world(5_000);
        s.meta.width = w;
        s.meta.height = h;
        s.terrain.mask = mask(w, h);
        if rand_next(&mut seed) % 3 == 0 {
            set_blocked(&mut s.terrain.mask, w, h, (rand_next(&mut seed) % w as u64) as u32, (rand_next(&mut seed) % h as u64) as u32);
        }
        let n = (rand_next(&mut seed) % 30) as usize;
        // Dense enough that some trials put 5+ players in a tile, and a few
        // points step off the map.
        for i in 0..n {
            let id = if rand_next(&mut seed) % 10 == 0 { 1 } else { i as u64 + 1 };
            let x = (rand_next(&mut seed) % (w as u64 * 1000 + 300)) as u32;
            let y = (rand_next(&mut seed) % (h as u64 * 1000 + 300)) as u32;
            s.players.push(Player {
                id, owner: id, is_human: false, x, y, face_dx: 0, face_dy: 0, speed: 0, dest_x: 0, dest_y: 0, pf: 0,
                pf_started: 0, wait_until: 0, last_input: 0, activity: 0, activity_until: 0, character: 0, name: Vec::new(),
            });
        }
        let fast = placement_ok(&s);
        assert_eq!(fast, placement_ok_scan(&s), "trial {trial}");
        agree[fast as usize] += 1;
    }
    assert!(agree[0] > 50 && agree[1] > 50, "{agree:?}");
}
