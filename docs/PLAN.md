# AI Town on h5i-app

This port replaces the Convex backend of [a16z-infra/ai-town](https://github.com/a16z-infra/ai-town)
with a Rust kernel, an h5i-app shell, and an async agent worker. The React and
PixiJS client stays. It talks to the shell over HTTP and a WebSocket instead of
Convex hooks.

The original layout is the guide (`ARCHITECTURE.md` in the reference tree):

| Original | Role |
|---|---|
| `convex/aiTown` | Game rules: worlds, players, movement, conversations, inputs |
| `convex/engine` | Load, step, save, input queue, historical positions |
| `convex/agent` | LLM conversations, memories, embeddings |
| `convex/util` | Geometry, heap, compression |
| `src/` | PixiJS client |

## Where each module goes

h5i-app splits a program into a pure kernel
(`transition(actor, snapshot, command) -> (writes, reply)`), a shell that only
loads and stores rows, and anything asynchronous beside that. Time enters
through `Kernel::stamp`, never through a client field.

| Convex module | Rust home | Why |
|---|---|---|
| `aiTown/world.ts`, `ids.ts`, `schema.ts` | `kernel` tables `Meta`, `Player`, `Agent` | The world is one tenant snapshot |
| `aiTown/worldMap.ts` | `kernel` `Terrain` (walk mask) plus `assets/gentle.json` served by the shell | Collision is a rule. Art is not |
| `aiTown/player.ts`, `location.ts` | `kernel` `Player` and `tick_player` | Position changes only inside `transition` |
| `aiTown/movement.ts`, `util/geometry.ts`, `util/minheap.ts` | `kernel` `geom` and `movement` | Pathfinding is a pure function of the mask and the other players |
| `aiTown/conversation.ts`, `conversationMembership.ts` | `kernel` `Conversation`, `Member` | Exclusivity and membership are kernel invariants |
| `aiTown/inputs.ts`, `inputHandler.ts`, `insertInput.ts` | `kernel` `Command` | A player's command is one transition. The actor comes from the token |
| `aiTown/agent.ts` (the `tick` rules) | `kernel` `tick_agent` | Timeouts, invite acceptance, walking toward someone, and "needs an operation" are rules |
| `aiTown/agent.ts` operations, `agentInputs.ts`, `agentOperations.ts` | `server` agent worker | They wait on the network, then submit ordinary commands |
| `agent/conversation.ts`, `memory.ts`, `embeddingsCache.ts`, `util/llm.ts` | `server` provider (`Nim` or `Stub`) and `kernel` `Memory` rows | The prompt and the HTTP call are not decisions about who may move. The stored summary is a row the kernel accepts |
| `engine/abstractGame.ts` | `server` sim loop | One `Step` command per second runs 60 ticks of 16 ms. The engine user is the only caller. Postgres commits the write set once |
| `engine/historicalObject.ts` | client interpolation along `PathNode` timestamps | The path already carries arrival times, so the client can replay the last second without a packed history buffer |
| `engine/schema.ts` inputs and generation numbers | shell idempotency keys and a single sim task | h5i-app already serializes one tenant. A second step cannot overlap |
| `messages.ts` | `kernel` `Message` | Posting checks membership. Text is not a game-rule by itself, but the check is |
| `music.ts`, `world.ts` queries, `init.ts` | shell routes and startup | Map JSON, session tokens, seeding agents |
| `src/` | `frontend/` | Same Pixi scene. `frontend/src/api.ts` replaces `useQuery` and `useMutation` |

The kernel crate depends on `h5i-app-schema` and `h5i-app-sql` 0.1.2 only, so
extraction never sees tokio or axum. The server depends on
`h5i-app = { version = "0.1.2", features = ["http", "postgres"] }`.

## Simulation

Constants match the original: tick 16 ms, step 1 s, speed 0.75 tiles per
second, collision radius 0.75 tiles, conversation distance 1.3 tiles, at most
16 path searches per step, at most 8 humans.

Positions are milli-tiles (`u32`). A tile is 1000 units. Comparisons use
squared distance, so the kernel stays in the integer subset Aeneas translates.
The client divides by 1000 when it draws.

Player commands (`Join`, `Leave`, `MoveTo`, `Stop`, conversation commands,
`Say`) commit as soon as they arrive. `Step`, sent by the shell once a second
as user 0, advances movement, conversation range, and agent rules. That is the
original batching: many ticks, one write. Inputs are not held until the next
step, so a click does not wait for the step boundary. Movement along a path
still integrates on the ticks.

A server built with the `gpu` feature, and started with `AI_TOWN_MOVE=gpu`,
replaces that `Step` with sixty rounds of the two-phase rule. Each round
proposes from the positions of the previous round, then accepts a candidate
only when no better-priority proposal is inside the collision radius. One
device thread handles one agent. The device rebuilds the spatial grid between
the phases and keeps the buffers for the whole batch. A refused walker on
the CPU enters `PF_WAIT` for about a second. The GPU batch keeps the moving
flags from the start of the batch and retries the later path pose. The pass
writes positions with `Command::Relocate`, which sets x, y, and speed 0 and
clears every path, and refuses the whole batch with `Occupied` unless every
player then stands on a walkable tile with nobody else inside the collision
radius (`placement_ok`). The loop then falls back to the CPU step for that
second. Face and the path flag stay as they were. Conversations,
invites, agent operations, and `meta.now_ms` advance when the loop runs
`Command::Step`. The default build runs the same two-phase round on the CPU.
`AI_TOWN_TICK=seq` selects `Command::StepSeq`, the old index-order rule, and
only the engine can send it. Clients cannot send `Relocate`.

`Step` is refused for every other user. The shell's authenticator is the only
code that can mint the user-0 token, and it never sends that token to a browser.

## Invariants

The Lean proofs are postconditions of the kernel functions below. `h5i app check`
is the gate that accepts them. Each row names the mutant in `h5i-app.toml` that
the postcondition is written to reject. `h5i app mutate` checks that
rejection: the unmodified copy builds, and each mutant makes `lake build` of
`proofs/` fail. The current four mutants are caught.

| Theorem | Statement | Mutant |
|---|---|---|
| `exclusive_preserved` | `dispatch_start` returns `Busy` when the caller is already a member, the invitee exists, and the two player ids differ | `second_conversation` changes the member check from or to and |
| `members_consistent_preserved` | A successful `dispatch_start` from empty member and conversation lists returns one conversation. Its members are the caller (`walkingOver`) and the invitee (`invited`). Its creator is the caller | `one_member_ok` drops the invitee push |
| `collision_safe_preserved` | `occupied` is true when the terrain mask is clear and another player is strictly inside the collision radius (562500 milli-tiles squared) | `overlap_allowed` flips `<` to `>` in `occupied` |
| `motion_isolated_for_player` | The id `own_player` returns belongs to the queried owner | `move_anyone` returns `players[0]` |

Refusals commit nothing, which is the engine's contract. `apply` is what a
later read returns. `h5i app prove` extracts the kernel and checks the proofs.
After the two-phase round moved into `step::propose_one`, `step::resolve_one`,
and `step::tick_round`, `h5i app check` accepted the regenerated extraction.
The axiom gate checked 61 theorems in `Theorems` and named only `propext`,
`Classical.choice`, and `Quot.sound`. `rules::occupied` stays in the extraction
start list so `collision_safe_preserved` still names that comparison. The
generated Lean has no `sorry` and no `axiom`.

`step_never_blocked` refuses a candidate whose tile the mask reports blocked.
The agent keeps its position and speed 0. `step_never_overlap` refuses a
candidate the spatial grid reports as occupied. `step_clear_moves` accepts the
candidate when the agent is moving, the mask is clear, and the grid reports
nobody else inside the radius. Those three take a successful `mask_blocked`,
and the last two a successful `grid_hit`, as hypotheses. `grid_hit` adds
coordinates with a fallible `+`, so the theorems apply when that call returns.
`tick_players_preserves_exclusive`, `tick_players_preserves_members`, and
`tick_players_preserves_own_player` say a batched pass that returns (`MovesOk`:
`tick_moves` succeeds and keeps the slice lengths) keeps the three invariants
above. The roster lemmas under them show the pass keeps conversation lists,
member lists, player ids, and owners. The four mutants in the table are unchanged.

`propose_never_blocked`, `propose_stays_on_hit`, and `propose_clear` are the
same three facts for a proposal. `resolve_stays`, `resolve_yields`, and
`resolve_accepts` say a rejected proposal stays, a proposal that `grid_yield`
reports stays, and a proposal that `grid_yield` clears is accepted. Those take
a successful `mask_blocked`, `grid_hit`, or `grid_yield` as a hypothesis.
`grid_hit` and `grid_yield` are not claimed to be total: the generated loops
use a fallible `+` and a slice index. `within_radius_neighbor_tile` says a
distance under 562500 milli-tiles squared puts the other agent on the same
tile or a neighbor tile, which is why a 3 by 3 neighborhood covers the radius
when the cell is one tile and the radius is 0.75. `two_phase_apart` is the
round on plain functions. Given positions already outside the radius, a
proposal that is clear of every previous position and unblocked, and an
acceptance that yields to every better-priority proposal inside the radius,
the resulting positions stay outside the radius and an accepted agent was not
blocked. A rejected agent keeps the previous position. The theorem does not
unfold `grid_hit` or `grid_yield` into that scan. `tick_players_round_preserves_exclusive`,
`tick_players_round_preserves_members`, and `tick_players_round_preserves_own_player`
say a round that returns under `RoundOk` (the call succeeds and the coordinate
slices keep the player count) keeps the three invariants. The cell-count
hypothesis is the map width times the height, which fits for the maps this
kernel builds.

## Agents and the model

`tick_agent` never calls the network. It accepts or rejects invites, walks
toward a partner, leaves on timeout, and sets `Agent.op` when it needs a
destination, a line of dialogue, or a memory.

The worker in `server` owns those operations. It reads `NVIDIA_API_KEY` only
when constructing the provider. An empty or missing key selects the
deterministic stub, which is what the tests run. The key is not logged, not
written into a reply, and not stored in the database. Prompt text lives in
code next to the worker, not in a committed instruction file.

Memories are the last summaries for a pair of players. The original embedding
index is not part of this port. The stub records a fixed summary. NIM, when
configured, writes the model's summary through the same `Remember` command.

The default model is `google/diffusiongemma-26b-a4b-it`. `NVIDIA_MODEL` overrides it. `meta/llama-3.1-8b-instruct` reached end of life on 2026-08-26, and the API returns HTTP 410. A non-success status, including 429, and a transport timeout store the stub line for that turn. A live call against the retired model logged that failure at 35 ms and 11 ms, and the stored lines were the stub. With the current model, Lucky and Bob exchanged five lines. Those replies took 994 ms, 1,991 ms, 1,000 ms, 819 ms, and 551 ms.

A later headless Chromium session joined the Vite page as Ada and selected Lucky. The sidebar bubble was `Lucky: You won't believe the incredible sights I saw on that distant planet!` Successful replies in that session took 1,830 ms, 2,401 ms, 5,040 ms, 1,753 ms, 1,219 ms, 1,730 ms, 2,565 ms, 1,129 ms, 1,299 ms, 1,933 ms, 6,541 ms, 1,368 ms, 995 ms, 3,754 ms, 2,402 ms, 740 ms, 1,318 ms, 1,625 ms, and 2,726 ms. Memory summaries took 6,702 ms, 1,035 ms, 954 ms, and 2,860 ms. Six calls returned HTTP 400, and one call hit the 20 s timeout; those turns stored the stub line. The bubble on screen was a successful NIM reply. Screenshots from that session are under `/dev/shm/ai-town-rust` (`town.png`, `town-joined.png`, `town-zoomed.png`, `town-conversation.png`).

## Shell

PostgreSQL runs from `docker-compose.yml`. Its data directory is
`/dev/shm/ai-town-rust/pgdata`, not a path in the repo. One schema, `ai_town`,
holds the kernel tables. The sim loop and the agent worker call
`Engine::execute` on that schema. Browsers use `POST /rpc` and `GET /ws`.

## Frontend

`frontend/` is the PixiJS town. `useTown` and `rpc` in `frontend/src/api.ts`
replace the Convex hooks. Positions in a snapshot are milli-tiles; the canvas
divides by 1000 and interpolates `PathNode` timestamps. The map file is the
original gentle tileset, converted to JSON so both the shell (collision mask)
and the canvas (tiles) share it.

A headless Chromium session opened `http://127.0.0.1:5173/ai-town/`. The canvas
rendered, the join form accepted the name Ada, and the sidebar showed Lucky's
NIM line quoted in the agents section.

## Benchmarks and the GPU

See [BENCHMARKS.md](BENCHMARKS.md) for current measurements against the
original AI Town, CPU and GPU timings with placement verification, server
latency, raw samples, and reproduction commands. These replace the older
measurements made before the per-tick CPU placement check.

`gpu/` is an optional package outside the workspace. `gpu/nvvm` compiles
`kernel`'s `propose_one` and `resolve_one` to PTX with `rustc_codegen_nvvm`.
The `round_phase` entry inserts positions, proposes, inserts proposals, and
resolves, with one thread per player. Atomic insertion can change grid chain
order; differential tests compare the resulting coordinates. `step_batch`
and the hand-written `seek.cu` remain baselines.

## Trusted boundary

The kernel does not see the clock, the network, or Postgres. Trusted pieces,
matching the h5i-app tutorials:

- the authenticator puts the user id and the attempt time on the principal
- user 0 is the sim loop, not a browser
- PostgreSQL commits the write set or nothing
- the agent worker sends commands as the agent user the kernel assigned
- the model, when enabled, only supplies text and a chosen tile. The kernel still accepts or refuses the command
- the optional GPU thread loads the PTX image of the two-phase round (`round_phase`) and submits `Relocate` as user 0. Placement safety does not require correct GPU, `rustc_codegen_nvvm`, NVVM, or driver execution: `relocate_placed` proves that an accepted `Relocate` is `Placed` (every player on a walkable tile, every player with another id at least the collision radius away), whatever positions it was sent. This does not prove that the GPU follows CPU movement semantics. The check is linear: `bucket` files each player into four slots of its own tile (a fifth player in one tile is refused, which is always safe and never changes the answer, since five points in one tile always include a pair closer than 750), and `near_any` reads the 36 slots of the 3 by 3 tiles around each player. `placement_ok_sound` proves it from `bucket_spec`, `near_any_spec`, and `within_radius_neighbor_tile`. The all-pairs `placement_ok_scan` remains the test oracle.
- the movement grid answers. `step_never_overlap`, `propose_*`, and `resolve_*` take the result of `grid_hit` / `grid_yield` as a hypothesis; no theorem yet says that a grid built by `grid_build` (or accepted by `grid_wellformed`) makes `grid_hit` false only when nobody is inside the radius. It no longer decides what is stored. In GPU mode `Relocate` re-checks the result with the proven `placement_ok`. On the CPU every tick ends with `settle`: the movement pass is kept only when `placement_ok` accepts it, and otherwise every player goes back to its position before the pass, stopped, while the clock, conversations, and agents still advance. `settle_placed` proves the kept positions are `Placed` whenever they were before the pass. `Reply::Stepped` reports `reverted`, the number of ticks put back; it is expected to stay 0. The proof does not cover the rest of the tick (that the movement pass keeps player ids and order, or that joining and leaving keep `Placed`); those are tested. `settle` costs one `placement_ok` per tick; current measurements are in [BENCHMARKS.md](BENCHMARKS.md). In-memory fixtures may use wider maps (`PLACE_MAX_DIM` is 1,024); `Init` still caps worlds at 128 tiles per side. Tests check it against the full scan under random chain orders. The PTX image builds its grid with atomic pushes through the kernel's `cell_of`; with `AI_TOWN_GPU_VERIFY=1` every device grid is copied back and must pass `grid_wellformed`, or that second falls back to the CPU step
