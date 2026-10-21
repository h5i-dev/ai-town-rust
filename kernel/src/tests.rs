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
fn diff_roundtrip_matches_apply() {
    let s = world(3_000);
    let (s, _) = run(&s, 1, 3_000, Command::Join { name: b"Ada".to_vec(), character: 0 });
    let cmd = Command::MoveTo { x: 5, y: 5 };
    let (ws, _) = transition(&actor(1, 3_100), &s, &cmd).unwrap();
    let next = apply(&s, &ws);
    let again = rules::diff(&s, &next);
    assert_eq!(apply(&s, &again), next);
}
