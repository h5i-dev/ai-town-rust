//! HTTP move, then the sim tick, then the websocket snapshot.
//!
//! Uses database `ai_town_pipe` and `127.0.0.1:18081`. `AI_TOWN_MOVE=gpu`
//! selects the GPU tick when this package is built with `--features gpu`.
//! A build without that feature rejects the GPU flag. `AI_TOWN_DRAIN=off`
//! keeps the agent worker out of the measurement, so the crowd stays idle.
//!
//! On a packed map the first free tile in owner-id order can be enclosed.
//! `begin_move` then returns success and leaves the player in place. The
//! session id is the spawn salt, so this binary keeps minting sessions until
//! that salt is a free tile with a free orthogonal step. The `move_to` target
//! is that neighbor. A far tile can be unreachable from a one-tile pocket, and
//! `begin_move` still returns success. After the position moves it sends
//! `stop`, so that walker does not occupy the next crowd's exit. A join whose
//! snapshot has no free step leaves and tries again.

use ai_town_kernel::{Command, ENGINE};
use futures_util::StreamExt;
use std::process::Command as Proc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("error,ai_town_server::http=info").with_ansi(false).init();
    #[cfg(not(feature = "gpu"))]
    if std::env::var("AI_TOWN_MOVE").ok().as_deref() == Some("gpu") {
        panic!("GPU pipe requires --features gpu");
    }
    let base = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://h5i:h5i@127.0.0.1:55432/h5i".into());
    let url = match pipe_database_url(&base) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    if let Err(err) = reset_pipe_database() {
        eprintln!("{err}");
        std::process::exit(1);
    }
    if let Err(err) = open_world(&url).await {
        eprintln!("{err}");
        std::process::exit(1);
    }
    let addr = "127.0.0.1:18081";
    let url_for_task = url.clone();
    let served = tokio::spawn(async move {
        if ai_town_server::serve(&url_for_task, "pipe-secret", addr, ai_town_server::Provider::Stub).await.is_err() {
            eprintln!("pipe server stopped");
        }
    });
    if !wait_health(addr).await {
        served.abort();
        eprintln!("pipe server did not become healthy");
        std::process::exit(1);
    }
    let mode = std::env::var("AI_TOWN_MOVE").unwrap_or_else(|_| "cpu".into());
    let result = sample(&url, addr, &mode).await;
    served.abort();
    match result {
        Ok(()) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

fn pipe_database_url(base: &str) -> Result<String, String> {
    let (head, query) = match base.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (base, None),
    };
    let slash = head.rfind('/').ok_or("DATABASE_URL has no database name")?;
    let mut url = format!("{}/ai_town_pipe", &head[..slash]);
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    Ok(url)
}

fn reset_pipe_database() -> Result<(), String> {
    let container = std::env::var("AI_TOWN_PG_CONTAINER").unwrap_or_else(|_| "ai-town-rust-postgres-1".into());
    // Each statement is its own `psql -c`. One script is one transaction, and
    // DROP DATABASE cannot run inside a transaction block.
    let created = Proc::new("docker")
        .args([
            "exec",
            &container,
            "psql",
            "-U",
            "h5i",
            "-d",
            "postgres",
            "-c",
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'ai_town_pipe' AND pid <> pg_backend_pid();",
            "-c",
            "DROP DATABASE IF EXISTS ai_town_pipe;",
            "-c",
            "CREATE DATABASE ai_town_pipe;",
        ])
        .output()
        .map_err(|_| "docker exec failed".to_string())?;
    if created.status.success() {
        Ok(())
    } else {
        Err("could not create the pipe database".into())
    }
}

async fn open_world(url: &str) -> Result<(), String> {
    let eng = ai_town_server::engine(url).map_err(|_| "database connection failed")?;
    eng.install_schema().await.map_err(|_| "database connection failed")?;
    let side = 128u32;
    let cells = (side as usize) * (side as usize);
    let mask = vec![0u8; (cells - 1) / 8 + 1];
    match eng.execute(&ai_town_server::user_actor(ENGINE), &Command::Init { width: side, height: side, mask }).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Err("init refused".into()),
        Err(_) => Err("init failed".into()),
    }
}

