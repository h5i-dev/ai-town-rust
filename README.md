# ai-town-rust

[AI Town](https://github.com/a16z-infra/ai-town) with a Rust backend,
PostgreSQL, and game rules extracted to Lean 4 through
[h5i-app](https://crates.io/crates/h5i-app). The React/PixiJS client talks to
HTTP and WebSocket endpoints. Agents use a user-managed OpenAI-compatible
inference server or a deterministic stub.

## Run

```bash
mkdir -p /dev/shm/ai-town-rust/pgdata
docker compose up -d
export DATABASE_URL=postgres://h5i:h5i@127.0.0.1:55432/h5i
export H5I_APP_SECRET=dev-secret
cargo run -p ai-town-server
```

In another shell:

```bash
cd frontend
npm install
npm run dev
```

Open <http://127.0.0.1:5173/ai-town/>. The backend listens on port 8080.
Without LLM configuration, agents use the deterministic stub. To enable model
inference, run an OpenAI-compatible server or proxy and configure the backend:

```bash
export LLM_BASE_URL=http://127.0.0.1:4000/v1
export LLM_MODEL=town-model
# Only if your inference server requires authentication:
export LLM_API_KEY=your-server-access-token
cargo run -p ai-town-server
```

`town-model` is the model name or alias exposed by your server. Configure upstream
provider credentials and routing in that server; AI Town does not read NVIDIA,
Gemini, or other provider keys. See [inference server setup](docs/LLM.md) for the
API contract, proxy guidance, and migration from the old NVIDIA settings.

## Town demo

```bash
./bench/demo.sh                    # 10,000 residents; starts backend + UI + browser
./bench/demo.sh --players 128      # choose 1–14,400 residents
./bench/demo.sh --engine original  # original AI Town game loop, same demo UI
./bench/demo.sh --seed 7 --no-open # reproducible setup / SSH
```

Residents walk independently around the original town terrain, read, garden,
daydream, invite neighbors, converse, and complete conversation memories. Both
engines use their real player, agent, conversation, pathfinding and collision
rules. External dialogue and memory operations use fixed offline responses;
this is a disposable simulation with no LLM, API key, PostgreSQL or Convex.
Click **Neighborhood** to see activities and conversations up close, or **Overview**
to see the population. Select a resident to inspect its activity and conversation. The UI shows active
routes, activities, conversations, and completed messages/memories.

Requires Node.js 20+, npm, Python 3, and curl; Rust mode also needs Rust/Cargo.
The first run builds the backend and installs frontend dependencies. Open
<http://127.0.0.1:5174/ai-town/> if a browser is unavailable; drag to pan and
scroll to zoom. Ctrl-C stops both servers. Use `--port` and `--api-port` to
change ports. Logs and the reusable build cache live under the temporary directory.

`--engine original` loads `Game` from `~/Ref/ai-town`. Town mode adds a small
zero-duration path interpolation guard to the bundle; the checkout stays intact.
See the [benchmark conditions](docs/TOWN_BENCHMARKS.md) for the fix and scope.
For another checkout:

```bash
AI_TOWN_UPSTREAM=/path/to/ai-town ./bench/demo.sh --engine original --players 128
```

Large populations occupy repeated copies of the original map, all in one game
instance. The copies have blocked edges and do not allow travel between districts.
At 10,000 residents this uses a 256 × 192 map, beyond the production server's
128 × 128 map limit. It demonstrates the in-memory engine, not deployed capacity.
The seeded starting state mixes walking, activities, invitations and ongoing
conversations; subsequent decisions use the same offline policy in both engines.
Native scheduling and acceptance rules differ, so their trajectories differ too.

The original engine runs in a worker so HTTP and camera controls remain responsive
during slow ticks. Both engines display computed positions and simulated time;
at 10,000 residents the original can advance far below real time. Original mode
also packs its position history; Rust does not produce those buffers. The demo
adapts batch sizes for responsiveness; use the fixed-step
[town benchmark](docs/TOWN_BENCHMARKS.md) for timing comparisons.

The recorded Rust town benchmark processes 0.96 simulated seconds for 10,000
residents in 0.389 seconds (previously 43.626 seconds). A longer 76.8-second
simulation takes 23.60 seconds, with new activities, invitations and memories.
A separate movement-only GPU benchmark takes 16.78 ms for 60 rounds including
preparation and checked relocation (previously 70.95 ms).
These are in-memory CPU measurements; snapshot serialization, database and LLM
costs have separate scope. See [results and profiling](docs/TOWN_BENCHMARKS.md).

`./bench/demo.sh --scenario crowd` retains the old synchronized, open-field
movement demo for either engine. It has no autonomous agents or conversations.

## Verification

```bash
cargo test --workspace
h5i app prove
h5i app mutate
```

Proof extraction needs Charon, Aeneas, and Lean 4.31.0 (`h5i app doctor`).
`h5i app check` checks the committed Lean files without extracting again.

Every CPU movement tick checks placement and restores the previous positions
if the check fails. Optional CUDA movement writes through `Relocate`, which
checks walkable tiles and player separation before accepting coordinates.
GPU mode currently skips agent, conversation, facing, and clock updates.
Lean proves placement safety; GPU/CPU behavioral equivalence and the whole
server are outside these proofs. See the
[proof boundary](docs/PLAN.md#trusted-boundary).

## Performance

The [town benchmark](docs/TOWN_BENCHMARKS.md) exercises the same offline
resident simulation as the default demo, with raw samples and completed-work
counts for both engines:

```bash
./bench/town.sh --players 32,128,1000,10000 --samples 3
./bench/town.sh --players 32,128 --steps 80 --samples 3 # 76.8 simulated seconds
```

The [earlier movement microbenchmarks](docs/BENCHMARKS.md) measure synchronized
walking, GPU movement and HTTP latency. Their numbers do not describe autonomous
town life or equivalent full-game work.

## Code

- `kernel/`: pure game transitions; `proofs/`: generated Lean and theorems.
- `server/`: HTTP, WebSocket, PostgreSQL, simulation, and agent worker.
- `frontend/`: React/PixiJS client; `gpu/`: optional CUDA movement.
- [Design notes](docs/PLAN.md): port mapping, movement rules, and proof scope.
