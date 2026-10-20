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

`Step` is refused for every other user. The shell's authenticator is the only
code that can mint the user-0 token, and it never sends that token to a browser.

## Invariants

The proofs in `proofs/` are about `transition`.

| Theorem | Statement |
|---|---|
| Conversation exclusivity | A successful command leaves each player in at most one conversation |
| Membership consistency | Every conversation has exactly two members, both players exist, the creator is one of them, and the status pair is invited/walking, walking/walking, or participating/participating |
| Authorization | A non-engine command does not change another player's owner, position, facing, speed, or path. It only writes conversation rows for a conversation it is allowed to touch |
| Collision safety | After a successful command, every player stands on a walkable tile and no two players are inside the collision radius |

Refusals commit nothing, which is the engine's contract. `apply` is what a
later read returns. `h5i app prove` checks the proofs. `h5i app mutate` checks
that the mutants in `h5i-app.toml` break them.

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

## Benchmarks and the GPU

`ai-town-bench` writes under `/dev/shm/ai-town-rust/bench/`:

- in-memory ticks per second against the agent count
- HTTP input latency against the agent count, with the sim loop running

`kernel` also has a batch position step. A test checks it against the scalar
step on the same inputs. That is the CPU side of the later GPU backend. A CUDA
build is not part of this tree yet. `docs/PLAN.md` is the place that records
it. The intended check is the same arrays on CPU and on one of the RTX 3090s,
compared element-wise, with Lean still covering only the extracted CPU kernel.
GPU execution stays trusted, as in h5i's GPU note.

## Trusted boundary

The kernel does not see the clock, the network, or Postgres. Trusted pieces,
matching the h5i-app tutorials:

- the authenticator puts the user id and the attempt time on the principal
- user 0 is the sim loop, not a browser
- PostgreSQL commits the write set or nothing
- the agent worker sends commands as the agent user the kernel assigned
- the model, when enabled, only supplies text and a chosen tile. The kernel still accepts or refuses the command
