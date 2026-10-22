//! Turns `Agent.op` into ordinary commands. The model only supplies text.

use crate::llm::Provider;
use crate::Town;
use ai_town_kernel::{
    mask_blocked, Command, Error, Member, Player, Principal, Snapshot, ENGINE, MAX_PLAN, MAX_TEXT, MILLI, OP_DO,
    OP_REMEMBER, OP_SPEAK,
};
use h5i_app::pg::Engine;
use h5i_app::TenantId;

pub const TENANT: u64 = 1;

pub fn engine_actor() -> Principal {
    Principal { org: TENANT, user: ENGINE, now_ms: 0 }
}

pub fn user_actor(user: u64) -> Principal {
    Principal { org: TENANT, user, now_ms: 0 }
}

/// What an idle agent does. Conversation on every fourth second-bucket, otherwise a walk.
pub fn choose_do(snap: &Snapshot, index: usize) -> Option<Command> {
    if index >= snap.agents.len() {
        return None;
    }
    let agent = &snap.agents[index];
    let me = agent.player;
    let bucket = snap.meta.now_ms / 1000;
    if (agent.id + bucket) % 4 == 0 {
        if let Some(invitee) = invite_target(snap, me) {
            return Some(Command::StartConversation { invitee });
        }
    }
    let (tx, ty) = wander(snap, agent.id + me + bucket)?;
    Some(Command::MoveTo { x: tx, y: ty })
}

fn invite_target(snap: &Snapshot, me: u64) -> Option<u64> {
    let mut best_human: Option<u64> = None;
    let mut best_any: Option<u64> = None;
    for p in &snap.players {
        if p.id == me || in_conversation(&snap.members, p.id) {
            continue;
        }
        if p.is_human {
            best_human = Some(min_id(best_human, p.id));
        }
        best_any = Some(min_id(best_any, p.id));
    }
    best_human.or(best_any)
}

fn min_id(cur: Option<u64>, id: u64) -> u64 {
    match cur {
        Some(c) if c < id => c,
        _ => id,
    }
}

fn in_conversation(members: &[Member], player: u64) -> bool {
    members.iter().any(|m| m.player == player)
}

fn wander(snap: &Snapshot, salt: u64) -> Option<(u32, u32)> {
    let w = snap.meta.width;
    let h = snap.meta.height;
    if w == 0 || h == 0 {
        return None;
    }
    let n = (w as u64) * (h as u64);
    let mut k = 0u64;
    while k < n {
        let idx = salt.wrapping_add(k) % n;
        let tx = (idx / h as u64) as u32;
        let ty = (idx % h as u64) as u32;
        if !mask_blocked(&snap.terrain.mask, w, h, tx * MILLI, ty * MILLI) {
            return Some((tx, ty));
        }
        k += 1;
    }
    None
}

pub fn stub_line(name: &str, n: u64) -> String {
    const LINES: [&str; 4] = [
        "The square is quiet today.",
        "I have been thinking about that.",
        "Tell me something I do not know.",
        "I will remember this.",
    ];
    clip(&format!("{name}: {}", LINES[(n as usize) % LINES.len()]), MAX_TEXT)
}

pub fn stub_memory(other: &str, messages: u64) -> String {
    clip(&format!("Talked with {other} across {messages} messages."), MAX_PLAN)
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return "…".to_string();
    }
    s[..end].to_string()
}

pub async fn speak(provider: &Provider, name: &str, identity: &str, plan: &str, history: &str, n: u64) -> String {
    let fallback = stub_line(name, n);
    let system = format!("You are {name}. {identity} {plan} Reply with one short spoken line and nothing else.");
    let user = if history.is_empty() {
        "Start the conversation with one short line.".to_string()
    } else {
        format!("The conversation so far:\n{history}\nReply with your next line.")
    };
    match provider.complete(&system, &user, 0.7).await {
        Some(text) => clip(&text, MAX_TEXT),
        None => fallback,
    }
}

pub async fn remember_text(provider: &Provider, name: &str, other: &str, history: &str, messages: u64) -> String {
    let system = format!("You are {name}. Summarize the conversation in one sentence.");
    match provider.complete(&system, history, 0.2).await {
        Some(text) => clip(&text, MAX_PLAN),
        None => stub_memory(other, messages),
    }
}

