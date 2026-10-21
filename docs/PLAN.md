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
replaces that `Step` with sixty greedy seeks of `kernel`'s `seek_one` (12
milli-tiles each) and writes the positions with `Command::Relocate`.
Conversations, invites, agent operations, and `meta.now_ms` advance when the
loop runs `Command::Step`. The default build keeps the spatial-grid step.
Clients cannot send `Relocate`.

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
After placement moved onto the spatial grid, `h5i app check` accepted the
regenerated extraction: 26 theorems, and the axiom gate named only `propext`,
`Classical.choice`, and `Quot.sound`. `rules::occupied` stays in the extraction
start list so `collision_safe_preserved` still names that comparison. The
generated Lean has no `sorry` and no `axiom`.

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

`h5i-app-gpu` is not a crate on crates.io, and this tree does not depend on it. The h5i GPU note describes an experimental search and push backend outside extraction. The seek decision that runs on the device is `kernel`'s `seek_one`, the same Rust function the host oracle calls. The extraction roots are `transition` and the proof helpers, and they do not call `seek_one`, so the generated Lean does not name it. The proved collision check remains `occupied`. `gpu/nvvm` compiles that source with `rustc_codegen_nvvm` from rust-cuda `3a82c35a7196731fb60eebdac2a40d2b1bf0ba95` on nightly-2026-04-02, target `nvptx64-nvidia-cuda`, arch `compute_86`. The toolchain, the LLVM download, and the build directory are `/dev/shm/ai-town-rust`. Launches set `CUDA_VISIBLE_DEVICES=0`. Arena packing follows the h5i GPU layout. `cargo test --manifest-path gpu/Cargo.toml` checks that packing, and, when the PTX image is present, checks the device positions against `scalar_seek`. The hand-written `seek.cu` cubin stays as a baseline. The launch, the copies, and the `Relocate` write-back sit outside the proof.

`seek.cu` follows `scalar_seek`: move on x, then on y, then reject the whole step if the tile is blocked or another agent, at its original position, is inside the collision radius. The release cubin bench on this machine (sm_86) measured one seek at 1,000 / 10,000 / 50,000 agents. The CPU column here is that scalar scan:

| Agents | Scalar scan | Cubin |
|---:|---:|---:|
| 1,000 | 1.54 ms (650,328/s) | 6.66 ms (150,085/s) |
| 10,000 | 152.25 ms (65,682/s) | 12.82 ms (779,935/s) |
| 50,000 | 3,980 ms (12,562/s) | 16.67 ms (2,999,238/s) |

The coordinates matched at each size. At 1,000 agents the launch overhead is larger than the scan. From 10,000 up the cubin is faster than the scalar scan.

The fair bench times the product CPU step against that PTX seek. The CPU columns are `run_ticks` on the spatial grid and `transition(Command::Step)`, each covering 60 ticks. The GPU columns are 60 greedy steps of 12 milli-tiles from the same `seek_one`, one warmup then one timed minute. Device time is CUDA events. Wall time includes the copies.

| Agents | CPU `run_ticks` | CPU `Step` | GPU 60 kernel | GPU 60 wall |
|---:|---:|---:|---:|---:|
| 1,000 | 9.06 ms | 9.26 ms | 7.43 ms | 157.36 ms |
| 10,000 | 97.35 ms | 99.46 ms | 72.36 ms | 222.49 ms |
| 50,000 | 507.47 ms | 515.69 ms | 885.41 ms | 1,022.23 ms |

One launch, after a warmup, took 0.13 ms, 1.25 ms, and 15.11 ms of device time (wall 13.87 ms, 14.93 ms, and 19.56 ms). At 1,000 and 10,000 agents a minute of device time is under the spatial-grid tick. At 50,000 the grid tick is the faster of the two, and the copies dominate every wall time. The GPU seek is still one scan per agent. The spatial grid is what makes the CPU step cheaper at 50,000.

The sim loop can run that PTX path. Build `ai-town-server` with `--features gpu` and set `AI_TOWN_MOVE=gpu`. A dedicated thread owns the CUDA context. Each second it runs 60 seeks for the players whose path flag is moving, leaves everyone else in place as an obstacle, and commits `Command::Relocate`. A failed open or a failed tick logs a static line and the loop falls back to `Command::Step` for that second. With the feature off, `AI_TOWN_MOVE=gpu` logs `gpu seek unavailable` and keeps the CPU step.

End-to-end latency is `bench`'s `pipe` binary: `POST /rpc` `move_to`, then the batched tick, then the websocket snapshot whose x or y changed. It uses database `ai_town_pipe` on `127.0.0.1:18081` and the stub provider. `AI_TOWN_DRAIN=off` leaves the agent worker idle so the crowd is not also pathfinding. The world is a 128 by 128 open mask, the largest `Init` accepts (`MAX_DIM` is 128, 16,384 tiles). The gentle map is 64 by 48. A 50,000-agent HTTP world needs a wider map than `Init` allows, so that size stays in the in-memory fair bench.

| Mode | Agents | HTTP `move_to` | Position on the websocket |
|---|---:|---:|---:|
| cpu | 1,000 | 36.7 ms | 1,323 ms |
| cpu | 10,000 | 262.5 ms | 14,876 ms |
| gpu | 1,000 | 43.0 ms | 577 ms |
| gpu | 10,000 | 229.9 ms | 1,863 ms |

The sim loop wakes once a second, so the push includes the wait for the next tick. At 10,000 agents the CPU push was 14.9 s and the GPU push was 1.86 s. The GPU rows used the PTX image and `CUDA_VISIBLE_DEVICES=0`.

## Trusted boundary

The kernel does not see the clock, the network, or Postgres. Trusted pieces,
matching the h5i-app tutorials:

- the authenticator puts the user id and the attempt time on the principal
- user 0 is the sim loop, not a browser
- PostgreSQL commits the write set or nothing
- the agent worker sends commands as the agent user the kernel assigned
- the model, when enabled, only supplies text and a chosen tile. The kernel still accepts or refuses the command
- the optional GPU thread loads the PTX image of `seek_one` and submits `Relocate` as user 0
