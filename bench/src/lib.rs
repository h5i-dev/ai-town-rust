//! Offline town fixture shared by the demo and full game-loop benchmark.
use ai_town_kernel::*;
use serde_json::{json, Value};
use std::time::Instant;

pub const START: u64 = 1_000_000;
pub fn hash(mut x: u32) -> u32 {
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb352d);
    x = (x ^ (x >> 15)).wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}
fn u(v: &Value) -> u64 {
    v.as_u64().unwrap()
}
#[derive(Clone, Default, Debug)]
pub struct Metrics {
    pub decisions: u64,
    pub destinations: u64,
    pub activities: u64,
    pub invitations: u64,
    pub messages: u64,
    pub memories: u64,
    pub pathfinds: u64,
}
impl Metrics {
    pub fn json(&self) -> Value {
        json!({"decisions":self.decisions,"destinations":self.destinations,"activities":self.activities,
        "invitations":self.invitations,"messages":self.messages,"memories":self.memories,"pathfinds":self.pathfinds})
    }
}
pub struct Town {
    pub state: Snapshot,
    pub fixture: Value,
    pub metrics: Metrics,
    pub command_ms: Option<[f64; 6]>,
    pub history_pack_ms: f64,
    pub history_bytes: usize,
}
impl Town {
    pub fn load(path: &str) -> Self {
        let f: Value = serde_json::from_slice(&std::fs::read(path).expect("read fixture"))
            .expect("parse fixture");
        assert_eq!(u(&f["version"]), 1);
        let mut s = Snapshot::default();
        s.meta.width = u(&f["map"]["width"]) as u32;
        s.meta.height = u(&f["map"]["height"]) as u32;
        s.meta.now_ms = START;
        let players = f["players"].as_array().unwrap();
        s.meta.next_id = players.len() as u64 * 4 + 1;
        s.terrain.mask = vec![0; (s.meta.width as usize * s.meta.height as usize).div_ceil(8)];
        for layer in f["map"]["objectTiles"].as_array().unwrap() {
            for (x, col) in layer.as_array().unwrap().iter().enumerate() {
                for (y, tile) in col.as_array().unwrap().iter().enumerate() {
                    if tile.as_i64().unwrap() != -1 {
                        let bit = x * s.meta.height as usize + y;
                        s.terrain.mask[bit / 8] |= 1 << (bit % 8);
                    }
                }
            }
        }
        for p in players {
            let id = u(&p["id"]);
            let x = u(&p["x"]) as u32;
            let y = u(&p["y"]) as u32;
            let path = p["path"].as_array().unwrap();
            let moving = path.len() > 1;
            let (dx, dy) = if moving {
                (
                    u(&path.last().unwrap()[0]) as u32,
                    u(&path.last().unwrap()[1]) as u32,
                )
            } else {
                (x, y)
            };
            s.players.push(Player {
                id,
                owner: AGENT_BASE + id,
                is_human: false,
                x,
                y,
                face_dx: FACE_BIAS + MILLI,
                face_dy: FACE_BIAS,
                speed: 0,
                dest_x: dx,
                dest_y: dy,
                pf: if moving { PF_MOVE } else { PF_NONE },
                pf_started: START,
                wait_until: 0,
                last_input: START,
                activity: u(&p["activity"]) as u32,
                activity_until: u(&p["until"]),
                character: ((id - 1) % 8) as u32,
                name: format!("Resident {id}").into_bytes(),
            });
            s.agents.push(Agent {
                id: id + players.len() as u64,
                player: id,
                owner: AGENT_BASE + id,
                to_remember: 0,
                last_conversation: 0,
                last_invite: START,
                op: OP_NONE,
                op_since: 0,
                op_arg: 0,
                character: ((id - 1) % 8) as u32,
                name: format!("Resident {id}").into_bytes(),
                identity: b"A resident of the town.".to_vec(),
                plan: b"Walk around, enjoy an activity, and talk with neighbors.".to_vec(),
            });
            for (j, node) in path.iter().enumerate().filter(|_| moving) {
                let next = &path[(j + 1).min(path.len() - 1)];
                let prev = &path[j.saturating_sub(1)];
                let nx = u(&next[0]) as i64;
                let ny = u(&next[1]) as i64;
                let cx = u(&node[0]) as i64;
                let cy = u(&node[1]) as i64;
                let (fx, fy) = if j + 1 < path.len() {
                    ((nx - cx).signum(), (ny - cy).signum())
                } else {
                    (
                        (cx - u(&prev[0]) as i64).signum(),
                        (cy - u(&prev[1]) as i64).signum(),
                    )
                };
                s.paths.push(PathNode {
                    player: id,
                    seq: j as u64,
                    x: cx as u32,
                    y: cy as u32,
                    face_dx: (1000 + fx * 1000) as u32,
                    face_dy: (1000 + fy * 1000) as u32,
                    t: START + j as u64 * 4000 / 3,
                });
            }
        }
        for c in f["conversations"].as_array().unwrap() {
            let id = u(&c["id"]);
            let a = u(&c["a"]);
            let b = u(&c["b"]);
            let status = u(&c["status"]) as u32;
            let msgs = u(&c["messages"]);
            s.conversations.push(Conversation {
                id,
                creator: a,
                created: START - 5000,
                typing_player: 0,
                typing_since: 0,
                last_author: if msgs > 0 { b } else { 0 },
                last_at: if msgs > 0 { START - 3000 } else { 0 },
                num_messages: msgs,
            });
            s.members.push(Member {
                conversation: id,
                player: a,
                invited: START - 1000,
                status: if status == ST_TALKING {
                    ST_TALKING
                } else {
                    ST_WALKING
                },
                started: if status == ST_TALKING {
                    START - 5000
                } else {
                    0
                },
            });
            s.members.push(Member {
                conversation: id,
                player: b,
                invited: START - 1000,
                status,
                started: if status == ST_TALKING {
                    START - 5000
                } else {
                    0
                },
            });
        }
        assert!(placement_ok(&s), "fixture has invalid placements");
        Self {
            state: s,
            fixture: f,
            metrics: Metrics::default(),
            command_ms: None,
            history_pack_ms: 0.0,
            history_bytes: 0,
        }
    }
    fn command(&mut self, owner: u64, cmd: Command) -> bool {
        let category = match cmd { Command::MoveTo { .. } => 0, Command::StartConversation { .. } => 1, _ => 2 };
        let start = self.command_ms.map(|_| Instant::now());
        let result = transition(
            &Principal {
                org: 1,
                user: owner,
                now_ms: self.state.meta.now_ms,
            },
            &self.state,
            &cmd,
        );
        if let (Some(start), Some(times)) = (start, &mut self.command_ms) {
            times[category] += start.elapsed().as_secs_f64() * 1000.;
        }
        match result {
            Ok((writes, _)) => {
                let start = self.command_ms.map(|_| Instant::now());
                apply_mut(&mut self.state, writes);
                if let (Some(start), Some(times)) = (start, &mut self.command_ms) {
                    times[category + 3] += start.elapsed().as_secs_f64() * 1000.;
                }
                true
            }
            Err(_) => false,
        }
    }
    fn destination(&self, id: u64) -> (u32, u32) {
        let s = &self.state;
        let p = &s.players[id as usize - 1];
        let salt = hash((id + u(&self.fixture["seed"]) + (s.meta.now_ms - START) / 15000) as u32);
        for k in 0..512 {
            let h = hash(salt.wrapping_add(k));
            let x = p.x as i64 / 1000 + (h % 25) as i64 - 12;
            let y = p.y as i64 / 1000 + ((h >> 8) % 25) as i64 - 12;
            if x >= 0
                && y >= 0
                && x < s.meta.width as i64
                && y < s.meta.height as i64
                // A wander should cover real distance. This also avoids the
                // original engine's zero-duration path at floating-point epsilon.
                && ((x * 1000 - p.x as i64).pow(2) + (y * 1000 - p.y as i64).pow(2)
                    >= 500 * 500)
                && !mask_blocked(
                    &s.terrain.mask,
                    s.meta.width,
                    s.meta.height,
                    x as u32 * 1000,
                    y as u32 * 1000,
                )
            {
                return (x as u32, y as u32);
            }
        }
        (p.x / 1000, p.y / 1000)
    }
    pub fn drain(&mut self) {
        // Use the same authorized transitions and write interpreter as the
        // server. This in-memory owner can commit without cloning the world.
        let pending: Vec<_> = self
            .state
            .agents
            .iter()
            .filter(|a| a.op != OP_NONE)
            .map(|a| (a.player, a.owner, a.op))
            .collect();
        for (id, owner, op) in pending {
            if op == OP_DO {
                self.metrics.decisions += 1;
                let salt = hash(
                    (id + u(&self.fixture["seed"]) + (self.state.meta.now_ms - START) / 15000)
                        as u32,
                );
                let mut invited = false;
                if salt % 4 == 0 {
                    let me = &self.state.players[id as usize - 1];
                    let target = self
                        .state
                        .players
                        .iter()
                        .find(|p| {
                            p.id != id
                                && dist_sq(me.x, me.y, p.x, p.y) < 144_000_000
                                && !self.state.members.iter().any(|m| m.player == p.id)
                        })
                        .map(|p| p.id);
                    if let Some(invitee) = target {
                        if self.command(owner, Command::StartConversation { invitee }) {
                            self.metrics.invitations += 1;
                            invited = true;
                        }
                    }
                }
                if !invited {
                    if salt % 4 == 1 {
                        if self.command(owner, Command::Activity { kind: 1 + salt % 3 }) {
                            self.metrics.activities += 1;
                        }
                    } else {
                        let (x, y) = self.destination(id);
                        if self.command(owner, Command::MoveTo { x, y }) {
                            self.metrics.destinations += 1;
                            self.metrics.pathfinds += 1;
                        }
                    }
                }
            } else if op == OP_SPEAK {
                let text = format!(
                    "{} sounds nice. See you around town!",
                    ["reading a book", "gardening", "daydreaming"][id as usize % 3]
                );
                if self.command(
                    owner,
                    Command::Say {
                        text: text.into_bytes(),
                    },
                ) {
                    self.metrics.messages += 1;
                }
            } else if op == OP_REMEMBER {
                if self.command(
                    owner,
                    Command::Remember {
                        summary: b"Talked with a neighbor about life in town.".to_vec(),
                    },
                ) {
                    self.metrics.memories += 1;
                }
            }
            self.command(owner, Command::FinishOp);
        }
    }
    pub fn step(&mut self, ticks: u64) -> f64 {
        let clock = Instant::now();
        let end = self.state.meta.now_ms + ticks * 16;
        let (_, _, capture) = run_ticks_captured(&mut self.state, end, false);
        self.metrics.pathfinds += self.state.meta.pathfinds as u64;
        self.drain();
        let tick_ms = clock.elapsed().as_secs_f64() * 1000.0;
        let pack_clock = Instant::now();
        self.state.motion_history = capture.finish(self.state.meta.now_ms);
        self.history_pack_ms += pack_clock.elapsed().as_secs_f64() * 1000.0;
        self.history_bytes += self.state.motion_history.data.len();
        tick_ms
    }
    pub fn snapshot(&self, tick_ms: f64, step_ms: u64) -> String {
        let mut v = ai_town_server::to_value_with_options(&self.state, false, 100);
        // Both engines show authoritative positions. Never animate a planned
        // future path while an overloaded engine is still computing the tick.
        v["demo"] = json!({"engine":"rust","scenario":"town","tickMs":tick_ms,"stepMs":step_ms,
            "metrics":self.metrics.json(),"density":self.fixture["density"],"districts":self.fixture["districts"]});
        v.to_string()
    }
}
