//! HTTP server, one-second simulation, and the agent worker.

use ai_town_kernel::{Command, AGENT_BASE, ENGINE};
use ai_town_server::{bootstrap, drain, engine, next_user_seed, principal, publish, AppState, Provider, Town, TownStore, TENANT};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use h5i_app::http::{error_body, reply, Actor, HmacAuth};
use h5i_app::pg::Engine;
use h5i_app::TenantId;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set")?;
    let secret = std::env::var("H5I_APP_SECRET").map_err(|_| "H5I_APP_SECRET is not set")?;
    let addr = std::env::var("LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());

    let auth = Arc::new(HmacAuth::<Town>::new(secret, principal));
    if let Ok(spec) = std::env::var("H5I_APP_ISSUE") {
        let (org, user) = spec.split_once(':').ok_or("H5I_APP_ISSUE=org:user")?;
        let user: u64 = user.parse()?;
        if user == ENGINE || user >= AGENT_BASE {
            return Err("refusing to issue that user".into());
        }
        println!("{}", auth.issue(org.parse()?, user, 86_400));
        return Ok(());
    }

    let engine = Arc::new(engine(&url)?);
    engine.install_schema().await?;
    bootstrap(&engine).await?;
    let snap = engine.snapshot(TenantId(TENANT)).await?;
    let users = Arc::new(AtomicU64::new(next_user_seed(&snap)));
    let (events, _) = tokio::sync::broadcast::channel(32);
    let provider = Arc::new(Provider::from_env());
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

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn sim_loop(engine: Arc<Engine<Town, TownStore>>, events: tokio::sync::broadcast::Sender<String>, provider: Arc<Provider>) {
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    loop {
        let started = std::time::Instant::now();
        match engine.execute(&ai_town_server::user_actor(ENGINE), &Command::Step).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!(?e, "step refused"),
            Err(e) => tracing::error!(error = %e, "step failed"),
        }
        publish(&engine, &events).await;
        if busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
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
    let cmd = match <TownStore as h5i_app::http::Api<Town>>::decode_command(body) {
        Ok(cmd) => cmd,
        Err(m) => return reply(StatusCode::BAD_REQUEST, error_body(&m)),
    };
    match state.app.run(&actor, cmd, &headers).await {
        Ok(r) => {
            publish(&state.engine, &state.events).await;
            reply(StatusCode::OK, <TownStore as h5i_app::http::Api<Town>>::encode_reply(&r))
        }
        Err(res) => res,
    }
}

async fn snapshot(State(state): State<AppState>) -> axum::response::Response {
    match state.engine.snapshot(TenantId(TENANT)).await {
        Ok(snap) => (StatusCode::OK, [("content-type", "application/json")], ai_town_server::to_json(&snap)).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "snapshot failed");
            reply(StatusCode::SERVICE_UNAVAILABLE, error_body("unavailable"))
        }
    }
}

async fn map() -> axum::response::Response {
    match ai_town_server::gentle() {
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
        let text = ai_town_server::to_json(&snap);
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
