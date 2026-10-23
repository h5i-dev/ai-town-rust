# Movement microbenchmarks (historical)

For the current default demo workload, see [Town benchmarks](TOWN_BENCHMARKS.md).
The measurements below use synchronized movement without autonomous residents.
They do not measure the town-life scenario.

Measured on 2026-10-21 after the CPU `settle` check and GPU `Relocate`
placement check landed. These replace the earlier README tables.

## Original AI Town

The baseline runs the actual `Game` class from `~/Ref/ai-town` at
`8e05997f2409275669c8344b84a51692e83f3f33`, bundled without source edits or
simulation replacements. Our kernel/server are at
`b7465c8f7894312465e032242a2f97b1c41b96ca`; the harness changes accompany
this report. Three measured samples follow one warmup per size.

Each sample starts with the same open map, positions, paths, and timestamp.
Players walk east at 0.75 tiles/second, one tile apart.
A batch is **60 ticks of 16 ms: 0.96 simulated seconds**.
Both harnesses check that every player moves exactly 0.72 tiles (720 Rust
milli-tiles, or floating point within 1e-8 upstream). There are no agent
records, conversations, obstacles, inputs, or LLM calls. Counts mean moving
players, not autonomous agents.

All values are milliseconds per 60-tick batch: median (minimum–maximum). Upstream has three measured samples per size; Rust has five.

| Players | Original tick | Rust CPU | Rust CPU sequential | GPU checked | GPU checked + device-grid checks |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 1,476.299 (1,424.867–1,526.718) | 14.168 (14.147–15.038) | 12.527 (12.497–12.529) | 7.404 (7.374–7.443) | 12.584 (12.561–12.628) |
| 10,000 | 128,668.396 (126,758.160–131,677.419) | 147.898 (147.839–148.665) | 129.689 (129.397–130.066) | 63.538 (63.406–64.217) | 83.235 (83.168–83.391) |

Upstream history packing adds 29.993 ms at 1,000 players, 984.578 ms at 10,000 players.

Upstream timing includes `beginStep`, `Game.tick`, and per-tick position
history recording. `takeDiff` serialization/history packing is timed
separately. Rust CPU includes `run_ticks` and all 60 placement checks; initial
snapshot cloning is outside the timer. `CPU sequential` uses `run_ticks_seq`,
the index-order movement rule with our spatial lookup and placement backstop.
The default CPU uses two-phase movement. Both produce the same coordinates
in this uncontested fixture. Upstream collision checks scan the other
players; Rust uses spatial lookups. These timings compare the implementations,
not language throughput.

`GPU checked` includes fixture/candidate packing, device allocations, copies,
60 rounds, output copies, snapshot cloning, and an accepted `relocate` call.
The server additionally groups paths, queues work, persists to PostgreSQL,
and publishes snapshots. The final placement check is always enabled.
`AI_TOWN_GPU_VERIFY=1` additionally copies back and checks both device grids
in every round.

These are implementation costs for this workload, not a Convex/PostgreSQL
deployment comparison or feature-equivalent full game steps. Rust does not
produce upstream's historical interpolation buffers. The GPU pass does not
advance agent rules, conversations, facing, or the simulation clock.

## Rust movement and verification

Five measured samples follow one warmup per size. Tables show medians; CSVs
retain all samples. Raw single-thread rounds, the 32-thread pool, and raw GPU
timings receive precomputed candidates and omit final placement verification.
Use `GPU checked` for the checked movement pipeline.

Milliseconds per batch, medians:

| Scenario | Players | CPU step, checked | CPU rounds, raw | CPU pool 32, raw | GPU events | GPU wall, raw | GPU checked |
|---|---:|---:|---:|---:|---:|---:|---:|
| east | 1,000 | 14.168 | 4.421 | 6.188 | 1.423 | 1.748 | 7.404 |
| east | 10,000 | 147.898 | 47.498 | 21.689 | 1.665 | 3.027 | 63.538 |
| east | 50,000 | 792.938 | 243.280 | 42.104 | 2.204 | 7.531 | 310.877 |
| east | 100,000 | 1,648.579 | 485.603 | 76.416 | 3.595 | 12.692 | 616.559 |
| headon | 1,000 | 11.899 | 3.258 | 5.930 | 1.441 | 1.781 | 7.187 |
| headon | 10,000 | 117.851 | 32.852 | 18.996 | 1.489 | 2.865 | 60.784 |
| headon | 50,000 | 641.339 | 169.163 | 33.012 | 2.045 | 7.324 | 295.402 |
| headon | 100,000 | 1,525.910 | 384.979 | 57.876 | 3.453 | 12.563 | 588.903 |

East pipeline breakdown, medians in milliseconds:

| Players | Prepare | Relocate including clone | GPU checked | GPU checked + grid checks |
|---:|---:|---:|---:|---:|
| 1,000 | 5.466 | 0.074 | 7.404 | 12.584 |
| 10,000 | 57.985 | 1.889 | 63.538 | 83.235 |
| 50,000 | 289.877 | 11.288 | 310.877 | 387.610 |
| 100,000 | 579.679 | 21.584 | 616.559 | 747.692 |

At 10,000 eastbound players, candidate preparation takes 57.985 ms, versus 1.889 ms for cloning and the final placement check. Raw GPU wall time alone omits both. Independent medians of components need not sum to the median total.

`east` is the uncontested fixture above. `headon` alternates eastbound and
westbound players 1.5 tiles apart. Raw single-thread rounds, the pool, and
CUDA must produce identical coordinates in both scenarios. Product CPU steps
enter a wait state on refusal; the GPU batch retries later path poses.
Head-on timings therefore describe different product behavior, not a speedup
comparison against upstream.

