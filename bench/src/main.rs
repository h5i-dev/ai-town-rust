//! Tick rate and input latency. CSV files go under `/dev/shm/ai-town-rust/bench`.
//!
//! The in-memory run steps the kernel directly. The HTTP run starts a second
//! server on `127.0.0.1:18080` against the `ai_town_bench` database so it does
//! not share the dev server's world. The agent worker in that server is the stub.

use ai_town_kernel::{apply, transition, Command, Error, Principal, Reply, Snapshot, AGENT_BASE, ENGINE, MAX_DIM};
use std::process::Command as Proc;
use std::time::Instant;

const ORG: u64 = 1;

fn main() {
    let dir = std::env::var("AI_TOWN_BENCH_DIR").unwrap_or_else(|_| "/dev/shm/ai-town-rust/bench".into());
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {dir}: {e}"));

    let ticks = tick_rows();
    write_csv(&dir, "ticks.csv", "agents,steps,ticks,seconds,ticks_per_s,step_ms", &ticks);
    let batch = batch_rows();
    write_csv(&dir, "batch.csv", "agents,repeats,seconds,seeks_per_s", &batch);

    let rt = tokio::runtime::Builder::new_multi_thread().enable_time().enable_all().build().expect("runtime");
    let latency = rt.block_on(http_rows());
    match latency {
        Ok(rows) => write_csv(&dir, "latency.csv", "agents,samples,mean_ms,p50_ms", &rows),
        Err(reason) => {
            let path = format!("{dir}/latency.csv");
            std::fs::write(&path, format!("agents,samples,mean_ms,p50_ms\n# skipped: {reason}\n")).expect("write latency.csv");
            println!("http latency skipped: {reason}");
        }
    }
    println!("wrote {dir}/ticks.csv {dir}/batch.csv {dir}/latency.csv");
}

fn write_csv(dir: &str, name: &str, header: &str, rows: &[String]) {
    let mut body = String::from(header);
    body.push('\n');
    for row in rows {
        body.push_str(row);
        body.push('\n');
    }
    let path = format!("{dir}/{name}");
    std::fs::write(&path, body).unwrap_or_else(|e| panic!("write {path}: {e}"));
}

fn actor(user: u64, now: u64) -> Principal {
    Principal { org: ORG, user, now_ms: now }
}

fn run(s: &Snapshot, user: u64, now: u64, cmd: Command) -> Result<(Snapshot, Reply), Error> {
    let (ws, reply) = transition(&actor(user, now), s, &cmd)?;
    Ok((apply(s, &ws), reply))
}

fn open_map(side: u32) -> Snapshot {
    let cells = (side as usize) * (side as usize);
    let bytes = if cells == 0 { 0 } else { (cells - 1) / 8 + 1 };
    let mask = vec![0u8; bytes];
    let (s, reply) = run(&Snapshot::default(), ENGINE, 1_000, Command::Init { width: side, height: side, mask }).expect("init");
    assert_eq!(reply, Reply::Ok);
    s
}

fn map_side(n: usize) -> u32 {
    let mut side = 48u32;
    while (side as usize) * (side as usize) < n && side < MAX_DIM {
        side = (side * 2).min(MAX_DIM);
    }
    side
}

