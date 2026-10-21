//! HTTP routes and the one-second simulation loop.

use crate::{bootstrap, drain, engine, next_user_seed, principal, publish, to_json, AppState, Provider, Town, TownStore, TENANT};
use ai_town_kernel::{Command, AGENT_BASE, ENGINE};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use h5i_app::http::{error_body, reply, Actor, Api, HmacAuth};
use h5i_app::pg::Engine;
use h5i_app::TenantId;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Bind `addr`, install the schema, and run the sim loop until the listener ends.
/// `provider` is the agent worker. Pass [`Provider::Stub`] when the model must stay out of the loop.
pub async fn serve(url: &str, secret: &str, addr: &str, provider: Provider) -> Result<(), String> {
    let auth = Arc::new(HmacAuth::<Town>::new(secret.to_string(), principal));
    let engine = Arc::new(engine(url).map_err(|_| "database connection failed".to_string())?);
    engine.install_schema().await.map_err(|_| "database connection failed".to_string())?;
    bootstrap(&engine).await?;
    let snap = engine.snapshot(TenantId(TENANT)).await.map_err(|_| "database connection failed".to_string())?;
    let users = Arc::new(AtomicU64::new(next_user_seed(&snap)));
    let (events, _) = tokio::sync::broadcast::channel(32);
    let provider = Arc::new(provider);
    let tick = Arc::new(tokio::sync::Mutex::new(()));
    tracing::info!(provider = ?provider, agents = snap.agents.len(), "world ready");

    let app = h5i_app::http::H5iApp::new(engine.clone(), auth.clone());
    let state = AppState {
        engine: engine.clone(),
        app,
        auth,
        events: events.clone(),
        users,
        provider: provider.clone(),
        tick: tick.clone(),
    };

    let sim_engine = engine.clone();
    let sim_events = events.clone();
    let sim_provider = provider.clone();
    tokio::spawn(async move { sim_loop(sim_engine, sim_events, sim_provider, tick).await });

    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/session", post(session))
        .route("/rpc", post(rpc))
        .route("/snapshot", get(snapshot))
        .route("/map", get(map))
        .route("/ws", get(ws))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| e.to_string())?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, router).await.map_err(|e| e.to_string())
}

fn gpu_requested() -> bool {
    std::env::var("AI_TOWN_MOVE").ok().as_deref() == Some("gpu")
}

/// The pipe bench sets this so a crowd of idle agents is not also pathfinding.
fn drain_enabled() -> bool {
    std::env::var("AI_TOWN_DRAIN").ok().as_deref() != Some("off")
}

fn sequential_ticks() -> bool {
    std::env::var("AI_TOWN_TICK").ok().as_deref() == Some("seq")
}

async fn cpu_step(engine: &Engine<Town, TownStore>) {
    let cmd = if sequential_ticks() { Command::StepSeq } else { Command::Step };
    match engine.execute(&crate::user_actor(ENGINE), &cmd).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::warn!(?e, "step refused"),
        Err(_) => tracing::error!("step failed"),
    }
}

#[cfg(feature = "gpu")]
async fn gpu_step(engine: &Engine<Town, TownStore>, mover: &std::sync::mpsc::Sender<crate::gpu_move::MoveReq>) -> bool {
    let snap = match engine.snapshot(h5i_app::TenantId(TENANT)).await {
        Ok(snap) => snap,
        Err(_) => return false,
    };
    if snap.meta.width == 0 {
        return false;
    }
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let req = crate::gpu_move::from_snapshot(&snap, reply_tx);
    if mover.send(req).is_err() {
        return false;
    }
    let positions = match tokio::time::timeout(Duration::from_secs(60), reply_rx).await {
        Ok(Ok(Some(pos))) => pos,
        _ => return false,
    };
    match engine.execute(&crate::user_actor(ENGINE), &Command::Relocate { x: positions.0, y: positions.1 }).await {
        Ok(Ok(_)) => true,
        _ => false,
    }
}

