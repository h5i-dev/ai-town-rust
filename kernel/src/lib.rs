//! AI Town kernel. Every accepted command is a pure function of the caller,
//! the tenant snapshot, and the command. The server stores the write set.
//!
//! Positions are milli-tiles. Time is milliseconds on the principal, stamped
//! by the shell. User 0 is the simulation, not a browser.

mod batch;
mod geom;
mod movement;
mod rules;
mod step;

pub use batch::{batch_seek, scalar_seek};
pub use geom::{dist_sq, grid_build, grid_hit, grid_yield, isqrt, manhattan, mask_blocked, near_index, path_pose, set_blocked, MinHeap, NearIndex, Pose};
pub use rules::{collision_safe, diff, exclusive, members_consistent, motion_isolated, paths_isolated, run_ticks, run_ticks_seq, same_motion};
pub use step::{agent_step, propose_one, resolve_one, scan_hit, scan_yield, tick_moves, tick_players, tick_players_round, tick_round, Landing, Proposal};

/// One tile, in milli-tiles.
pub const MILLI: u32 = 1000;
/// 0.75 tiles per second.
pub const SPEED: u32 = 750;
pub const COLLISION_SQ: u64 = 750 * 750;
pub const TALK_SQ: u64 = 1300 * 1300;
pub const TICK_MS: u64 = 16;
pub const MAX_TICKS: u32 = 60;
pub const MAX_PATHFINDS: u32 = 16;
pub const PATH_TIMEOUT_MS: u64 = 60_000;
pub const BACKOFF_MS: u64 = 1000;
pub const MAX_HUMANS: usize = 8;
pub const IDLE_MS: u64 = 5 * 60 * 1000;
pub const TYPING_MS: u64 = 15_000;
pub const INVITE_MS: u64 = 60_000;
pub const CONV_COOLDOWN_MS: u64 = 15_000;
pub const AWKWARD_MS: u64 = 60_000;
pub const MAX_CONV_MS: u64 = 10 * 60 * 1000;
pub const MAX_MESSAGES: u64 = 8;
pub const MESSAGE_COOLDOWN_MS: u64 = 2_000;
pub const ACTION_TIMEOUT_MS: u64 = 120_000;
pub const ACTIVITY_MS: u64 = 60_000;
pub const FACE_BIAS: u32 = 1000;
pub const ENGINE: u64 = 0;
pub const AGENT_BASE: u64 = 1_000_000;
pub const MAX_DIM: u32 = 128;
pub const MAX_NAME: usize = 32;
pub const MAX_TEXT: usize = 280;
pub const MAX_PLAN: usize = 2_000;

pub const PF_NONE: u32 = 0;
pub const PF_NEED: u32 = 1;
pub const PF_WAIT: u32 = 2;
pub const PF_MOVE: u32 = 3;

pub const ST_INVITED: u32 = 1;
pub const ST_WALKING: u32 = 2;
pub const ST_TALKING: u32 = 3;

pub const OP_NONE: u32 = 0;
pub const OP_DO: u32 = 1;
pub const OP_REMEMBER: u32 = 2;
pub const OP_SPEAK: u32 = 3;

/// Who is calling. `user == 0` is the sim loop. `now_ms` is stamped by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Principal {
    pub org: u64,
    pub user: u64,
    pub now_ms: u64,
}

