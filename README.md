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
| `proofs/` | Lean 4 postconditions: exclusivity, membership, collision, authorization, the sequential step, and the two-phase round. See `docs/PLAN.md` |
| `gpu/` | Optional CUDA movement, outside the workspace. PTX entry `round_phase` is one thread per agent for the two-phase round. `step_batch` (`tick_moves`) and `seek.cu` stay baselines |
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

The product GPU path compiles the same `kernel` functions `propose_one` and
`resolve_one` to PTX with `rustc_codegen_nvvm` (rust-cuda `3a82c35`,
nightly-2026-04-02). The toolchain, the LLVM download, and the device target
stay under `/dev/shm/ai-town-rust`. One GPU:

```bash
gpu/nvvm/setup.sh
gpu/nvvm/build-device.sh
CUDA_VISIBLE_DEVICES=0 cargo run --release --manifest-path gpu/Cargo.toml --bin fair-bench
```

`fair-bench` times the two-phase round on the CPU and on the GPU at 1,000,
10,000, 50,000, and 100,000 agents. Every agent walks east on an open mask, so
the rules agree and nobody waits. `cpu_tick_ms` is the product `run_ticks`
(path pose, a refused step's wait, and the round). `cpu_round_ms` is 60 calls
of `tick_round` on one thread, with candidates prepared before the timer.
`cpu_parallel_ms` is those rounds on a pool of 8 threads, started once. The
grid build stays on the main thread. This machine reported 128 hardware
threads. The GPU columns are `RustDevice::rounds`: buffers stay on the device
for the minute, with one copy in and one copy out. Kernel time is the CUDA
events around the memsets and the launches.

| Agents | CPU tick | CPU round | CPU 8 threads | GPU one kernel | GPU one wall | GPU 60 kernel | GPU 60 wall |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 10.518 ms | 4.442 ms | 7.427 ms | 0.025 ms | 18.314 ms | 1.297 ms | 12.631 ms |
| 10,000 | 110.665 ms | 47.452 ms | 49.898 ms | 0.030 ms | 15.958 ms | 1.553 ms | 9.339 ms |
| 50,000 | 570.884 ms | 241.943 ms | 176.000 ms | 0.038 ms | 1.820 ms | 2.011 ms | 8.286 ms |
| 100,000 | 1,202.314 ms | 481.704 ms | 312.842 ms | 0.059 ms | 1.713 ms | 5.752 ms | 15.447 ms |

Positions matched across `run_ticks`, the single-thread round, the 8-thread
pool, and the GPU. The pool takes 7.427 ms at 1,000 agents, where one thread
takes 4.442 ms, and 49.898 ms at 10,000, where one thread takes 47.452 ms. It
takes 176.000 ms at 50,000 and 312.842 ms at 100,000, where one thread takes
241.943 ms and 481.704 ms. The 60-round GPU kernel takes 1.297 ms, 1.553 ms,
2.011 ms, and 5.752 ms. The one-round wall of 18.314 ms at 1,000 agents pays
for device allocation. The one-round walls at 50,000 and 100,000 agents are
1.820 ms and 1.713 ms.

The run used GPU 2. A snapshot at 2026-10-21T06:43:58-04:00 showed all four
RTX 3090s busy: 93%, 95%, 100%, and 100% utilization, with 21,440, 21,228,
22,048, and 23,636 MiB of 24,576 MiB in use. GPU 2, the device that produced
these times, was at 100% and 22,048 MiB, so the times are an upper bound for
an idle card. Other processes were left running. The one-thread `tick_moves`
table and the earlier `seek_one` table stay in `docs/PLAN.md`.

The default server tick is this two-phase round. `AI_TOWN_TICK=seq` selects
`Command::StepSeq`, which only the engine can send. That is the old
index-order rule: a later agent sees landings already accepted in the same
tick, including a gap someone just left. The two-phase rule reads the previous
positions only, so that step waits. Resolve keeps a candidate when no
better-priority proposal lies inside the collision radius. Better means a
lower player id, then a lower index. A lower id that is itself rejected still
makes the higher id stay. A refused walker on the CPU enters `PF_WAIT` for
about a second. The GPU minute keeps the moving flags from the start of the
minute and retries the later path pose.

`--features gpu` together with `AI_TOWN_MOVE=gpu` replaces the CPU step with
60 rounds of `round_phase` and writes positions through `Command::Relocate`.
Candidates for the minute are copied once. The device rebuilds the spatial
grid between phases. `Relocate` writes x, y, and speed 0, and clears paths.
Conversations and the clock advance on `Step`. The pipe below is this two-phase
round on database `ai_town_pipe` with `AI_TOWN_DRAIN=off`. The joined player is
a session whose spawn tile has a free orthogonal step, and `move_to` targets
that neighbor. A one-tile pocket can have a legal step that leads away from a
far tile, and `begin_move` then returns success with no path. After the body
moves, the bench sends `stop`. The server holds one lock across the sim tick
and each RPC, so a path committed during the device minute is still present
when `Relocate` runs. The GPU minute's last sample is the wall clock when
`meta.now_ms` is more than 960 ms behind that clock. Paths are stamped with
the command clock.

```bash
AI_TOWN_DRAIN=off cargo run --release -p ai-town-bench --bin pipe
AI_TOWN_MOVE=gpu AI_TOWN_DRAIN=off CUDA_VISIBLE_DEVICES=2 \
  cargo run --release -p ai-town-bench --features gpu --bin pipe
```

From the `move_to` POST until a websocket snapshot showed the body move: CPU
1,164.234 ms at 1,000 agents and 15,293.189 ms at 10,000; GPU 263.860 ms and
1,173.561 ms. The HTTP call itself was 45.848 ms and 478.995 ms on the CPU
path, 41.644 ms and 359.155 ms on the GPU path. Both rows are that same server
build. The GPU run used GPU 2. A snapshot at 2026-10-21T07:37:30-04:00 showed
that card idle at 0% utilization and 4 MiB of 24,576 MiB. The other three RTX
3090s were at 100%, 98%, and 100%, with 20,508, 21,420, and 21,982 MiB in use,
and those processes were left running. `Init` stops at 128 tiles on a side, so
the 50,000-agent and 100,000-agent figures stay in the in-memory fair bench.
The earlier `seek_one` pipe is in `docs/PLAN.md`.

Creating 10,000 agents on the spatial grid, then pathing them, took 22.3 s and
49.6 s on an idle machine (71.9 s together). The step after that was 193 ms.

A headless Chromium session opened the Vite page, rendered the town, and
showed Lucky's NIM line in the sidebar. Screenshots are under
`/dev/shm/ai-town-rust`. The full tables are in `docs/PLAN.md`.
