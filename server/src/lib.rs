//! Shell around the AI Town kernel. It authenticates, stores rows, and renders JSON.
//! Game rules stay in `ai-town-kernel`.

mod agent;
mod cast;
#[cfg(feature = "gpu")]
mod gpu_move;
mod http;
mod map;
mod world;

use ai_town_kernel as k;
use axum::http::StatusCode;
use h5i_app::http::Api;
use h5i_app::json::Value as Out;
use h5i_app::pg::{DbError, Engine, ReplyCodec, Store, Tx};
use h5i_app::{Kernel, TenantId, Timestamp};
use serde::Deserialize;
use std::sync::Arc;

pub use agent::{drain, engine_actor, user_actor, Provider, TENANT};
pub use http::serve;
pub use map::gentle;
pub use world::to_json;

pub struct Town;

impl Kernel for Town {
    type Principal = k::Principal;
    type Snapshot = k::Snapshot;
    type Command = k::Command;
    type WriteSet = Vec<k::Write>;
    type Reply = k::Reply;
    type Error = k::Error;

    fn tenant(actor: &k::Principal) -> TenantId {
        TenantId(actor.org)
    }

    fn transition(actor: &k::Principal, snap: &k::Snapshot, cmd: &k::Command) -> Result<(Vec<k::Write>, k::Reply), k::Error> {
        k::transition(actor, snap, cmd)
    }

    fn apply(snap: &k::Snapshot, ws: &Vec<k::Write>) -> k::Snapshot {
        k::apply(snap, ws)
    }

    fn stamp(actor: &mut k::Principal, now: Timestamp) {
        actor.now_ms = now.micros() / 1000;
    }

    fn read_only(cmd: &k::Command) -> bool {
        k::read_only(cmd)
    }
}

pub fn principal(org: u64, user: u64) -> k::Principal {
    k::Principal { org, user, now_ms: 0 }
}

k::town_tables!(Town);

pub struct TownStore;

impl Store<Town> for TownStore {
    fn ddl() -> Vec<String> {
        schema_ddl()
    }

    fn tables() -> Vec<&'static str> {
        schema_tables()
    }

    async fn load(tx: &Tx<'_>, tenant: TenantId) -> Result<k::Snapshot, DbError> {
        schema_load(tx, tenant).await
    }

    async fn write(tx: &Tx<'_>, tenant: TenantId, ws: &Vec<k::Write>) -> Result<(), DbError> {
        schema_store(tx, tenant, ws).await
    }
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
enum CommandJson {
    Init { width: u32, height: u32, mask: Vec<u8> },
    CreateAgent { name: String, character: u32, identity: String, plan: String },
    Join { name: String, character: u32 },
    Leave,
    MoveTo { x: u32, y: u32 },
    Stop,
    StartConversation { invitee: u64 },
    AcceptInvite,
    RejectInvite,
    LeaveConversation,
    StartTyping,
    Say { text: String },
    FinishOp,
    Remember { summary: String },
    Activity { kind: u32 },
    Step,
    Freeze,
    Resume,
}

impl From<CommandJson> for k::Command {
    fn from(c: CommandJson) -> Self {
        match c {
            CommandJson::Init { width, height, mask } => k::Command::Init { width, height, mask },
            CommandJson::CreateAgent { name, character, identity, plan } => k::Command::CreateAgent {
                name: name.into_bytes(),
                character,
                identity: identity.into_bytes(),
                plan: plan.into_bytes(),
            },
            CommandJson::Join { name, character } => k::Command::Join { name: name.into_bytes(), character },
            CommandJson::Leave => k::Command::Leave,
            CommandJson::MoveTo { x, y } => k::Command::MoveTo { x, y },
            CommandJson::Stop => k::Command::Stop,
            CommandJson::StartConversation { invitee } => k::Command::StartConversation { invitee },
            CommandJson::AcceptInvite => k::Command::AcceptInvite,
            CommandJson::RejectInvite => k::Command::RejectInvite,
            CommandJson::LeaveConversation => k::Command::LeaveConversation,
            CommandJson::StartTyping => k::Command::StartTyping,
            CommandJson::Say { text } => k::Command::Say { text: text.into_bytes() },
            CommandJson::FinishOp => k::Command::FinishOp,
            CommandJson::Remember { summary } => k::Command::Remember { summary: summary.into_bytes() },
            CommandJson::Activity { kind } => k::Command::Activity { kind },
            CommandJson::Step => k::Command::Step,
            CommandJson::Freeze => k::Command::Freeze,
            CommandJson::Resume => k::Command::Resume,
        }
    }
}

impl Api<Town> for TownStore {
    fn decode_command(body: serde_json::Value) -> Result<k::Command, String> {
        serde_json::from_value::<CommandJson>(body).map(Into::into).map_err(|e| e.to_string())
    }

    fn encode_reply(r: &k::Reply) -> Out {
        match r {
            k::Reply::Ok => Out::obj([("ok", true.into())]),
            k::Reply::Player(id) => Out::obj([("player", (*id).into())]),
            k::Reply::Conversation(id) => Out::obj([("conversation", (*id).into())]),
            k::Reply::Stepped { now_ms, ticks } => Out::obj([("nowMs", (*now_ms).into()), ("ticks", u64::from(*ticks).into())]),
        }
    }