h5i_app_schema::schema! {
    mapping town_tables for ai_town_kernel, writes Write, lean "../proofs/generated/Schema.lean";

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Snapshot {
        meta: Meta,
        terrain: Terrain,
        players: Vec<Player>,
        paths: Vec<PathNode>,
        conversations: Vec<Conversation>,
        members: Vec<Member>,
        agents: Vec<Agent>,
        memories: Vec<Memory>,
        messages: Vec<Message>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Meta in "meta" {
        key {}
        next_id: u64,
        now_ms: u64,
        width: u32,
        height: u32,
        frozen: bool,
        pathfinds: u32,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Terrain in "terrain" {
        key {}
        mask: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Player in "players" {
        key { id: u64 }
        owner: u64,
        is_human: bool,
        x: u32,
        y: u32,
        face_dx: u32,
        face_dy: u32,
        speed: u32,
        dest_x: u32,
        dest_y: u32,
        pf: u32,
        pf_started: u64,
        wait_until: u64,
        last_input: u64,
        activity: u32,
        activity_until: u64,
        character: u32,
        name: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct PathNode in "path_nodes" {
        key { player: u64, seq: u64 }
        x: u32,
        y: u32,
        face_dx: u32,
        face_dy: u32,
        t: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Conversation in "conversations" {
        key { id: u64 }
        creator: u64,
        created: u64,
        typing_player: u64,
        typing_since: u64,
        last_author: u64,
        last_at: u64,
        num_messages: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Member in "members" {
        key { conversation: u64, player: u64 }
        invited: u64,
        status: u32,
        started: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Agent in "agents" {
        key { id: u64 }
        player: u64,
        owner: u64,
        to_remember: u64,
        last_conversation: u64,
        last_invite: u64,
        op: u32,
        op_since: u64,
        op_arg: u64,
        character: u32,
        name: Vec<u8>,
        identity: Vec<u8>,
        plan: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Memory in "memories" {
        key { id: u64 }
        agent: u64,
        other: u64,
        conversation: u64,
        summary: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Message in "messages" {
        key { id: u64 }
        conversation: u64,
        author: u64,
        ts: u64,
        text: Vec<u8>,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Init { width: u32, height: u32, mask: Vec<u8> },
    CreateAgent { name: Vec<u8>, character: u32, identity: Vec<u8>, plan: Vec<u8> },
    Join { name: Vec<u8>, character: u32 },
    Leave,
    MoveTo { x: u32, y: u32 },
    Stop,
    StartConversation { invitee: u64 },
    AcceptInvite,
    RejectInvite,
    LeaveConversation,
    StartTyping,
    Say { text: Vec<u8> },
    FinishOp,
    Remember { summary: Vec<u8> },
    Activity { kind: u32 },
    Step,
    /// Engine-only. The original sequential movement tick.
    StepSeq,
    /// Engine-only. Writes positions from a GPU seek and drops paths.
    /// `pf` and `dest_*` stay so the next seek still knows who is walking.
    Relocate { x: Vec<u32>, y: Vec<u32> },
    Freeze,
    Resume,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    PutMeta(Meta),
    PutTerrain(Terrain),
    PutPlayer(Player),
    DelPlayer(u64),
    PutPath(PathNode),
    ClearPath(u64),
    PutConv(Conversation),
    DelConv(u64),
    PutMember(Member),
    DelMember(u64, u64),
    PutAgent(Agent),
    DelAgent(u64),
    PutMemory(Memory),
    PutMessage(Message),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Ok,
    Player(u64),
    Conversation(u64),
    Stepped { now_ms: u64, ticks: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Forbidden,
    NotFound,
    BadText,
    Busy,
    Full,
    Occupied,
    Already,
    BadState,
    Uninit,
    Limit,
}

pub fn transition(actor: &Principal, snap: &Snapshot, cmd: &Command) -> Result<(Vec<Write>, Reply), Error> {
    rules::command(actor.user, actor.now_ms, snap, cmd)
}

fn put_player(v: &mut Vec<Player>, x: Player) {
    let mut lo = 0usize;
    let mut hi = v.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if v[mid].id < x.id {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < v.len() && v[lo].id == x.id {
        v[lo] = x;
    } else {
        Player::put(v, x);
    }
}

fn put_agent(v: &mut Vec<Agent>, x: Agent) {
    let mut lo = 0usize;
    let mut hi = v.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if v[mid].id < x.id {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < v.len() && v[lo].id == x.id {
        v[lo] = x;
    } else {
        Agent::put(v, x);
    }
}

fn apply_write(s: &mut Snapshot, w: Write) {
    match w {
        Write::PutMeta(m) => s.meta = m,
        Write::PutTerrain(t) => s.terrain = t,
        Write::PutPlayer(p) => put_player(&mut s.players, p),
        Write::DelPlayer(id) => s.players = Player::del(&s.players, id),
        Write::PutPath(n) => PathNode::put(&mut s.paths, n),
        Write::ClearPath(player) => {
            s.paths = PathNode::del_where(&s.paths, town_tables::PathNode::player, &h5i_app_sql::Column::to_val(&player));
        }
        Write::PutConv(c) => Conversation::put(&mut s.conversations, c),
        Write::DelConv(id) => s.conversations = Conversation::del(&s.conversations, id),
        Write::PutMember(m) => Member::put(&mut s.members, m),
        Write::DelMember(c, p) => s.members = Member::del(&s.members, c, p),
        Write::PutAgent(a) => put_agent(&mut s.agents, a),
        Write::DelAgent(id) => s.agents = Agent::del(&s.agents, id),
        Write::PutMemory(m) => Memory::put(&mut s.memories, m),
        Write::PutMessage(m) => Message::put(&mut s.messages, m),
    }
}

fn sql_write(w: &Write, out: &mut Vec<h5i_app_sql::Write>) {
    match w {
        Write::PutMeta(m) => out.push(m.sql_put()),
        Write::PutTerrain(t) => out.push(t.sql_put()),
        Write::PutPlayer(p) => out.push(p.sql_put()),
        Write::DelPlayer(id) => out.push(Player::sql_del(*id)),
        Write::PutPath(n) => out.push(n.sql_put()),
        Write::ClearPath(player) => {
            out.push(PathNode::sql_del_where(town_tables::PathNode::player, h5i_app_sql::Column::to_val(player)));
        }
        Write::PutConv(c) => out.push(c.sql_put()),
        Write::DelConv(id) => out.push(Conversation::sql_del(*id)),
        Write::PutMember(m) => out.push(m.sql_put()),
        Write::DelMember(c, p) => out.push(Member::sql_del(*c, *p)),
        Write::PutAgent(a) => out.push(a.sql_put()),
        Write::DelAgent(id) => out.push(Agent::sql_del(*id)),
        Write::PutMemory(m) => out.push(m.sql_put()),
        Write::PutMessage(m) => out.push(m.sql_put()),
    }
}

pub fn read_only(_cmd: &Command) -> bool {
    false
}

#[cfg(test)]
mod geom_f64;

#[cfg(test)]
mod tests;
