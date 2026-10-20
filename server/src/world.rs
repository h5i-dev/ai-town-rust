//! JSON the browser renders. Positions stay in milli-tiles.

use ai_town_kernel::{Conversation, Message, PathNode, Snapshot};

pub fn to_json(snap: &Snapshot) -> String {
    let players: Vec<serde_json::Value> = snap
        .players
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "owner": p.owner,
                "human": p.is_human,
                "name": text(&p.name),
                "character": p.character,
                "x": p.x,
                "y": p.y,
                "faceDx": p.face_dx,
                "faceDy": p.face_dy,
                "speed": p.speed,
                "destX": p.dest_x,
                "destY": p.dest_y,
                "pf": p.pf,
                "activity": p.activity,
                "activityUntil": p.activity_until,
                "path": path_json(&snap.paths, p.id),
            })
        })
        .collect();
    let conversations: Vec<serde_json::Value> = snap
        .conversations
        .iter()
        .map(|c| conv_json(c, snap))
        .collect();
    let messages: Vec<serde_json::Value> = snap.messages.iter().map(message_json).collect();
    let agents: Vec<serde_json::Value> = snap
        .agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "player": a.player,
                "owner": a.owner,
                "name": text(&a.name),
                "character": a.character,
                "identity": text(&a.identity),
                "plan": text(&a.plan),
                "op": a.op,
                "toRemember": a.to_remember,
            })
        })
        .collect();
    serde_json::json!({
        "nowMs": snap.meta.now_ms,
        "width": snap.meta.width,
        "height": snap.meta.height,
        "frozen": snap.meta.frozen,
        "players": players,
        "conversations": conversations,
        "messages": messages,
        "agents": agents,
    })
    .to_string()
}

fn conv_json(c: &Conversation, snap: &Snapshot) -> serde_json::Value {
    let mut members = Vec::new();
    for m in &snap.members {
        if m.conversation == c.id {
            members.push(serde_json::json!({
                "player": m.player,
                "status": m.status,
                "invited": m.invited,
                "started": m.started,
            }));
        }
    }
    serde_json::json!({
        "id": c.id,
        "creator": c.creator,
        "created": c.created,
        "typingPlayer": c.typing_player,
        "typingSince": c.typing_since,
        "lastAuthor": c.last_author,
        "lastAt": c.last_at,
        "numMessages": c.num_messages,
        "members": members,
    })
}

fn message_json(m: &Message) -> serde_json::Value {
    serde_json::json!({
        "id": m.id,
        "conversation": m.conversation,
        "author": m.author,
        "ts": m.ts,
        "text": text(&m.text),
    })
}

fn path_json(paths: &[PathNode], player: u64) -> Vec<serde_json::Value> {
    let mut nodes: Vec<&PathNode> = paths.iter().filter(|n| n.player == player).collect();
    nodes.sort_by_key(|n| n.seq);
    nodes
        .into_iter()
        .map(|n| {
            serde_json::json!({
                "seq": n.seq,
                "x": n.x,
                "y": n.y,
                "faceDx": n.face_dx,
                "faceDy": n.face_dy,
                "t": n.t,
            })
        })
        .collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
