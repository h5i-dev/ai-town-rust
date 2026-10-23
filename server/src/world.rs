//! JSON the browser renders. Positions stay in milli-tiles.

use base64::Engine;
use ai_town_kernel::{Conversation, Member, Message, PathNode, Snapshot};

pub fn to_json(snap: &Snapshot) -> String {
    to_value(snap).to_string()
}

pub fn to_value(snap: &Snapshot) -> serde_json::Value {
    to_value_with_options(snap, true, usize::MAX)
}

/// Select displayed history before allocating JSON rows for large demo frames.
pub fn to_value_with_options(snap: &Snapshot, include_paths: bool, message_limit: usize) -> serde_json::Value {
    // Index once: scanning every path for every player is quadratic for crowds.
    let mut paths = std::collections::HashMap::<u64, Vec<&PathNode>>::new();
    for node in &snap.paths {
        if include_paths {
            paths.entry(node.player).or_default().push(node);
        }
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
    let mut memberships = std::collections::HashMap::<u64, Vec<&Member>>::new();
    for member in &snap.members {
        memberships.entry(member.conversation).or_default().push(member);
    }
    let conversations: Vec<serde_json::Value> = snap
        .conversations
        .iter()
        .map(|c| conv_json(c, memberships.get(&c.id).map(Vec::as_slice).unwrap_or(&[])))
        .collect();
    let messages: Vec<serde_json::Value> = snap.messages.iter()
        .skip(snap.messages.len().saturating_sub(message_limit)).map(message_json).collect();
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
        "motionHistory": motion_history_json(snap),
        "width": snap.meta.width,
        "height": snap.meta.height,
        "frozen": snap.meta.frozen,
        "players": players,
        "conversations": conversations,
        "messages": messages,
        "agents": agents,
    })
}

fn conv_json(c: &Conversation, rows: &[&Member]) -> serde_json::Value {
    let mut members = Vec::new();
    for m in rows {
        members.push(serde_json::json!({
            "player": m.player,
            "status": m.status,
            "invited": m.invited,
            "started": m.started,
        }));
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
    fn compressed_tick_history_is_delivered_and_stale_or_truncated_data_is_omitted() {
        let mut s = Snapshot::default();
        s.meta.now_ms = 32;
        let packet = vec![187,242,5,8];
        let mut data = 9u64.to_le_bytes().to_vec();
        data.extend_from_slice(&(packet.len() as u32).to_le_bytes());
        data.extend_from_slice(&packet);
        s.motion_history = ai_town_kernel::MotionHistory { start_ms:0,end_ms:32,data };
        let json = to_value(&s);
        let h = &json["motionHistory"];
        assert_eq!(h["startMs"],0); assert_eq!(h["endMs"],32);
        assert_eq!(h["locations"][0]["playerId"],9);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(h["locations"][0]["buffer"].as_str().unwrap()).unwrap(),packet);
        s.meta.now_ms=48;
        assert!(to_value(&s)["motionHistory"].is_null());
        s.meta.now_ms=32; s.motion_history.data.pop();
        assert!(to_value(&s)["motionHistory"].is_null());
    }

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
        for id in 1..=3 {
            s.messages.push(Message { id, conversation: 1, author: 1, ts: id, text: b"hello".to_vec() });
        }
        let limited = to_value_with_options(&s, false, 2);
        assert_eq!(limited["players"][0]["path"], serde_json::json!([]));
        let ids: Vec<_> = limited["messages"].as_array().unwrap().iter().map(|m| m["id"].as_u64().unwrap()).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(to_value(&s)["messages"].as_array().unwrap().len(), 3);
    }
}

/// Each location packet uses the upstream HistoricalObject wire format.
fn motion_history_json(snap: &Snapshot) -> serde_json::Value {
    let h = &snap.motion_history;
    // GPU relocation and old snapshots have no tick history for this interval.
    if h.end_ms != snap.meta.now_ms || h.end_ms <= h.start_ms {
        return serde_json::Value::Null;
    }
    let mut locations = Vec::new();
    let mut data = h.data.as_slice();
    while data.len() >= 12 {
        let id = u64::from_le_bytes(data[..8].try_into().unwrap());
        let len = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        data = &data[12..];
        if len > data.len() { return serde_json::Value::Null; }
        locations.push(serde_json::json!({"playerId":id,
            "buffer":base64::engine::general_purpose::STANDARD.encode(&data[..len])}));
        data = &data[len..];
    }
    if !data.is_empty() { return serde_json::Value::Null; }
    serde_json::json!({"startMs":h.start_ms,"endMs":h.end_ms,"locations":locations})
}