    fn encode_error(e: &k::Error) -> (StatusCode, Out) {
        let (status, code) = match e {
            k::Error::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            k::Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            k::Error::BadText => (StatusCode::UNPROCESSABLE_ENTITY, "bad_text"),
            k::Error::Busy => (StatusCode::CONFLICT, "busy"),
            k::Error::Full => (StatusCode::CONFLICT, "full"),
            k::Error::Occupied => (StatusCode::CONFLICT, "occupied"),
            k::Error::Already => (StatusCode::CONFLICT, "already"),
            k::Error::BadState => (StatusCode::UNPROCESSABLE_ENTITY, "bad_state"),
            k::Error::Uninit => (StatusCode::UNPROCESSABLE_ENTITY, "uninit"),
            k::Error::Limit => (StatusCode::UNPROCESSABLE_ENTITY, "limit"),
        };
        (status, h5i_app::http::error_body(code))
    }
}

impl ReplyCodec<Town> for TownStore {
    fn fingerprint(cmd: &k::Command) -> Vec<u8> {
        format!("{cmd:?}").into_bytes()
    }

    fn scope(actor: &k::Principal) -> String {
        format!("u{}", actor.user)
    }

    fn encode(r: &k::Reply) -> Vec<u8> {
        match r {
            k::Reply::Ok => b"ok".to_vec(),
            k::Reply::Player(id) => format!("p:{id}").into_bytes(),
            k::Reply::Conversation(id) => format!("c:{id}").into_bytes(),
            k::Reply::Stepped { now_ms, ticks } => format!("s:{now_ms}:{ticks}").into_bytes(),
        }
    }

    fn decode(bytes: &[u8]) -> Result<k::Reply, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "reply is not utf-8".to_string())?;
        if text == "ok" {
            return Ok(k::Reply::Ok);
        }
        if let Some(rest) = text.strip_prefix("p:") {
            return rest.parse().map(k::Reply::Player).map_err(|_| "bad player reply".into());
        }
        if let Some(rest) = text.strip_prefix("c:") {
            return rest.parse().map(k::Reply::Conversation).map_err(|_| "bad conversation reply".into());
        }
        if let Some(rest) = text.strip_prefix("s:") {
            let (now, ticks) = rest.split_once(':').ok_or("bad step reply")?;
            let now_ms = now.parse().map_err(|_| "bad step time")?;
            let ticks = ticks.parse().map_err(|_| "bad step ticks")?;
            return Ok(k::Reply::Stepped { now_ms, ticks });
        }
        Err("unknown reply".into())
    }
}

/// Seed the map and the five residents when the tenant is empty.
pub async fn bootstrap(engine: &Engine<Town, TownStore>) -> Result<(), String> {
    let map = gentle()?;
    let snap = engine.snapshot(TenantId(TENANT)).await.map_err(|e| e.to_string())?;
    if snap.meta.width == 0 {
        accept(engine, k::Command::Init { width: map.width, height: map.height, mask: map.mask }).await?;
    }
    let snap = engine.snapshot(TenantId(TENANT)).await.map_err(|e| e.to_string())?;
    if snap.agents.is_empty() {
        for person in cast::CAST {
            accept(
                engine,
                k::Command::CreateAgent {
                    name: person.name.as_bytes().to_vec(),
                    character: person.character,
                    identity: person.identity.as_bytes().to_vec(),
                    plan: person.plan.as_bytes().to_vec(),
                },
            )
            .await?;
        }
    }
    Ok(())
}

async fn accept(engine: &Engine<Town, TownStore>, cmd: k::Command) -> Result<(), String> {
    match engine.execute(&engine_actor(), &cmd).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("bootstrap refused: {e:?}")),
        Err(_) => Err("bootstrap failed".into()),
    }
}

pub fn next_user_seed(snap: &k::Snapshot) -> u64 {
    let mut max = 0u64;
    for p in &snap.players {
        if p.is_human && p.owner > max && p.owner < k::AGENT_BASE {
            max = p.owner;
        }
    }
    max + 1
}

pub async fn publish(engine: &Engine<Town, TownStore>, events: &tokio::sync::broadcast::Sender<String>) {
    match engine.snapshot(TenantId(TENANT)).await {
        Ok(snap) => {
            let _ = events.send(to_json(&snap));
        }
        Err(_) => tracing::warn!("snapshot failed"),
    }
}

impl axum::extract::FromRef<AppState> for h5i_app::http::Auth<Town> {
    fn from_ref(state: &AppState) -> Self {
        h5i_app::http::Auth::<Town>::from_ref(&state.app)
    }
}

pub fn engine(url: &str) -> Result<Engine<Town, TownStore>, DbError> {
    let url = h5i_app::pg::with_schema(url, "ai_town")?;
    Ok(Engine::new(h5i_app::pg::pool(&url, 8)?, h5i_app::pg::EngineConfig::default()))
}

/// Shared by the routes and the sim loop.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine<Town, TownStore>>,
    pub app: h5i_app::http::H5iApp<Town, TownStore>,
    pub auth: Arc<h5i_app::http::HmacAuth<Town>>,
    pub events: tokio::sync::broadcast::Sender<String>,
    pub users: Arc<std::sync::atomic::AtomicU64>,
    pub provider: Arc<Provider>,
}
