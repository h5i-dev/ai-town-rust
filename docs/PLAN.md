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
the phases and keeps the buffers for the whole minute. A refused walker on
the CPU enters `PF_WAIT` for about a second. The GPU minute keeps the moving
flags from the start of the minute and retries the later path pose. The pass
writes positions with `Command::Relocate`, which sets x, y, and speed 0 and
clears every path. Face and the path flag stay as they were. Conversations,
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

`ai-town-bench` writes under `/dev/shm/ai-town-rust/bench/`:

- in-memory `Step` rate for 1, 8, 16, 32, 64, 128, 512, 1024, 4096, and 10000 agents (`ticks.csv`)
- greedy batch seeks for 128, 1024, and 4096 agents, checked against the scalar scan (`batch.csv`)
- HTTP `move_to` latency for 5, 16, and 32 agents on `127.0.0.1:18080` and database `ai_town_bench`, with the stub provider (`latency.csv`)

The map side starts at 48 and doubles until it can hold the agents, capped at 128. Each agent walks to its own tile. One `Step` is 60 ticks of 16 ms, so real time is about 63 ticks/s.

A tick used to scan every path and every player, and pathfinding asked every neighbor whether it overlapped every other player. The kernel now keeps a one-tile spatial grid (a collision check reads the 3 by 3 neighborhood; the largest step plus the collision radius stays under one tile), an index of each player's path span, a sorted diff, and, on the tick, a binary search for a player id. A miss scans. The membership proofs still unfold the full scan.

Paired in-memory probe, two measured steps after one warmup step, spread destinations, release build. The before column is the kernel from the tick-benchmark commit. At 4096 and 10000 that kernel was still placing agents when the 300 s setup limit fired (3326 and 3346 placed).

| Agents | Before ticks/s | After ticks/s | After step |
|---:|---:|---:|---:|
| 16 | 216,597 | 259,709 | 0.23 ms |
| 32 | 80,906 | 154,894 | 0.39 ms |
| 64 | 30,069 | 84,701 | 0.71 ms |
| 128 | 9,314 | 38,425 | 1.56 ms |
| 512 | 770 | 5,501 | 10.91 ms |
| 1,024 | 203 | 2,747 | 21.84 ms |
| 4,096 | setup stopped at 300 s | 654 | 91.80 ms |
| 10,000 | setup stopped at 300 s | 260 | 230.78 ms |

An earlier 4-step run with every agent walking to the bottom row, on that same committed kernel, measured 5,604,858 / 473,107 / 203,943 / 40,147 / 21,246 ticks/s at 1 / 8 / 16 / 32 / 64 agents. `ai-town-bench` now uses the spread placement, 4 steps, and the longer size list. On this kernel it measured 399,080 / 185,196 / 116,873 / 48,418 / 37,875 / 23,654 / 4,026 / 2,218 / 457 / 315 ticks/s at 1 / 8 / 16 / 32 / 64 / 128 / 512 / 1,024 / 4,096 / 10,000 agents. Those are the published tick rates.

`CreateAgent` now asks the same one-tile grid the tick uses. On an idle machine, placing 10,000 agents took 22.3 s and giving each a path took 49.6 s, 71.9 s together. The step after the warmup was 193 ms (310 ticks/s). The earlier combined setup, still scanning every player for every candidate tile, was about 87 s. Each create still builds a fresh grid, so the placement total stays quadratic, and pathfinding is the larger share.

`h5i-app-gpu` is not a crate on crates.io, and this tree does not depend on it. The h5i GPU note describes an experimental search and push backend outside extraction. The movement decision on the device is `kernel`'s `propose_one` and `resolve_one`, the same Rust functions `tick_round` calls on the CPU. The device entry is `round_phase`. Each phase is one thread per agent. Phase 0 inserts the previous positions, phase 1 proposes, phase 2 inserts the proposals, and phase 3 resolves. The grid insert is an atomic exchange onto the cell head, so the chain membership matches `grid_build` and the chain order may differ. The differential test compares final coordinates. `step_batch` remains the one-thread `tick_moves` baseline. `seek_one` remains the baseline entry `seek_batch`, and the hand-written `seek.cu` cubin stays beside it. `gpu/nvvm` compiles that source with `rustc_codegen_nvvm` from rust-cuda `3a82c35a7196731fb60eebdac2a40d2b1bf0ba95` on nightly-2026-04-02, target `nvptx64-nvidia-cuda`, arch `compute_86`. The toolchain, the LLVM download, and the build directory are `/dev/shm/ai-town-rust`. Arena packing follows the h5i GPU layout. `cargo test --manifest-path gpu/Cargo.toml` checks that packing and, when the PTX image is present, checks `round_phase` against host `tick_round` and `step_batch` against host `tick_moves`. The launch, the copies, the path pose, and the `Relocate` write-back sit outside the proof.