/// Run every pending agent operation, then clear it.
pub async fn drain<S>(engine: &Engine<Town, S>, provider: &Provider)
where
    S: h5i_app::pg::Store<Town>,
{
    let snap = match engine.snapshot(TenantId(TENANT)).await {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!("agent snapshot failed");
            return;
        }
    };
    let mut i = 0;
    while i < snap.agents.len() {
        let agent = &snap.agents[i];
        let owner = agent.owner;
        let op = agent.op;
        let player = agent.player;
        let name = String::from_utf8_lossy(&agent.name).into_owned();
        let identity = String::from_utf8_lossy(&agent.identity).into_owned();
        let plan = String::from_utf8_lossy(&agent.plan).into_owned();
        let conv = agent.op_arg;
        let action = if op == OP_DO {
            choose_do(&snap, i)
        } else if op == OP_SPEAK {
            let history = history_of(&snap, conv);
            let n = message_count(&snap, conv);
            let text = speak(provider, &name, &identity, &plan, &history, n).await;
            Some(Command::Say { text: text.into_bytes() })
        } else if op == OP_REMEMBER {
            let other = other_name(&snap, conv, player);
            let history = history_of(&snap, agent.to_remember);
            let n = message_count(&snap, agent.to_remember);
            let text = remember_text(provider, &name, &other, &history, n).await;
            Some(Command::Remember { summary: text.into_bytes() })
        } else {
            None
        };
        if let Some(cmd) = action {
            let _ = run(engine, owner, cmd).await;
        }
        if op == OP_DO || op == OP_SPEAK || op == OP_REMEMBER {
            let _ = run(engine, owner, Command::FinishOp).await;
        }
        i += 1;
    }
}

async fn run<S>(engine: &Engine<Town, S>, user: u64, cmd: Command) -> Result<(), ()>
where
    S: h5i_app::pg::Store<Town>,
{
    match engine.execute(&user_actor(user), &cmd).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => {
            if !matches!(e, Error::Busy | Error::BadState | Error::NotFound) {
                tracing::debug!(?e, "agent command refused");
            }
            Err(())
        }
        Err(_) => {
            tracing::warn!("agent command failed");
            Err(())
        }
    }
}

fn history_of(snap: &Snapshot, conv: u64) -> String {
    let mut lines = Vec::new();
    for m in &snap.messages {
        if m.conversation == conv {
            let who = name_of(&snap.players, m.author);
            lines.push(format!("{who}: {}", String::from_utf8_lossy(&m.text)));
        }
    }
    lines.join("\n")
}

fn message_count(snap: &Snapshot, conv: u64) -> u64 {
    snap.messages.iter().filter(|m| m.conversation == conv).count() as u64
}

fn other_name(snap: &Snapshot, conv: u64, me: u64) -> String {
    for m in &snap.members {
        if m.conversation == conv && m.player != me {
            return name_of(&snap.players, m.player).to_string();
        }
    }
    "someone".into()
}

