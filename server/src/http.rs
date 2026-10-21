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
    tracing::info!(provider = ?provider, agents = snap.agents.len(), "world ready");

    let app = h5i_app::http::H5iApp::new(engine.clone(), auth.clone());
    let state = AppState { engine: engine.clone(), app, auth, events: events.clone(), users, provider: provider.clone() };

    let sim_engine = engine.clone();
    let sim_events = events.clone();
    let sim_provider = provider.clone();
    tokio::spawn(async move { sim_loop(sim_engine, sim_events, sim_provider).await });

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

async fn sim_loop(engine: Arc<Engine<Town, TownStore>>, events: tokio::sync::broadcast::Sender<String>, provider: Arc<Provider>) {
    let busy = Arc::new(AtomicBool::new(false));
    loop {
        let started = std::time::Instant::now();
        match engine.execute(&crate::user_actor(ENGINE), &Command::Step).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!(?e, "step refused"),
            Err(_) => tracing::error!("step failed"),
        }
        publish(&engine, &events).await;
        if busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
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
