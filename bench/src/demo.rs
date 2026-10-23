//! Disposable town / crowd demo: native CPU rules, offline external operations.
use ai_town_kernel::{FACE_BIAS, MILLI, PF_MOVE, PathNode, Player, Snapshot, run_ticks};
use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::Message},
    response::IntoResponse,
    routing::get,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

const STEP_MS: u64 = 960; // 60 checked kernel ticks per snapshot.
const LEG_MS: u64 = 3_840;
const MAX_PLAYERS: usize = 14_400; // 120 x 120 plus eight tiles of margin.

#[derive(Clone)]
struct Demo {
    frames: watch::Receiver<Arc<String>>,
    map: Arc<String>,
}

fn world(n: usize) -> Snapshot {
    let columns = (n as f64).sqrt().ceil() as u32;
    let mut s = Snapshot::default();
    s.meta.width = (columns + 8).max(16);
    s.meta.height = (columns + 8).max(16);
    s.meta.next_id = n as u64 + 1;
    s.meta.now_ms = 1_000_000;
    s.terrain.mask = vec![0; (s.meta.width * s.meta.height).div_ceil(8) as usize];
    for i in 0..n {
        let x = (4 + i as u32 % columns) * MILLI;
        let y = (4 + i as u32 / columns) * MILLI;
        s.players.push(Player {
            id: i as u64 + 1,
            owner: 0,
            is_human: false,
            x,
            y,
            face_dx: FACE_BIAS + MILLI,
            face_dy: FACE_BIAS,
            speed: 0,
            dest_x: x,
            dest_y: y,
            pf: PF_MOVE,
            pf_started: s.meta.now_ms,
            wait_until: 0,
            last_input: s.meta.now_ms,
            activity: 0,
            activity_until: 0,
            character: (i % 8) as u32,
            name: format!("Player {}", i + 1).into_bytes(),
        });
    }
    route(&mut s, true);
    s
}

// The whole crowd walks in step, then turns around. No pathfinding or LLM calls.
fn route(s: &mut Snapshot, east: bool) {
    s.paths.clear();
    let face = if east {
        FACE_BIAS + MILLI
    } else {
        FACE_BIAS - MILLI
    };
    for p in &mut s.players {
        p.dest_x = if east { p.x + 2_880 } else { p.x - 2_880 };
        p.dest_y = p.y;
        p.face_dx = face;
        p.pf = PF_MOVE;
        p.pf_started = s.meta.now_ms;
        for (seq, x, t) in [
            (0, p.x, s.meta.now_ms),
            (1, p.dest_x, s.meta.now_ms + LEG_MS),
        ] {
            s.paths.push(PathNode {
                player: p.id,
                seq,
                x,
                y: p.y,
                face_dx: face,
                face_dy: FACE_BIAS,
                t,
            });
        }
    }
}

fn demo_map(side: u32) -> String {
    let mut map: serde_json::Value =
        serde_json::from_str(ai_town_server::gentle().unwrap().json).unwrap();
    map["width"] = side.into();
    map["height"] = side.into();
    // Grass tile from gentle.json; an open field matching the collision mask.
    map["bgTiles"] = serde_json::json!([vec![vec![271; side as usize]; side as usize]]);
    map["objectTiles"] = serde_json::json!([]);
    map["animatedSprites"] = serde_json::json!([]);
    map["demo"] = true.into();
    map.to_string()
}

async fn snapshot(State(d): State<Demo>) -> impl IntoResponse {
    (
        [("content-type", "application/json")],
        d.frames.borrow().as_ref().clone(),
    )
}
async fn map_json(State(d): State<Demo>) -> impl IntoResponse {
    (
        [("content-type", "application/json")],
        d.map.as_ref().clone(),
    )
}
async fn ws(State(d): State<Demo>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(move |mut socket| async move {
        let mut frames = d.frames;
        loop {
            let frame = frames.borrow_and_update().as_ref().clone();
            if socket.send(Message::Text(frame.into())).await.is_err() {
                break;
            }
            tokio::select! {
                result = frames.changed() => if result.is_err() { break; },
                _ = socket.recv() => break,
            }
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n = args
        .first()
        .map(|v| v.parse::<usize>())
        .transpose()?
        .unwrap_or(10_000);
    if !(1..=MAX_PLAYERS).contains(&n) {
        return Err(format!("players must be 1..={MAX_PLAYERS}").into());
    }
    let addr = args.get(1).map(String::as_str).unwrap_or("127.0.0.1:18080");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let fixture = args.get(2).filter(|p| !p.is_empty());
    let mut town = fixture.map(|p| ai_town_bench::Town::load(p));
    let mut s = if let Some(t) = &town {
        t.state.clone()
    } else {
        world(n)
    };
    let map_data = if let Some(t) = &town {
        let mut map = t.fixture["map"].clone();
        map["demoEngine"] = "rust".into();
        map.to_string()
    } else {
        demo_map(s.meta.width)
    };
    let map = Arc::new(map_data);
    let initial = town
        .as_ref()
        .map(|t| t.snapshot(0.0, 0))
        .unwrap_or_else(|| ai_town_server::to_json(&s));
    let (tx, frames) = watch::channel(Arc::new(initial));
    std::thread::spawn(move || {
        if let Some(ref mut town) = town {
            let mut ticks = 1;
            loop {
                let clock = Instant::now();
                let tick_ms = town.step(ticks);
                if tx
                    .send(Arc::new(town.snapshot(tick_ms, ticks * 16)))
                    .is_err()
                {
                    break;
                }
                std::thread::sleep(
                    Duration::from_millis(ticks * 16).saturating_sub(clock.elapsed()),
                );
                ticks = ((500.0 / (tick_ms / ticks as f64).max(0.1)) as u64).clamp(1, 60);
            }
            return;
        }
        let mut east = true;
        let mut turn_at = s.meta.now_ms + LEG_MS;
        loop {
            let clock = Instant::now();
            let end = s.meta.now_ms + STEP_MS;
            run_ticks(&mut s, end);
            if s.meta.now_ms >= turn_at {
                east = !east;
                route(&mut s, east);
                turn_at = s.meta.now_ms + LEG_MS;
            }
            let tick_ms = clock.elapsed().as_secs_f64() * 1000.0;
            if tx.send(Arc::new(ai_town_server::to_json(&s))).is_err() {
                break;
            }
            eprintln!(
                "demo: players={n} tick_ms={tick_ms:.1} now_ms={}",
                s.meta.now_ms
            );
            std::thread::sleep(Duration::from_millis(STEP_MS).saturating_sub(clock.elapsed()));
        }
    });
    eprintln!("Demo: {n} residents at http://{addr} (no database or LLM)");
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/snapshot", get(snapshot))
        .route("/map", get(map_json))
        .route("/ws", get(ws))
        .with_state(Demo { frames, map });
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crowd_moves_and_turns_without_collisions() {
        let mut s = world(10_000);
        let initial = s.players[0].x;
        assert!(ai_town_kernel::placement_ok(&s));
        for east in [true, false, true] {
            let before = s.players[0].x;
            for _ in 0..4 {
                let end = s.meta.now_ms + STEP_MS;
                assert_eq!(run_ticks(&mut s, end), 60);
            }
            assert_eq!(s.players.len(), 10_000);
            assert!(ai_town_kernel::placement_ok(&s));
            if east {
                assert_eq!(s.players[0].x, before + 2_880);
            } else {
                assert_eq!(s.players[0].x, initial);
            }
            route(&mut s, !east);
        }
    }
}