Normal GPU event time brackets device memsets and launches. With optional
grid verification, the interval also spans host checks and copies and is
not pure kernel time. CSVs separate preparation and `relocate`; the latter
includes snapshot cloning. The server's map limit is 128 × 128 tiles;
50,000- and 100,000-player fixtures use wider, in-memory-only maps.

## HTTP to visible movement

The pipe runs our server, PostgreSQL, and a WebSocket client on localhost.
It creates 1,000 or 10,000 idle agents, joins one human with a free neighbor,
sends `move_to`, and waits for changed coordinates in a snapshot. `input_ms`
ends at the HTTP response; `push_ms` ends at that snapshot. The human stops
and leaves between samples. Three samples per size, no discarded warmup;
`AI_TOWN_DRAIN=off` disables the asynchronous agent worker; the provider is
the stub. Both modes use the same server configuration. The GPU run adds
`used_gpu` to the tick log; this is the only server-code difference. The
harness also rejects GPU samples if the simulation clock advances, which
would identify CPU fallback. Setup inserts agents directly through the engine
and can race a GPU batch: a changed player count rejects `Relocate` and falls
back to CPU. These setup ticks are outside the samples; measured GPU ticks
must use the GPU.

Milliseconds, median (minimum–maximum):

| Mode | Idle agents | HTTP response | Changed snapshot |
|---|---:|---:|---:|
| cpu | 1,000 | 43.850 (42.878–46.829) | 809.374 (771.684–809.882) |
| cpu | 10,000 | 516.810 (446.841–669.727) | 16,411.528 (15,189.120–17,311.480) |
| gpu | 1,000 | 42.892 (42.863–44.124) | 776.209 (105.257–783.960) |
| gpu | 10,000 | 535.705 (339.925–598.862) | 1,321.859 (1,139.794–1,440.341) |


GPU mode currently replaces the full `Step` with movement-only `Relocate`.
The CPU run processes the idle agents' rules; the GPU run does not. These
latencies therefore compare the current modes, not equivalent full-game work.

Push latency includes the once-per-second tick schedule, database work,
serialization, and transport. The original's deployment latency is unmeasured.
Each pipe run recreates only `ai_town_pipe` and binds `127.0.0.1:18081`.

## Machine and evidence

Two AMD EPYC 7502 CPUs: 64 physical cores, 128 hardware threads. GPU 0 is an
RTX 3090 with 24 GiB. All GPUs were idle before measurement. Rust 1.98.1,
Node 24.21.0, release Rust builds; Convex 1.41.0 matches upstream's lockfile.
This shared machine has no exclusive CPU reservation. Setup builds overlapped
part of the upstream run; benchmark workloads ran sequentially. Few samples
and fixed run order limit conclusions about close timings.

[Raw results and metadata](../bench/results/2026-10-21/) include revisions,
hardware/load snapshots, and the rebuilt PTX hash. PTX uses the current
kernel, `rustc_codegen_nvvm`, rust-cuda
`3a82c35a7196731fb60eebdac2a40d2b1bf0ba95`, nightly-2026-04-02, `compute_86`.
All six CUDA tests passed with PTX required, including differential tests with
checked grids; all 43 workspace tests passed. `relocate_placed` and
`settle_placed` prove placement safety. Benchmarks establish agreement for the
exercised inputs, not a general GPU equivalence theorem.

## Reproduce

From this repository, with upstream at `~/Ref/ai-town`:

```bash
export AI_TOWN_BENCH_DIR=/dev/shm/ai-town-rust/bench-current
npm ci --prefix bench/upstream --ignore-scripts --no-audit --no-fund
AI_TOWN_BENCH_AGENTS=1000,10000 AI_TOWN_BENCH_SAMPLES=3 \
  npm --prefix bench/upstream run bench

export CARGO_TARGET_DIR=/dev/shm/ai-town-rust/target-main
gpu/nvvm/build-device.sh  # gpu/nvvm/setup.sh first on a new machine
cargo build --release --manifest-path gpu/Cargo.toml --bin fair-bench
export CUDA_VISIBLE_DEVICES=0 AI_TOWN_CPU_THREADS=32 AI_TOWN_BENCH_SAMPLES=5
AI_TOWN_SCENARIO=east "$CARGO_TARGET_DIR/release/fair-bench"
AI_TOWN_SCENARIO=headon "$CARGO_TARGET_DIR/release/fair-bench"
AI_TOWN_GPU_VERIFY=1 AI_TOWN_SCENARIO=east \
  AI_TOWN_BENCH_DIR="$AI_TOWN_BENCH_DIR/grid-checked" \
  "$CARGO_TARGET_DIR/release/fair-bench"

cargo build --release -p ai-town-bench --features gpu --bin pipe
AI_TOWN_MOVE=cpu AI_TOWN_DRAIN=off AI_TOWN_BENCH_SAMPLES=3 \
  "$CARGO_TARGET_DIR/release/pipe"
AI_TOWN_MOVE=gpu AI_TOWN_DRAIN=off AI_TOWN_BENCH_SAMPLES=3 \
  "$CARGO_TARGET_DIR/release/pipe"
```

`AI_TOWN_UPSTREAM` changes the reference checkout. `AI_TOWN_BENCH_AGENTS`
and `AI_TOWN_BENCH_SAMPLES` control the in-memory harnesses;
`AI_TOWN_CPU_THREADS` accepts a comma-separated list. Separate output
directories preserve results from different verification modes.