fn tick_rows() -> Vec<String> {
    let mut rows = Vec::new();
    for n in [1usize, 8, 16, 32, 64, 128, 512, 1024, 4096, 10_000] {
        let side = map_side(n);
        let mut s = open_map(side);
        let mut now = 1_000u64;
        let mut owners = Vec::new();
        for i in 0..n {
            let cmd = Command::CreateAgent {
                name: format!("a{i}").into_bytes(),
                character: (i % 7) as u32,
                identity: b"bench".to_vec(),
                plan: b"walk".to_vec(),
            };
            match run(&s, ENGINE, now, cmd) {
                Ok((next, Reply::Player(id))) => {
                    s = next;
                    owners.push(AGENT_BASE + id);
                }
                Ok(_) => break,
                Err(_) => break,
            }
        }
        let placed = owners.len();
        let span = side - 2;
        for (k, owner) in owners.iter().enumerate() {
            let tx = 1 + (k as u32) % span;
            let ty = 1 + ((k as u32) / span) % span;
            if let Ok((next, _)) = run(&s, *owner, now, Command::MoveTo { x: tx, y: ty }) {
                s = next;
            }
        }
        now += 1_000;
        if let Ok((next, _)) = run(&s, ENGINE, now, Command::Step) {
            s = next;
        }
        let steps = 4u32;
        let started = Instant::now();
        let mut ticks = 0u64;
        for _ in 0..steps {
            now += 1_000;
            match run(&s, ENGINE, now, Command::Step) {
                Ok((next, Reply::Stepped { ticks: n, .. })) => {
                    s = next;
                    ticks += u64::from(n);
                }
                Ok((next, _)) => s = next,
                Err(_) => break,
            }
        }
        let seconds = started.elapsed().as_secs_f64().max(1e-9);
        let rate = ticks as f64 / seconds;
        let step_ms = seconds * 1_000.0 / f64::from(steps);
        rows.push(format!("{placed},{steps},{ticks},{seconds:.6},{rate:.1},{step_ms:.3}"));
        println!("ticks agents={placed} ticks/s={rate:.0} step_ms={step_ms:.2}");
    }
    rows
}

fn batch_rows() -> Vec<String> {
    let mut rows = Vec::new();
    for n in [128usize, 1024, 4096] {
        let width = 64u32;
        let height = 64u32;
        let cells = (width as usize) * (height as usize);
        let mask = vec![0u8; (cells - 1) / 8 + 1];
        let mut xs = Vec::with_capacity(n);
        let mut ys = Vec::with_capacity(n);
        let mut txs = Vec::with_capacity(n);
        let mut tys = Vec::with_capacity(n);
        for i in 0..n {
            xs.push(((i % 64) as u32) * 1000 + 100);
            ys.push(((i / 64) as u32 % 64) * 1000 + 100);
            txs.push(((i * 3) % 64) as u32);
            tys.push(((i * 5) % 64) as u32);
        }
        let repeats = if n >= 4096 { 1u32 } else { 3 };
        let started = Instant::now();
        for _ in 0..repeats {
            let (bx, by) = ai_town_kernel::batch_seek(&xs, &ys, &txs, &tys, 200, &mask, width, height);
            let (sx, sy) = ai_town_kernel::scalar_seek(&xs, &ys, &txs, &tys, 200, &mask, width, height);
            assert_eq!(bx, sx);
            assert_eq!(by, sy);
            std::hint::black_box(&bx);
        }
        let seconds = started.elapsed().as_secs_f64().max(1e-9);
        let rate = f64::from(repeats) * n as f64 / seconds;
        rows.push(format!("{n},{repeats},{seconds:.6},{rate:.1}"));
        println!("batch agents={n} seeks/s={rate:.0}");
    }
    rows
}

fn bench_database_url(base: &str) -> Result<String, String> {
    let (head, query) = match base.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (base, None),
    };
    let slash = head.rfind('/').ok_or("DATABASE_URL has no database name")?;
    if slash + 1 >= head.len() {
        return Err("DATABASE_URL has no database name".into());
    }
    let mut url = format!("{}/ai_town_bench", &head[..slash]);
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    Ok(url)
}

fn ensure_bench_database() -> Result<(), String> {
    let container = std::env::var("AI_TOWN_PG_CONTAINER").unwrap_or_else(|_| "ai-town-rust-postgres-1".into());
    let probe = Proc::new("docker")
        .args(["exec", &container, "psql", "-U", "h5i", "-d", "postgres", "-tAc", "SELECT 1 FROM pg_database WHERE datname = 'ai_town_bench'"])
        .output()
        .map_err(|_| "docker exec failed".to_string())?;
    if !probe.status.success() {
        return Err("could not query postgres for the bench database".into());
    }
    if String::from_utf8_lossy(&probe.stdout).contains('1') {
        return Ok(());
    }
    let created = Proc::new("docker")
        .args(["exec", &container, "psql", "-U", "h5i", "-d", "postgres", "-c", "CREATE DATABASE ai_town_bench"])
        .output()
        .map_err(|_| "docker exec failed".to_string())?;
    if created.status.success() {
        Ok(())
    } else {
        Err("could not create the bench database".into())
    }
}