`seek.cu` follows `scalar_seek`: move on x, then on y, then reject the whole step if the tile is blocked or another agent, at its original position, is inside the collision radius. The release cubin bench on this machine (sm_86) measured one seek at 1,000 / 10,000 / 50,000 agents. The CPU column here is that scalar scan:

| Agents | Scalar scan | Cubin |
|---:|---:|---:|
| 1,000 | 1.54 ms (650,328/s) | 6.66 ms (150,085/s) |
| 10,000 | 152.25 ms (65,682/s) | 12.82 ms (779,935/s) |
| 50,000 | 3,980 ms (12,562/s) | 16.67 ms (2,999,238/s) |

The coordinates matched at each size. At 1,000 agents the launch overhead is larger than the scan. From 10,000 up the cubin is faster than the scalar scan.

An earlier fair bench timed `seek_one`, the greedy scan of every agent, for 60 ticks. Those rows are the previous greedy path:

| Agents | CPU `run_ticks` | CPU `Step` | GPU 60 kernel | GPU 60 wall |
|---:|---:|---:|---:|---:|
| 1,000 | 9.06 ms | 9.26 ms | 7.43 ms | 157.36 ms |
| 10,000 | 97.35 ms | 99.46 ms | 72.36 ms | 222.49 ms |
| 50,000 | 507.47 ms | 515.69 ms | 885.41 ms | 1,022.23 ms |

One launch of that scan, after a warmup, took 0.13 ms, 1.25 ms, and 15.11 ms of device time (wall 13.87 ms, 14.93 ms, and 19.56 ms).

An earlier fair bench timed `tick_moves` on one device thread against the product CPU step. The CPU columns are `run_ticks` and `transition(Command::Step)`. The GPU columns rebuild the grid on the host and launch that one thread. Every agent walks east on an open mask. Positions matched. The one-thread kernel is slower than the CPU spatial tick at each size. GPU 0 was in use by another process, so these times are an upper bound for an idle card.

| Agents | CPU `run_ticks` | CPU `Step` | GPU one kernel | GPU one wall | GPU 60 kernel | GPU 60 wall |
|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 9.15 ms | 9.41 ms | 4.45 ms | 23.31 ms | 126.37 ms | 1,228.94 ms |
| 10,000 | 94.99 ms | 98.11 ms | 42.14 ms | 58.14 ms | 2,588.81 ms | 3,632.88 ms |
| 50,000 | 481.09 ms | 493.78 ms | 243.17 ms | 251.44 ms | 14,442.59 ms | 14,988.62 ms |

The current fair bench times the two-phase round. `cpu_tick_ms` is the product `run_ticks`. `cpu_round_ms` is 60 calls of `tick_round` on one thread, with candidates prepared before the timer. `cpu_parallel_ms` is those rounds on a pool of 8 threads, started once, with the grid build on the main thread. The machine reported 128 hardware threads. The GPU columns are `RustDevice::rounds`: one copy in, the minute resident on the device, one copy out. Kernel time is the CUDA events around the memsets and launches. Every agent walks east on an open mask, so the rules agree. Agent 0 moved, and the single-thread round, the pool, the GPU, and `run_ticks` landed on the same coordinates.

| Agents | CPU tick | CPU round | CPU 8 threads | GPU one kernel | GPU one wall | GPU 60 kernel | GPU 60 wall |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 10.518 ms | 4.442 ms | 7.427 ms | 0.025 ms | 18.314 ms | 1.297 ms | 12.631 ms |
| 10,000 | 110.665 ms | 47.452 ms | 49.898 ms | 0.030 ms | 15.958 ms | 1.553 ms | 9.339 ms |
| 50,000 | 570.884 ms | 241.943 ms | 176.000 ms | 0.038 ms | 1.820 ms | 2.011 ms | 8.286 ms |
| 100,000 | 1,202.314 ms | 481.704 ms | 312.842 ms | 0.059 ms | 1.713 ms | 5.752 ms | 15.447 ms |

