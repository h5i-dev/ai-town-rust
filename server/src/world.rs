//! JSON the browser renders. Positions stay in milli-tiles.

use ai_town_kernel::{Conversation, Message, PathNode, Snapshot};

pub fn to_json(snap: &Snapshot) -> String {
    // Index once: scanning every path for every player is quadratic for crowds.
    let mut paths = std::collections::HashMap::<u64, Vec<&PathNode>>::new();
    for node in &snap.paths {
        paths.entry(node.player).or_default().push(node);
    }
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
                "path": path_json(paths.get(&p.id).map(Vec::as_slice).unwrap_or(&[])),
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

fn path_json(paths: &[&PathNode]) -> Vec<serde_json::Value> {
    let mut nodes: Vec<&PathNode> = paths.to_vec();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_grouped_by_player_and_sorted_by_sequence() {
        let mut s = Snapshot::default();
        let p = ai_town_kernel::Player {
            id: 1,
            owner: 0,
            is_human: false,
            x: 0,
            y: 0,
            face_dx: 1000,
            face_dy: 1000,
            speed: 0,
            dest_x: 0,
            dest_y: 0,
            pf: 0,
            pf_started: 0,
            wait_until: 0,
            last_input: 0,
            activity: 0,
            activity_until: 0,
            character: 0,
            name: Vec::new(),
        };
        s.players = vec![p.clone(), ai_town_kernel::Player { id: 2, ..p }];
        for (player, seq) in [(1, 2), (99, 0), (1, 0), (1, 1)] {
            s.paths.push(PathNode {
                player,
                seq,
                x: 0,
                y: 0,
                face_dx: 1000,
                face_dy: 1000,
                t: seq,
            });
        }
        let json: serde_json::Value = serde_json::from_str(&to_json(&s)).unwrap();
        let nodes = json["players"][0]["path"].as_array().unwrap();
        assert_eq!(
            nodes
                .iter()
                .map(|n| n["seq"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(json["players"][1]["path"], serde_json::json!([]));
    }
}