async fn wait_health(addr: &str) -> bool {
    let url = format!("http://{addr}/healthz");
    for _ in 0..50 {
        if reqwest::get(&url).await.ok().is_some_and(|r| r.status().is_success()) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn sample(database_url: &str, addr: &str, mode: &str) -> Result<(), String> {
    let eng = ai_town_server::engine(database_url).map_err(|_| "database connection failed")?;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let samples: usize = std::env::var("AI_TOWN_BENCH_SAMPLES")
        .unwrap_or_else(|_| "5".into()).parse().map_err(|_| "invalid sample count")?;
    if samples == 0 { return Err("sample count must be positive".into()); }
    let dir = std::env::var("AI_TOWN_BENCH_DIR").unwrap_or_else(|_| "/dev/shm/ai-town-rust/bench".into());
    std::fs::create_dir_all(&dir).map_err(|_| "could not create the bench directory")?;
    let path = format!("{dir}/pipe-{mode}.csv");
    let mut rows = vec!["mode,agents,sample,input_ms,push_ms".to_string()];
    for target in [1_000usize, 10_000] {
        grow_agents(&eng, target).await?;
        for sample in 0..samples {
            let mut attempt = 0u32;
            loop {
                attempt += 1;
                if attempt > 8 {
                    return Err("joined player had no free step".into());
                }
                let (token, user) = session_on_open_tile(&client, &base).await?;
                let joined = client
                    .post(format!("{base}/rpc"))
                    .header("authorization", format!("Bearer {token}"))
                    .json(&serde_json::json!({"cmd": "join", "name": "Pipe", "character": 0}))
                    .send()
                    .await
                    .map_err(|_| "join request failed")?;
                if !joined.status().is_success() {
                    return Err("join was refused".into());
                }
                let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.map_err(|_| "websocket failed")?;
                let (mut _write, mut read) = socket.split();
                let first = next_text(&mut read, Duration::from_secs(30)).await.ok_or("websocket sent no snapshot")?;
                let first_clock = serde_json::from_str::<serde_json::Value>(&first)
                    .map_err(|_| "invalid snapshot")?["nowMs"].as_u64().ok_or("snapshot has no clock")?;
                let me = player_xy(&first, user).ok_or("joined player missing from snapshot")?;
                let Some((tx, ty)) = open_neighbor(&first, user) else {
                    let left = client
                        .post(format!("{base}/rpc"))
                        .header("authorization", format!("Bearer {token}"))
                        .json(&serde_json::json!({"cmd": "leave"}))
                        .send()
                        .await
                        .map_err(|_| "leave request failed")?;
                    if !left.status().is_success() {
                        return Err("leave was refused".into());
                    }
                    continue;
                };
                let started = Instant::now();
                let moved = client
                    .post(format!("{base}/rpc"))
                    .header("authorization", format!("Bearer {token}"))
                    .json(&serde_json::json!({"cmd": "move_to", "x": tx, "y": ty}))
                    .send()
                    .await
                    .map_err(|_| "move request failed")?;
                let input_ms = started.elapsed().as_secs_f64() * 1_000.0;
                if !moved.status().is_success() {
                    return Err("move_to was refused".into());
                }
                let mut push_ms = None;
                let deadline = Instant::now() + Duration::from_secs(60);
                while Instant::now() < deadline {
                    let left = deadline.saturating_duration_since(Instant::now());
                    let Some(text) = next_text(&mut read, left).await else { break };
                    if let Some((x, y)) = player_xy(&text, user) {
                        if x != me.0 || y != me.1 {
                            // Relocate preserves the clock; a CPU fallback advances it.
                            // Refuse to publish a CPU sample under the GPU label.
                            if mode == "gpu" {
                                let clock = serde_json::from_str::<serde_json::Value>(&text)
                                    .map_err(|_| "invalid snapshot")?["nowMs"].as_u64().ok_or("snapshot has no clock")?;
                                if clock != first_clock {
                                    return Err("GPU sample used CPU fallback (simulation clock advanced)".into());
                                }
                            }
                            push_ms = Some(started.elapsed().as_secs_f64() * 1_000.0);
                            break;
                        }
                    }
                }
                let push = push_ms.ok_or("position did not change on the websocket")?;
                let agents = eng.snapshot(h5i_app::TenantId(ai_town_server::TENANT)).await.map_err(|_| "snapshot failed")?.agents.len();
                println!("pipe mode={mode} agents={agents} sample={sample} input_ms={input_ms:.1} push_ms={push:.1}");
                rows.push(format!("{mode},{agents},{sample},{input_ms:.3},{push:.3}"));
                std::fs::write(&path, rows.join("\n") + "\n").map_err(|_| "could not write the pipe csv")?;
                let stopped = client
                    .post(format!("{base}/rpc"))
                    .header("authorization", format!("Bearer {token}"))
                    .json(&serde_json::json!({"cmd": "stop"}))
                    .send()
                    .await
                    .map_err(|_| "stop request failed")?;
                if !stopped.status().is_success() {
                    return Err("stop was refused".into());
                }
                let left = client
                    .post(format!("{base}/rpc"))
                    .header("authorization", format!("Bearer {token}"))
                    .json(&serde_json::json!({"cmd": "leave"}))
                    .send().await.map_err(|_| "leave request failed")?;
                if !left.status().is_success() { return Err("leave was refused".into()); }
                break;
            }
        }
    }
    std::fs::write(&path, rows.join("\n") + "\n").map_err(|_| "could not write the pipe csv")?;
    println!("wrote {path}");
    Ok(())
}

async fn next_text<S>(read: &mut S, wait: Duration) -> Option<String>
where
    S: futures_util::Stream<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let timeout = tokio::time::timeout(wait, read.next()).await.ok()??;
    match timeout.ok()? {
        tokio_tungstenite::tungstenite::Message::Text(text) => Some(text.to_string()),
        _ => None,
    }
}

/// Tile coordinates of the first orthogonal neighbor whose center is outside
/// the collision radius of every other player. `move_to` uses that tile.
/// A farther tile can be unreachable from a one-tile pocket, and `begin_move`
/// still returns success after it clears the path.
fn open_neighbor(text: &str, owner: u64) -> Option<(u64, u64)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let width = value.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = value.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
    let (x, y) = player_xy(text, owner)?;
    let players = value.get("players").and_then(|v| v.as_array())?;
    let tx = x / 1000;
    let ty = y / 1000;
    let steps = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)];
    let mut s = 0;
    while s < steps.len() {
        let nx = tx as i64 + steps[s].0;
        let ny = ty as i64 + steps[s].1;
        s += 1;
        if nx < 0 || ny < 0 || (nx as u64) >= width || (ny as u64) >= height {
            continue;
        }
        let cx = (nx as u64) * 1000;
        let cy = (ny as u64) * 1000;
        let mut blocked = false;
        for player in players {
            if player.get("owner").and_then(|v| v.as_u64()) == Some(owner) {
                continue;
            }
            let px = player.get("x").and_then(|v| v.as_u64()).unwrap_or(0);
            let py = player.get("y").and_then(|v| v.as_u64()).unwrap_or(0);
            let ddx = px.abs_diff(cx);
            let ddy = py.abs_diff(cy);
            if ddx * ddx + ddy * ddy < 750 * 750 {
                blocked = true;
                break;
            }
        }
        if !blocked {
            return Some((nx as u64, ny as u64));
        }
    }
    None
}