async fn http_rows() -> Result<Vec<String>, String> {
    let base = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set")?;
    let url = bench_database_url(&base)?;
    ensure_bench_database()?;
    let addr = "127.0.0.1:18080";
    let url_for_task = url.clone();
    let served = tokio::spawn(async move {
        if let Err(_err) = ai_town_server::serve(&url_for_task, "bench-secret", addr, ai_town_server::Provider::Stub).await {
            eprintln!("bench server stopped");
        }
    });
    if !wait_health(addr).await {
        served.abort();
        return Err("bench server did not become healthy".into());
    }
    let result = sample_latency(addr, &url).await;
    served.abort();
    result
}

async fn wait_health(addr: &str) -> bool {
    let url = format!("http://{addr}/healthz");
    for _ in 0..50 {
        if reqwest::get(&url).await.ok().is_some_and(|r| r.status().is_success()) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

async fn sample_latency(addr: &str, database_url: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session: serde_json::Value = client
        .post(format!("{base}/session"))
        .send()
        .await
        .map_err(|_| "session request failed")?
        .json()
        .await
        .map_err(|_| "session response was not json")?;
    let token = session.get("token").and_then(|v| v.as_str()).ok_or("session did not return a token")?.to_string();
    let joined = client
        .post(format!("{base}/rpc"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({"cmd": "join", "name": "Bench", "character": 0}))
        .send()
        .await
        .map_err(|_| "join request failed")?;
    if !joined.status().is_success() {
        return Err("join was refused".into());
    }

    let eng = ai_town_server::engine(database_url).map_err(|_| "database connection failed")?;
    let mut rows = Vec::new();
    for target in [5usize, 16, 32] {
        grow_agents(&eng, target).await?;
        let snap = eng.snapshot(h5i_app::TenantId(ai_town_server::TENANT)).await.map_err(|_| "snapshot failed")?;
        let agents = snap.agents.len();
        let mut samples = Vec::new();
        for k in 0..6 {
            let body = serde_json::json!({"cmd": "move_to", "x": 2 + (k % 5), "y": 3 + (k % 4)});
            let started = Instant::now();
            let res = client
                .post(format!("{base}/rpc"))
                .header("authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|_| "move request failed")?;
            let ms = started.elapsed().as_secs_f64() * 1_000.0;
            if res.status().is_success() {
                samples.push(ms);
            }
        }
        if samples.len() >= 2 {
            samples.remove(0);
        }
        if samples.is_empty() {
            return Err("move_to did not succeed".into());
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let p50 = samples[samples.len() / 2];
        rows.push(format!("{agents},{},{mean:.3},{p50:.3}", samples.len()));
        println!("latency agents={agents} mean_ms={mean:.2} p50_ms={p50:.2}");
    }
    Ok(rows)
}

async fn grow_agents(eng: &h5i_app::pg::Engine<ai_town_server::Town, ai_town_server::TownStore>, target: usize) -> Result<(), String> {
    let snap = eng.snapshot(h5i_app::TenantId(ai_town_server::TENANT)).await.map_err(|_| "snapshot failed")?;
    let mut have = snap.agents.len();
    while have < target {
        let cmd = Command::CreateAgent {
            name: format!("b{have}").into_bytes(),
            character: (have % 7) as u32,
            identity: b"bench".to_vec(),
            plan: b"walk".to_vec(),
        };
        match eng.execute(&ai_town_server::user_actor(ENGINE), &cmd).await {
            Ok(Ok(_)) => have += 1,
            _ => break,
        }
    }
    Ok(())
}
