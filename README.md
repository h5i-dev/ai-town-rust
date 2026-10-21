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

`fair-bench` times 60 two-phase rounds at 1,000, 10,000, 50,000, and 100,000
agents in two crowds (`AI_TOWN_SCENARIO`). `east`: every agent walks east in
step, one tile from its neighbors, so no landing is ever contested. `head-on`:
each row alternates eastbound and westbound agents 1.5 tiles apart, so every
pair meets within the minute and keeps proposing into a refused landing.
`Refused` counts (round, agent) proposals that did not land; at 18.5 per
agent, the head-on crowd spends most of the minute on contested landings.

`CPU tick` is the product `run_ticks` (path pose, a refused step's wait, and
the round). `CPU 1 thread` is 60 calls of `tick_round`, candidates prepared
before the timer. `CPU pool` runs the same rounds on 8, 16, 32, or 64 worker
threads (`AI_TOWN_CPU_THREADS`) started once and synchronized by a spinning
barrier; the workers also clear and fill the grid with atomic pushes, like the
PTX inserts, so no phase is serial. The table gives the fastest pool and its
thread count. The GPU columns are `RustDevice::rounds`: one copy in, the
minute resident on the device, one copy out; kernel time is the CUDA events
around the memsets and launches. In every row the single-thread round, every
pool, and the GPU landed on the same coordinates, and in `east` so did
`run_ticks`.

| Scenario | Agents | Refused | CPU tick | CPU 1 thread | CPU pool, best | GPU 60 kernel | GPU 60 wall |
|---|---:|---:|---:|---:|---:|---:|---:|
| head-on | 1,000 | 18,500 | 8.80 ms | 3.26 ms | 5.30 ms (8) | 1.26 ms | 12.60 ms |
| head-on | 10,000 | 185,000 | 84.15 ms | 33.41 ms | 15.21 ms (32) | 1.37 ms | 11.00 ms |
| head-on | 50,000 | 925,000 | 440.86 ms | 170.27 ms | 35.25 ms (32) | 1.90 ms | 12.99 ms |
| head-on | 100,000 | 1,850,000 | 1,038.02 ms | 396.55 ms | 64.48 ms (32) | 6.35 ms | 20.94 ms |
| east | 1,000 | 0 | 10.64 ms | 4.47 ms | 4.52 ms (64) | 1.30 ms | 12.58 ms |
| east | 10,000 | 0 | 111.10 ms | 48.14 ms | 16.22 ms (64) | 1.54 ms | 5.95 ms |
| east | 50,000 | 0 | 575.35 ms | 243.06 ms | 42.56 ms (32) | 2.07 ms | 9.63 ms |
| east | 100,000 | 0 | 1,206.79 ms | 488.46 ms | 68.61 ms (32) | 5.71 ms | 35.08 ms |

At 100,000 agents the best CPU pool takes 64 to 69 ms against 21 to 35 ms of
GPU wall time and 6 ms of kernel time: about 2 to 3 times faster end to end
and about 10 times in kernel time. Below 10,000 agents the device copies cost
more than the CPU pool. 128 threads are not in the table: on this shared
machine (load average 15 to 53 during the run) a spinning pool that large
oversubscribes the cores and took 1.6 to 3.0 s. An earlier pool woke its
workers with a condvar and built the grid on the main thread; it bottomed out
near 210 ms at 100,000 agents, which made the GPU look about 10 times faster
than it is.

The run used GPU 2 at 2026-10-21 08:20 EDT. All four RTX 3090s were at 98 to
100% utilization with 16,238 to 17,444 MiB in use by other users' processes,
which were left running, so the GPU times are an upper bound for an idle card.
CSVs: `/dev/shm/ai-town-rust/claude/v3/fair-{headon,east}.csv`.

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