fn name_of(players: &[Player], id: u64) -> &str {
    for p in players {
        if p.id == id {
            // Names are short ASCII from the cast or the session form.
            return std::str::from_utf8(&p.name).unwrap_or("someone");
        }
    }
    "someone"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::gentle;
    use crate::Town;
    use ai_town_kernel::{collision_safe, exclusive, members_consistent, Command, Snapshot};
    use h5i_app::{MemoryEngine, TenantId};

    fn apply(engine: &MemoryEngine<Town>, user: u64, now: u64, cmd: Command) -> Snapshot {
        let actor = Principal { org: TENANT, user, now_ms: now };
        engine.execute(&actor, &cmd).expect("command");
        engine.snapshot(TenantId(TENANT))
    }

    #[test]
    fn provider_debug_does_not_print_a_key() {
        let provider = Provider::openai_compatible("http://localhost:4000/v1", "town-model", Some("sekret-value".into()), 1024)
            .expect("provider");
        let shown = format!("{provider:?}");
        assert_eq!(shown, "OpenAiCompatible(town-model)");
        assert!(!shown.contains("sekret"));
        assert_eq!(format!("{:?}", Provider::Stub), "Stub");
        let line = stub_line("Lucky", 1);
        assert!(line.starts_with("Lucky:"));
        assert!(line.len() <= MAX_TEXT);
    }

    #[test]
    fn gentle_town_spawns_steps_and_wanders() {
        let map = gentle().expect("map");
        let engine = MemoryEngine::<Town>::default();
        let now = 1_000u64;
        apply(&engine, ENGINE, now, Command::Init { width: map.width, height: map.height, mask: map.mask });
        for person in crate::cast::CAST {
            apply(
                &engine,
                ENGINE,
                now,
                Command::CreateAgent {
                    name: person.name.as_bytes().to_vec(),
                    character: person.character,
                    identity: person.identity.as_bytes().to_vec(),
                    plan: person.plan.as_bytes().to_vec(),
                },
            );
        }
        let snap = engine.snapshot(TenantId(TENANT));
        assert_eq!(snap.agents.len(), 5);
        assert!(collision_safe(&snap));
        let walk = choose_do(&snap, 0).expect("wander");
        assert!(matches!(walk, Command::MoveTo { .. }));
        let mut later = snap.meta.now_ms;
        // Force the conversation bucket for the first agent.
        let mut t = 0u64;
        while (snap.agents[0].id + t) % 4 != 0 {
            t += 1;
        }
        let mut posed = snap.clone();
        posed.meta.now_ms = t * 1000;
        let talk = choose_do(&posed, 0).expect("talk");
        assert!(matches!(talk, Command::StartConversation { .. }));

        let mut n = 0;
        while n < 3 {
            later += 1000;
            let snap = apply(&engine, ENGINE, later, Command::Step);
            assert!(collision_safe(&snap));
            assert!(exclusive(&snap));
            assert!(members_consistent(&snap));
            n += 1;
        }
    }

    fn inference_at(base: String, timeout_ms: u64) -> Provider {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .expect("client");
        Provider::OpenAiCompatible {
            model: "town-model".into(), base, api_key: Some("test-key".into()), client, max_tokens: 1024,
        }
    }

    async fn serve_completions(status: axum::http::StatusCode, body: &'static str) -> String {
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move || async move { (status, body) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn rate_limit_and_timeout_fall_back_to_the_stub() {
        let fallback = stub_line("Lucky", 1);
        let base = serve_completions(axum::http::StatusCode::TOO_MANY_REQUESTS, "slow down").await;
        let provider = inference_at(base, 1_000);
        let line = speak(&provider, "Lucky", "curious", "gossip", "", 1).await;
        assert_eq!(line, fallback);
        assert_eq!(remember_text(&provider, "Lucky", "Bob", "hello", 1).await, stub_memory("Bob", 1));

        let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            "{}"
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = inference_at(format!("http://{addr}/v1"), 50);
        let hung = speak(&provider, "Lucky", "curious", "gossip", "", 1).await;
        assert_eq!(hung, fallback);
        assert_eq!(remember_text(&provider, "Lucky", "Bob", "hello", 1).await, stub_memory("Bob", 1));
        task.abort();
    }

    #[tokio::test]
    async fn a_model_line_is_kept_when_the_request_succeeds() {
        let body = "{\"choices\":[{\"message\":{\"content\":\"The oaks were arguing.\"}}]}";
        let base = serve_completions(axum::http::StatusCode::OK, body).await;
        let line = speak(&inference_at(base, 1_000), "Lucky", "curious", "gossip", "", 1).await;
        assert_eq!(line, "The oaks were arguing.");
        assert_ne!(line, stub_line("Lucky", 1));
    }

    #[tokio::test]
    async fn memory_uses_the_same_inference_server() {
        let body = "{\"choices\":[{\"message\":{\"content\":\"We talked about the oaks.\"}}]}";
        let base = serve_completions(axum::http::StatusCode::OK, body).await;
        let summary = remember_text(&inference_at(base, 1_000), "Lucky", "Bob", "Bob: Hello.", 1).await;
        assert_eq!(summary, "We talked about the oaks.");
    }

    #[tokio::test]
    async fn unusable_responses_fall_back_for_dialogue_and_memory() {
        for body in ["not json", "{}", "{\"choices\":[]}", "{\"choices\":[{\"message\":{\"content\":\"  \"}}]}"] {
            let base = serve_completions(axum::http::StatusCode::OK, body).await;
            let provider = inference_at(base, 1_000);
            assert_eq!(speak(&provider, "Lucky", "curious", "gossip", "", 1).await, stub_line("Lucky", 1));
            assert_eq!(remember_text(&provider, "Lucky", "Bob", "hello", 1).await, stub_memory("Bob", 1));
        }
    }

    /// Run explicitly against a separately configured inference server.
    #[tokio::test]
    #[ignore = "requires LLM_BASE_URL and LLM_MODEL; makes live inference requests"]
    async fn live_inference_server() {
        let provider = Provider::from_env().expect("inference configuration");
        assert!(!matches!(provider, Provider::Stub), "configure LLM_BASE_URL and LLM_MODEL");
        let line = speak(&provider, "Lucky", "You are a curious town resident.", "Greet Bob.", "Bob: Hello!", 1).await;
        assert_ne!(line, stub_line("Lucky", 1), "dialogue fell back to the stub");
        assert!(!line.is_empty() && line.len() <= MAX_TEXT);
        let summary = remember_text(&provider, "Lucky", "Bob", &format!("Bob: Hello!\nLucky: {line}"), 2).await;
        assert_ne!(summary, stub_memory("Bob", 2), "memory fell back to the stub");
        assert!(!summary.is_empty() && summary.len() <= MAX_PLAN);
        println!("Dialogue: {line}\nMemory: {summary}");
    }
}
