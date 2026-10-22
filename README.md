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

60 ticks of uncontested movement, medians in milliseconds (2026-10-21):

| Players | Original AI Town | Rust CPU | GPU with placement check |
|---:|---:|---:|---:|
| 1,000 | 1,476.3 | 14.2 | 7.4 |
| 10,000 | 128,668.4 | 147.9 | 63.5 |

GPU timing includes preparation, copies, and checked write-back. The original
also records position history. This is an in-memory movement comparison;
[methods, grid-check costs, server latency, and raw samples](docs/BENCHMARKS.md)
give the scope and reproduction commands.

## Code

- `kernel/`: pure game transitions; `proofs/`: generated Lean and theorems.
- `server/`: HTTP, WebSocket, PostgreSQL, simulation, and agent worker.
- `frontend/`: React/PixiJS client; `gpu/`: optional CUDA movement.
- [Design notes](docs/PLAN.md): port mapping, movement rules, and proof scope.
