# ai-town-rust

[AI Town](https://github.com/a16z-infra/ai-town) with a Rust backend,
PostgreSQL, and game rules extracted to Lean 4 through
[h5i-app](https://crates.io/crates/h5i-app). The React/PixiJS client talks to
HTTP and WebSocket endpoints. Agents use an OpenAI-compatible inference server
or a deterministic stub.

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

## LLM

Without configuration, agents use the deterministic stub. To use a model, point
the backend at any OpenAI-compatible server or proxy:

```bash
export LLM_BASE_URL=http://127.0.0.1:4000/v1
export LLM_MODEL=town-model     # model name or alias exposed by that server
export LLM_API_KEY=...          # only if the server requires a token
cargo run -p ai-town-server
```

AI Town reads no provider keys; Gemini, NVIDIA, and other credentials belong in
the proxy. For example, [h5i-ai-gateway](https://github.com/h5i-dev/h5i-ai-gateway)
(see its `examples/config.yaml`) can expose `town-model`:

```bash
# in an h5i-ai-gateway checkout
cargo build --release -p gateway-server
cat > town.yaml <<'YAML'
model_list:
  - model_name: town-model
    litellm_params:
      model: gemini/gemini-3.1-flash-lite   # or nvidia_nim/google/diffusiongemma-26b-a4b-it
      api_key: os.environ/GEMINI_API_KEY    # or os.environ/NVIDIA_NIM_API_KEY
general_settings:
  master_key: os.environ/LITELLM_MASTER_KEY
YAML
export LITELLM_MASTER_KEY=sk-town GEMINI_API_KEY=...
/dev/shm/h5i-ai-gateway/target/release/h5i-gateway --config town.yaml --port 4000
```

Then run the backend with `LLM_API_KEY=sk-town`. The build directory is set in
the gateway's `.cargo/config.toml`. See [inference server setup](docs/LLM.md)
for the API contract, verified models, and migration from the old `NVIDIA_*`
settings.

## Town demo

```bash
./bench/demo.sh                    # 10,000 residents; starts backend + UI + browser
./bench/demo.sh --players 128      # choose 1–14,400 residents
./bench/demo.sh --engine original  # original AI Town game loop, same demo UI
./bench/demo.sh --seed 7 --no-open # reproducible setup / SSH
```

Residents walk, read, garden, invite neighbors, converse, and write memories,
using fixed offline responses (no LLM, PostgreSQL, or Convex). Both engines run
their real game rules. Large populations use repeated copies of the town map,
beyond the server's 128 × 128 limit, so this shows the in-memory engine rather
than deployed capacity. Requirements, controls, and the original-engine setup
are in the [demo notes](docs/TOWN_BENCHMARKS.md#demo).

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

Offline town simulation, 10,000 residents, in-memory CPU, including tick
history recording (2026-10-23):

| Engine | 0.96 simulated seconds |
|---|---:|
| Original AI Town | 214.88 s |
| Rust | 0.385 s |

The engines complete different native work, so this is not a feature-equivalent
speedup ratio. Database, transport, and LLM costs are excluded. Reproduce with
`./bench/town.sh --players 32,128,1000,10000 --samples 3`; see
[results and profiling](docs/TOWN_BENCHMARKS.md), which also covers GPU
movement (16.78 ms for 60 checked rounds). The
[movement microbenchmarks](docs/BENCHMARKS.md) cover synchronized walking and
HTTP latency.

## Code

- `kernel/`: pure game transitions; `proofs/`: generated Lean and theorems.
- `server/`: HTTP, WebSocket, PostgreSQL, simulation, and agent worker.
- `frontend/`: React/PixiJS client; `gpu/`: optional CUDA movement.
- [Design notes](docs/PLAN.md): port mapping, movement rules, and proof scope.