fn player_xy(text: &str, owner: u64) -> Option<(u64, u64)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let players = value.get("players")?.as_array()?;
    for player in players {
        if player.get("owner")?.as_u64()? == owner {
            return Some((player.get("x")?.as_u64()?, player.get("y")?.as_u64()?));
        }
    }
    None
}

/// Spawn salt whose tile is empty and has an empty orthogonal neighbor.
/// `begin_move` reports success when the player cannot leave the start.
async fn session_on_open_tile(client: &reqwest::Client, base: &str) -> Result<(String, u64), String> {
    let snap: serde_json::Value = client
        .get(format!("{base}/snapshot"))
        .send()
        .await
        .map_err(|_| "snapshot request failed")?
        .json()
        .await
        .map_err(|_| "snapshot response was not json")?;
    let width = snap.get("width").and_then(|v| v.as_u64()).ok_or("snapshot has no width")?;
    let height = snap.get("height").and_then(|v| v.as_u64()).ok_or("snapshot has no height")?;
    let players = snap.get("players").and_then(|v| v.as_array()).ok_or("snapshot has no players")?;
    let n = width.checked_mul(height).ok_or("map size overflowed")?;
    if n == 0 || n > 128 * 128u64 {
        return Err("map size is not usable".into());
    }
    let mut blocked = vec![false; n as usize];
    for player in players {
        let x = player.get("x").and_then(|v| v.as_u64()).ok_or("player x missing")?;
        let y = player.get("y").and_then(|v| v.as_u64()).ok_or("player y missing")?;
        let tx = x / 1000;
        let ty = y / 1000;
        let mut dx = -1i64;
        while dx <= 1 {
            let mut dy = -1i64;
            while dy <= 1 {
                let nx = tx as i64 + dx;
                let ny = ty as i64 + dy;
                if nx >= 0 && ny >= 0 && (nx as u64) < width && (ny as u64) < height {
                    let cx = (nx as u64) * 1000;
                    let cy = (ny as u64) * 1000;
                    let ddx = x.abs_diff(cx);
                    let ddy = y.abs_diff(cy);
                    if ddx * ddx + ddy * ddy < 750 * 750 {
                        blocked[(nx as u64 * height + ny as u64) as usize] = true;
                    }
                }
                dy += 1;
            }
            dx += 1;
        }
    }
    let mut good = vec![false; n as usize];
    let mut found = false;
    let mut tx = 0u64;
    while tx < width {
        let mut ty = 0u64;
        while ty < height {
            let i = (tx * height + ty) as usize;
            if !blocked[i] && open_step(&blocked, width, height, tx, ty) {
                good[i] = true;
                found = true;
            }
            ty += 1;
        }
        tx += 1;
    }
    if !found {
        return Err("no tile has a free step".into());
    }
    let mut tries = 0u64;
    while tries < n {
        let session: serde_json::Value = client
            .post(format!("{base}/session"))
            .send()
            .await
            .map_err(|_| "session request failed")?
            .json()
            .await
            .map_err(|_| "session response was not json")?;
        let token = session.get("token").and_then(|v| v.as_str()).ok_or("session did not return a token")?.to_string();
        let user = session.get("user").and_then(|v| v.as_u64()).ok_or("session did not return a user")?;
        if user != 0 && good[(user % n) as usize] {
            return Ok((token, user));
        }
        tries += 1;
    }
    Err("no open seat in one pass over the map".into())
}

