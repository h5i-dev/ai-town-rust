# ai-town-rust

AI Town (a16z-infra/ai-town) with the Convex backend replaced by an
[h5i-app](https://crates.io/crates/h5i-app) kernel. Game rules live in
`kernel/` and are extracted to Lean 4. The shell in `server/` is HTTP,
WebSocket, PostgreSQL, a one-step-per-second simulation, and an agent worker.
`frontend/` is the React and PixiJS client.

The design map from each Convex module to kernel, shell, or worker is
[`docs/PLAN.md`](docs/PLAN.md).

## Layout

| Path | What it is |
|---|---|
| `kernel/` | Pure transitions: world, movement, conversations, inputs. Depends on `h5i-app-schema` and `h5i-app-sql` 0.1.2 |
| `server/` | h5i-app 0.1.2 with the `http` and `postgres` features, sim loop, NIM or stub agents |
| `frontend/` | PixiJS town. A small HTTP and WebSocket client replaces the Convex hooks |
| `proofs/` | Lean 4 postconditions: exclusivity, membership, collision, authorization, and the shared movement step. See `docs/PLAN.md` |
| `gpu/` | Optional CUDA movement, outside the workspace. PTX entry `step_batch` is `kernel`'s `tick_moves`; `seek.cu` and `seek_one` stay the cubin baseline |
| `assets/gentle.json` | The gentle map. The shell reads the collision mask. The client draws the tiles |

## Run

PostgreSQL uses a data directory on tmpfs, not in this repo.

```bash
mkdir -p /dev/shm/ai-town-rust/pgdata
docker compose up -d
export DATABASE_URL=postgres://h5i:h5i@127.0.0.1:55432/h5i
export H5I_APP_SECRET=dev-secret
cargo run -p ai-town-server
```

The server listens on `127.0.0.1:8080`. In another shell:

```bash
cd frontend && npm install && npm run dev
```

The Vite dev server proxies `/rpc`, `/ws`, `/map`, `/snapshot`, `/session`, and `/healthz` to port 8080. The page is `http://127.0.0.1:5173/ai-town/`.

`NVIDIA_API_KEY` selects the NVIDIA NIM provider
(`https://integrate.api.nvidia.com/v1`). The default model is
`google/diffusiongemma-26b-a4b-it`, and `NVIDIA_MODEL` overrides it. With no
key, agents use the deterministic stub. The process never prints the key.

## Tests and proofs

```bash
cargo test
h5i app prove
h5i app mutate
```

`h5i app prove` needs Charon, Aeneas, and Lean v4.31.0 on `PATH`
(`h5i app doctor`). It regenerates `proofs/generated/` from the kernel.
Those Lean files are committed with the kernel change. `h5i app check`
builds them and does not extract again.

## Benchmarks

```bash
mkdir -p /dev/shm/ai-town-rust/bench
cargo run --release -p ai-town-bench
```

Results are CSV files under `/dev/shm/ai-town-rust/bench/`. The optional CUDA
package is `gpu/`, excluded from the workspace, so `cargo test` does not need
nvcc. The hand-written cubin baseline, default arch `sm_86`:

```bash
cargo test --manifest-path gpu/Cargo.toml
cargo run --release --manifest-path gpu/Cargo.toml --bin seek-bench
```

The product GPU path compiles the same `kernel` `tick_moves` to PTX with
`rustc_codegen_nvvm` (rust-cuda `3a82c35`, nightly-2026-04-02). The toolchain,
the LLVM download, and the device target stay under `/dev/shm/ai-town-rust`.
One GPU:

```bash
gpu/nvvm/setup.sh
gpu/nvvm/build-device.sh
CUDA_VISIBLE_DEVICES=0 cargo run --release --manifest-path gpu/Cargo.toml --bin fair-bench
```

`fair-bench` times one CPU spatial-grid step against 60 rounds of that PTX
tick, at 1,000, 10,000, and 50,000 agents. The device entry `step_batch` runs
`tick_moves` on one thread, in order, so a later agent sees an earlier landing.
On this machine the 60-round device time was 126.37 ms, 2,588.81 ms, and
14,442.59 ms. The spatial-grid `run_ticks` time was 9.15 ms, 94.99 ms, and
481.09 ms. Wall time, including host path poses, grid rebuilds, and copies,
was 1,228.94 ms, 3,632.88 ms, and 14,988.62 ms. Every position matched the CPU
run. GPU 0 was busy with another job, so these times are an upper bound. The
CPU spatial tick is faster at each size. `seek_one` and `seek.cu` stay the
hand-written baseline in `docs/PLAN.md`.

The default server tick is that CPU step. `--features gpu` together with
`AI_TOWN_MOVE=gpu` replaces it with 60 rounds of the PTX tick and writes
positions through `Command::Relocate`. Each round retries the path pose.
`Relocate` writes x, y, and speed 0, and clears paths. Conversations and the
clock advance on `Step`. The pipe figures below are from the earlier `seek_one`
path (database `ai_town_pipe`, `AI_TOWN_DRAIN=off`):

```bash
AI_TOWN_DRAIN=off cargo run --release -p ai-town-bench --bin pipe
AI_TOWN_MOVE=gpu AI_TOWN_DRAIN=off CUDA_VISIBLE_DEVICES=0 \
  cargo run --release -p ai-town-bench --features gpu --bin pipe
```

On that greedy path, from the `move_to` POST until a websocket snapshot showed
the body move: CPU 1.32 s at 1,000 agents and 14.9 s at 10,000; GPU 0.58 s and
1.86 s. The HTTP call itself was 37 ms and 262 ms on the CPU path, 43 ms and
230 ms on the GPU path. `Init` stops at 128 tiles on a side, so the
50,000-agent figure stays in the in-memory fair bench.

Creating 10,000 agents on the spatial grid, then pathing them, took 22.3 s and
49.6 s on an idle machine (71.9 s together). The step after that was 193 ms.

A headless Chromium session opened the Vite page, rendered the town, and
showed Lucky's NIM line in the sidebar. Screenshots are under
`/dev/shm/ai-town-rust`. The full tables are in `docs/PLAN.md`.