async fn sim_loop(
    engine: Arc<Engine<Town, TownStore>>,
    events: tokio::sync::broadcast::Sender<String>,
    provider: Arc<Provider>,
    tick: Arc<tokio::sync::Mutex<()>>,
) {
    #[cfg(feature = "gpu")]
    let mover = if gpu_requested() { Some(crate::gpu_move::spawn_mover()) } else { None };
    #[cfg(not(feature = "gpu"))]
    if gpu_requested() {
        tracing::error!("gpu step unavailable");
    }
    #[cfg(feature = "gpu")]
    let mut gpu_warned = false;
    let busy = Arc::new(AtomicBool::new(false));
    loop {
        let started = std::time::Instant::now();
        let _hold = tick.lock().await;
        #[cfg(feature = "gpu")]
        let mut used_gpu = false;
        #[cfg(not(feature = "gpu"))]
        let used_gpu = false;
        #[cfg(feature = "gpu")]
        if let Some(mover) = &mover {
            if gpu_step(&engine, mover).await {
                used_gpu = true;
            } else if !gpu_warned {
                tracing::error!("gpu step failed");
                gpu_warned = true;
            }
        }
        if !used_gpu {
            cpu_step(&engine).await;
        }
        let ms = started.elapsed().as_secs_f64() * 1_000.0;
        tracing::info!(ms, used_gpu, "sim tick");
        publish(&engine, &events).await;
        drop(_hold);
        if drain_enabled() && busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            let engine = engine.clone();
            let provider = provider.clone();
            let events = events.clone();
            let busy = busy.clone();
            tokio::spawn(async move {
                drain(&engine, &provider).await;
                publish(&engine, &events).await;
                busy.store(false, Ordering::Release);
            });
        }
        let elapsed = started.elapsed();
        if elapsed < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_secs(1) - elapsed).await;
        }
    }
}

async fn session(State(state): State<AppState>) -> axum::response::Response {
    let id = state.users.fetch_add(1, Ordering::Relaxed);
    if id == ENGINE || id >= AGENT_BASE {
        return reply(StatusCode::SERVICE_UNAVAILABLE, error_body("no free user id"));
    }
    let token = state.auth.issue(TENANT, id, 7 * 24 * 3600);
    reply(StatusCode::OK, h5i_app::json::Value::obj([("token", h5i_app::json::Value::str(token)), ("user", id.into())]))
}

async fn rpc(State(state): State<AppState>, actor: Actor<Town>, headers: HeaderMap, Json(body): Json<serde_json::Value>) -> axum::response::Response {
    let cmd = match <TownStore as Api<Town>>::decode_command(body) {
        Ok(cmd) => cmd,
        Err(m) => return reply(StatusCode::BAD_REQUEST, error_body(&m)),
    };
    let _hold = state.tick.lock().await;
    match state.app.run(&actor, cmd, &headers).await {
        Ok(r) => {
            publish(&state.engine, &state.events).await;
            reply(StatusCode::OK, <TownStore as Api<Town>>::encode_reply(&r))
        }
        Err(res) => res,
    }
}

async fn snapshot(State(state): State<AppState>) -> axum::response::Response {
    match state.engine.snapshot(TenantId(TENANT)).await {
        Ok(snap) => (StatusCode::OK, [("content-type", "application/json")], to_json(&snap)).into_response(),
        Err(_) => {
            tracing::warn!("snapshot failed");
            reply(StatusCode::SERVICE_UNAVAILABLE, error_body("unavailable"))
        }
    }
}

async fn map() -> axum::response::Response {
    match crate::gentle() {
        Ok(map) => (StatusCode::OK, [("content-type", "application/json")], map.json).into_response(),
        Err(_) => reply(StatusCode::SERVICE_UNAVAILABLE, error_body("map")),
    }
}

async fn ws(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let mut rx = state.events.subscribe();
    if let Ok(snap) = state.engine.snapshot(TenantId(TENANT)).await {
        let text = to_json(&snap);
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }
    }
}