fn open_step(blocked: &[bool], width: u64, height: u64, tx: u64, ty: u64) -> bool {
    let steps = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)];
    let mut s = 0;
    while s < steps.len() {
        let nx = tx as i64 + steps[s].0;
        let ny = ty as i64 + steps[s].1;
        if nx >= 0 && ny >= 0 && (nx as u64) < width && (ny as u64) < height {
            let j = (nx as u64 * height + ny as u64) as usize;
            if !blocked[j] {
                return true;
            }
        }
        s += 1;
    }
    false
}

async fn grow_agents(eng: &h5i_app::pg::Engine<ai_town_server::Town, ai_town_server::TownStore>, target: usize) -> Result<(), String> {
    let snap = eng.snapshot(h5i_app::TenantId(ai_town_server::TENANT)).await.map_err(|_| "snapshot failed")?;
    let mut have = snap.agents.len();
    while have < target {
        let cmd = Command::CreateAgent {
            name: format!("p{have}").into_bytes(),
            character: (have % 7) as u32,
            identity: b"pipe".to_vec(),
            plan: b"walk".to_vec(),
        };
        match eng.execute(&ai_town_server::user_actor(ENGINE), &cmd).await {
            Ok(Ok(_)) => have += 1,
            Ok(Err(_)) => return Err("create agent refused".into()),
            Err(_) => return Err("create agent failed".into()),
        }
    }
    Ok(())
}