The 8-thread pool takes 7.427 ms at 1,000 agents and 49.898 ms at 10,000, where one thread takes 4.442 ms and 47.452 ms. It takes 176.000 ms at 50,000 and 312.842 ms at 100,000, where one thread takes 241.943 ms and 481.704 ms. The 60-round GPU kernel takes 1.297 ms, 1.553 ms, 2.011 ms, and 5.752 ms. The one-round wall of 18.314 ms at 1,000 agents pays for device allocation. The run used GPU 2. A snapshot at 2026-10-21T06:43:58-04:00 showed all four RTX 3090s busy (93%, 95%, 100%, 100% utilization and 21,440, 21,228, 22,048, 23,636 MiB of 24,576 MiB). GPU 2 was at 100% and 22,048 MiB, so these times are an upper bound for an idle card. Other processes were left running.

The sim loop can run that PTX path. Build `ai-town-server` with `--features gpu` and set `AI_TOWN_MOVE=gpu`. A dedicated thread owns the CUDA context. Each second it runs 60 two-phase rounds for the players whose path flag is already moving, leaves everyone else in place as an obstacle, and commits `Command::Relocate`. Candidates for the minute are copied once. A refused CPU walker enters `PF_WAIT`. The GPU minute retries the later path pose. Its last sample is the wall clock when `meta.now_ms` is more than 960 ms behind that clock, because paths are stamped with the command clock. `Relocate` clears every path, and the sim tick and RPC share one lock for the whole step, so a path committed during the device minute is still present when `Relocate` runs. A failed open or a failed tick logs `gpu step failed` and the loop falls back to `Command::Step` for that second. With the feature off, `AI_TOWN_MOVE=gpu` logs `gpu step unavailable` and keeps the CPU step. The CPU step is the two-phase round unless `AI_TOWN_TICK=seq`.

End-to-end latency is `bench`'s `pipe` binary on this two-phase round: `POST /rpc` `move_to`, then the batched tick, then the websocket snapshot whose x or y changed. The run uses database `ai_town_pipe` on `127.0.0.1:18081` and the stub provider. `AI_TOWN_DRAIN=off` leaves the agent worker idle so the crowd is not also pathfinding. The world is a 128 by 128 open mask, the largest `Init` accepts (`MAX_DIM` is 128, 16,384 tiles). The gentle map is 64 by 48. A 50,000-agent HTTP world needs a wider map than `Init` allows, so that size stays in the in-memory fair bench. The joined player is a session id whose spawn tile has a free orthogonal step, and `move_to` targets that neighbor. A one-tile pocket can have a collision-legal step that leads away from a far destination, and `find_route` then returns no path while `begin_move` still returns success. After the position moves, the bench sends `stop`. CPU and GPU rows are the same server build.

| Mode | Agents | HTTP `move_to` | Position on the websocket |
|---|---:|---:|---:|
| cpu | 1,000 | 45.848 ms | 1,164.234 ms |
| cpu | 10,000 | 478.995 ms | 15,293.189 ms |
| gpu | 1,000 | 41.644 ms | 263.860 ms |
| gpu | 10,000 | 359.155 ms | 1,173.561 ms |

The sim loop wakes once a second, so the push includes the wait for the next tick. At 10,000 agents the CPU push was 15,293.189 ms and the GPU push was 1,173.561 ms. The GPU rows used `round_phase` on GPU 2. A snapshot at 2026-10-21T07:37:30-04:00 showed that card idle at 0% utilization and 4 MiB of 24,576 MiB. The other three RTX 3090s were at 100%, 98%, and 100% utilization, with 20,508, 21,420, and 21,982 MiB in use. Other processes were left running.

An earlier pipe measured the greedy `seek_one` path. The `tick_moves` pipe was left unmeasured. Those GPU rows used `CUDA_VISIBLE_DEVICES=0`:

| Mode | Agents | HTTP `move_to` | Position on the websocket |
|---|---:|---:|---:|
| cpu | 1,000 | 36.7 ms | 1,323 ms |
| cpu | 10,000 | 262.5 ms | 14,876 ms |
| gpu | 1,000 | 43.0 ms | 577 ms |
| gpu | 10,000 | 229.9 ms | 1,863 ms |

At 10,000 agents on that path the CPU push was 14.9 s and the GPU push was 1.86 s.

## Trusted boundary

The kernel does not see the clock, the network, or Postgres. Trusted pieces,
matching the h5i-app tutorials:

- the authenticator puts the user id and the attempt time on the principal
- user 0 is the sim loop, not a browser
- PostgreSQL commits the write set or nothing
- the agent worker sends commands as the agent user the kernel assigned
- the model, when enabled, only supplies text and a chosen tile. The kernel still accepts or refuses the command
- the optional GPU thread loads the PTX image of the two-phase round (`round_phase`) and submits `Relocate` as user 0
