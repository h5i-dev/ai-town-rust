# Offline town benchmarks

The default demo and `bench/town.sh` use the same fixture generator and engine
adapters. This replaces synchronized eastbound walking as the main demo and
benchmark workload. The older [movement microbenchmarks](BENCHMARKS.md) remain
useful for isolated movement costs.

The latest results with tick history enabled in both engines are in
[Tick history and playback](#tick-history-and-playback--2026-10-23). Earlier
optimization tables retain their original measurement scope.

## Workload

Every resident has a native player and agent record. The original gentle map
supplies both graphics and collision terrain. Starting positions are shuffled
with a reproducible seed and occupy distinct walkable tiles. The fixture mixes
independent four-direction walking, reading, gardening, daydreaming, invitations,
walking toward a conversation, and ongoing conversations with different message
counts. Initial walking paths are supplied by the fixture, outside timing;
subsequent native pathfinding, collision refusal/replanning, invitation handling,
conversation transitions, and agent operation completions run inside timing.

| Residents | Map tiles | Districts | Initial walking | Initial activities | Initial conversations | Walkable occupancy |
|---:|---:|---:|---:|---:|---:|---:|
| 32 | 64 × 48 | 1 | 12 | 12 | 4 | 1.7% |
| 128 | 64 × 48 | 1 | 48 | 48 | 16 | 6.7% |
| 1,000 | 128 × 48 | 2 | 368 | 375 | 125 | 26.2% |
| 10,000 | 256 × 192 | 16 | 3,656 | 3,750 | 1,244 | 32.8% |

Large counts repeat the original map to maintain walkable space. All districts
belong to one game instance, including native global scans; this is not sharding.
Blocked map edges prevent travel between districts. The 10,000-resident fixture
exceeds the production server's 128 × 128 map limit and measures only in-memory
engine capacity.

The shared offline decision policy chooses a nearby destination, activity, or
neighbor invitation using resident ID, seed and simulation time. Dialogue uses
fixed text and memory operations store a fixed summary. Both engines finish
these through their real command/input handlers. No provider, API key, inference
latency, network, database, embedding search or Convex action scheduling is
included. This exercises autonomous town state transitions, not the semantic
quality of LLM-driven personalities.

Original town bundles include one numerical fix: `path-guard.mjs` returns the
segment endpoint with zero velocity when both path timestamps are equal, avoiding
division by zero. Without it, the 128-resident, seed-1 lifecycle stops at 49.184
simulated seconds: a conversation replan goes from `13.999999999999957` to `14`
in zero representable milliseconds. This can occur even for a distant destination.
The guard is regression-tested against that captured path and ordinary
interpolation. It is included in all reported original timings; the original
checkout stays intact. Original mode therefore means upstream Game with this
explicit numerical guard, plus offline operation/transport adapters, rather
than a byte-for-byte unmodified deployment.

## Timing and interpretation

Each sample loads the same fixture outside timing. One warmup precedes three
measured samples per engine/count. The main window is one 60-tick batch:
**0.96 simulated seconds**, at 16 ms per tick. This seeded window includes
walking, activity expiration, invitation handling and conversation operations.
It is a short scaling measurement, not a steady-state simulation of a whole day.
A separate 80-batch run covers **76.8 simulated seconds** and checks the longer
activity → decision → movement/invitation → conversation → memory lifecycle.

`loop_ms` includes native ticks and offline operation completions after each
batch. Rust runs the checked ticks with history capture, then the server's `transition` command boundary
and the kernel's owned `apply_mut` write interpreter, including validation and
row projection/diff costs. The baseline below used immutable `apply`, which
copied the entire snapshot for each operation completion. Original mode calls
`beginStep`, `Game.tick`, then the native operation-finish input handlers.
Both engines record each tick’s confirmed position, facing and speed inside
`loop_ms`. Compression is recorded separately as `history_pack_ms`; add the two
for simulation plus history delivery preparation. Upstream uses native `takeDiff`,
which also serializes the native world; Rust packs its captured tick data.
UI-compatible snapshot JSON serialization, including base64 motion packets, is measured separately
as `snapshot_ms`; both limit displayed message history to the latest 100.
Fixture load, final-state checks and transport/rendering are outside timing.

These are the actual engine implementations, **not identical completed work**.
For example, Rust waits for walking to finish before scheduling another decision;
original AI Town can schedule decisions while wandering. Invitation acceptance,
conversation endings and collision scheduling also differ. The adapters preserve
those rules rather than forcing identical trajectories. CSVs therefore include
actual moved residents, decisions, destinations, activities, invitations,
messages, memories and pathfinding attempts alongside timings. Do not interpret
a timing ratio as a feature-equivalent full-game or deployment speedup.

Rust checks placement after every movement tick; both benchmark drivers also
check final bounds, walkable terrain, player separation and conversation membership
consistency where available. Fixture tests cover unique walkable spawns, legal
axis-aligned paths, all four directions, mixed conversation states and reproducible
seeds.

## Reproduce

```bash
./bench/demo.sh --players 10000
./bench/demo.sh --engine original --players 128
./bench/demo.sh --scenario crowd --players 10000 # old movement-only demo

./bench/town.sh --players 32,128,1000,10000 --samples 3 --out /tmp/town-scaling
./bench/town.sh --players 32,128,10000 --engine rust --samples 3 --steps 80 --out /tmp/town-lifecycle
node --test bench/scenarios/town.test.mjs bench/upstream/path-guard.test.mjs
```

Use `--seed N` to select a reproducible population, `--engine rust|original` to
measure one implementation, and `--ticks 1..60` / `--steps N` to change the window.
`AI_TOWN_UPSTREAM` selects the original checkout (default `~/Ref/ai-town`). The
runner saves CSVs, fixtures and a metadata file with arguments, revisions and
SHA-256 hashes. The demo adapts batch size to keep frames responsive and replays
only recorded positions; its UI compute counter is not the fixed-step
benchmark. Original 10,000-resident measurements can take minutes per sample.

## Rust optimization

The phase profiler (`cargo run --release -p ai-town-bench --bin town-profile --
FIXTURE 60 80`) separates native ticks, offline operation completions, command
transitions and write commits. It also probes whole-world clone/diff costs outside
the timed loop. Ordinary benchmarks disable per-command profiling clocks.

The initial 10,000-resident diagnostic spent 2.79 seconds in ticks and 43.59
seconds completing operations for just 0.96 simulated seconds. Each tiny speech,
finish or memory command cloned and diffed the whole world. Commands now project
the rows their unchanged dispatchers read, and the in-memory owner commits the
resulting writes in place. Move commands still see every other resident's
position and the same terrain. Tests compare exact writes, replies and errors
against the full-snapshot reference, including unsorted rows and error cases.

Agent and conversation ticks now index memberships and conversations once,
rebuilding after removals, with a scan fallback for sparse IDs. Differential tests
compare indexed ticks to scanning through conversation removals. Snapshot output
indexes memberships and selects the last 100 messages and authoritative positions
before allocating JSON, instead of constructing then discarding paths/history.

One scheduling rule changes deliberately: conversation approaches previously
called pathfinding directly, bypassing the existing 16-search batch budget.
An intermediate run still spent 5–7 seconds per 0.96-second batch retrying those
routes. Approaches now join the shared pathfinding queue; failed approaches wait
before retrying. Overflow remains queued and collision/terrain checks remain
active. This changes conversation timing and long-run trajectories; the speedup
is not entirely a semantics-preserving implementation change. The baseline's
pathfind counter also omitted these bypassed searches. Direct MoveTo commands
retain their existing synchronous pathfinding behavior.

These improvements apply to the Rust kernel and owned in-memory demo adapter.
The PostgreSQL service still loads snapshots at its command boundary and enforces
its existing map limits. No 10,000-resident database deployment or LLM throughput
claim follows from these measurements. The UI receives batches of authoritative
positions, rather than 60 position frames per second.

The optimization sections below retain the earlier measurements without Rust
motion history. The new history-enabled comparison appears at the end.

## After optimization — 2026-10-22

Same seed and fixtures as the baseline, three samples after a warmup. Milliseconds,
median (minimum–maximum). Original-engine results below remain the previously
measured native baseline; the original adapter and upstream sources did not change.

| Residents | Baseline Rust loop | Optimized Rust loop | Optimized snapshot |
|---:|---:|---:|---:|
| 32 | 0.832 (0.813–0.835) | 0.570 (0.556–0.590) | 0.375 (0.361–0.389) |
| 128 | 6.063 (5.624–6.562) | 2.028 (1.932–2.571) | 1.464 (1.278–1.878) |
| 1,000 | 287.086 (286.502–289.953) | 24.666 (23.281–24.820) | 13.396 (11.450–15.677) |
| 10,000 | 43,626.161 (43,489.102–44,400.720) | 505.019 (491.017–509.777) | 208.490 (171.133–211.674) |

The short 10,000-resident window improves by 86.4×. All displayed completion counters
and moved-resident counts match the baseline for this short window. It covers
the seeded scene before new decision waves; the longer measurement is necessary
to assess those waves. Snapshot serialization remains a separate CPU cost.

### Optimized lifecycle — 76.8 simulated seconds

| Residents | Loop ms, median (range) | Destinations | Activities | Invitations | Messages | Memories |
|---:|---:|---:|---:|---:|---:|---:|
| 32 | 45.739 (43.303–46.103) | 40 | 28 | 7 | 65 | 18 |
| 128 | 278.616 (276.303–279.136) | 151 | 79 | 71 | 410 | 148 |
| 10,000 | 24,362.906 (24,204.229–24,556.039) | 9,179 | 4,489 | 4,660 | 5,839 | 8,902 |

At 10,000 residents, 76.8 simulated seconds take 24.36 seconds of loop time
(3.15× real-time throughput), plus one final snapshot. Each of the
80 batches drains agent operations; all measured samples pass final placement
and membership checks. There is no matched 10,000-resident baseline lifecycle
timing, so no before/after long-window speedup is claimed. The approach queue
changes long-window completion counts, as explained above. Per-frame transport,
snapshot frequency, browser rendering and database overhead are outside this
fixed-step benchmark. These results cover 76.8 seconds, not an all-day soak.

A separate final phase profile of those 80 batches spent 19.59 seconds in native
ticks and 6.23 seconds draining operations. Its 95th-percentile batch took
0.633 seconds and the slowest took 1.921 seconds, so decision waves can still
exceed the 0.96-second simulated batch. These are single diagnostic samples,
not the three-sample medians above; their raw phase timings are in `profiling/`.

### Live demo check

The final Rust demo served 10,000 residents to the UI without browser/HTTP
errors; overview, neighborhood zoom and selecting a resident were checked.
In a separate roughly one-minute HTTP observation, 58.45 wall-clock seconds
advanced 57.12 simulated seconds, including frame serialization and demo pacing.
The host also ran proof extraction; no rendering FPS claim is made. Adaptive
batches differ from the fixed-step benchmark, and synchronous decision waves
can still produce a longer update. Raw frame counters and source hashes are
saved under `validation/` with the measurement results.

## Further CPU, GPU and UI optimization — 2026-10-23

The next pass reduces exact integer geometry costs: axis-aligned path lengths
avoid square roots, stationary interpolation avoids division, and integer square
root starts near the answer instead of repeatedly halving the input. Random and
boundary tests include `u64::MAX`. GPU calls reuse device allocations across
seek/step/round entry points, growing slots as needed and releasing them before
context destruction. Each call still uploads current input, mask and shape.
Server GPU packing counts and prefix-packs path rows once instead of allocating
a vector per resident and cloning every path twice. The demo caches conversation
and thinking indices per received snapshot instead of rebuilding them every frame.

| 10,000-resident measurement | Previous pass | Current pass |
|---|---:|---:|
| Town, 0.96 simulated seconds, loop median | 505.019 ms | 388.929 ms (358.444–389.404) |
| Town, 76.8 simulated seconds, loop median | 24,362.906 ms | 23,603.723 ms (20,778.876–23,858.506) |
| Eastbound movement only, CPU 60 ticks | 209.950 ms | 106.552 ms |
| Eastbound movement only, GPU input preparation | 62.193 ms | 11.491 ms |
| Eastbound movement only, GPU 60 rounds including copies | 4.351 ms | 2.926 ms |
| Eastbound movement only, preparation + GPU + checked relocation | 70.949 ms | 16.782 ms |

Town rows use one warmup plus three samples; GPU rows use one warmup plus five.
The GPU comparison is a separate movement microbenchmark, not a GPU town engine.
The device algorithm is unchanged; most of the gain is CPU preparation and
allocation reuse. Its final placements are checked against CPU movement.
Head-on contention samples are included in the raw evidence too.
The machines and GPUs are shared and unpinned. In particular, the longer town
measurement is close to the previous pass and does not demonstrate a substantial
additional lifecycle speedup. All short and long town work counters match the
previous optimized pass; the long run still completes 18,328 decisions, 4,489
activities, 4,660 invitations, 5,839 messages and 8,902 memories.

A separate instrumented 4,800-tick run measured 4.64 s of movement (including
position backup), 4.72 s of placement checking, 4.30 s of agent updates,
1.13 s of conversations, and 0.93 s of scheduled pathfinding. Offline operation
completion added 4.90 s, including 4.50 s in MoveTo commands. These totals include
instrumentation overhead and are not benchmark medians. They identify remaining
CPU work without removing the per-tick placement check. Bursts of simultaneous
MoveTo decisions can still pause updates; these results do not establish a frame
latency guarantee or a full-day soak result.

The live 10k UI advanced 48.144 simulated seconds over 48.097 wall seconds in one
cadence check. Browser verification covered overview, neighborhood, resident
selection and activity display with no console errors or failed requests. This
checks demo operation and pacing, not 60 FPS or production database capacity.

[Raw results, source/fixture hashes, profiling patch and validation](../bench/results/2026-10-23/)
record this pass. Use the reproduction commands above for town timings and the
instructions in that directory for GPU timings.

## Before optimization — 2026-10-22

Milliseconds, median (minimum–maximum), three measured samples after one warmup.

| Residents | Rust loop | Original loop | Original history pack (median) | Rust snapshot (median) | Original snapshot (median) |
|---:|---:|---:|---:|---:|---:|
| 32 | 0.832 (0.813–0.835) | 13.570 (8.210–17.798) | 1.845 | 1.007 | 0.213 |
| 128 | 6.063 (5.624–6.562) | 57.664 (57.336–60.996) | 4.055 | 3.557 | 0.450 |
| 1,000 | 287.086 (286.502–289.953) | 1,419.109 (1,376.602–1,551.898) | 14.301 | 29.967 | 4.196 |
| 10,000 | 43,626.161 (43,489.102–44,400.720) | 232,809.971 (225,960.725–236,869.788) | 416.440 | 339.692 | 36.111 |

These are 0.96-second windows. Before optimization, the 10,000-resident case was below real time in
both implementations. In particular, the Rust town workload takes far more time
than the old movement-only numbers suggest. No GPU full-town result is claimed.

Completed work per measured short window (identical across the three samples):

| Engine | Residents | Moved | Decisions | Activities | Invitations | Messages | Memories |
|---|---:|---:|---:|---:|---:|---:|---:|
| original | 1,000 | 384 | 30 | 7 | 8 | 116 | 0 |
| rust | 1,000 | 368 | 0 | 0 | 0 | 192 | 44 |
| original | 10,000 | 3,670 | 320 | 84 | 80 | 1,168 | 0 |
| rust | 10,000 | 3,656 | 0 | 0 | 0 | 1,909 | 440 |

The zero Rust decision count in the first short window reflects the native
15-second decision cooldown seeded at the start; it does not mean agents are
disabled. The longer run below exercises new decisions and activities too.
The original schedules decisions under different conditions and completes a
different amount of conversation and memory work.

### Longer lifecycle — 76.8 simulated seconds

| Engine | Residents | Loop ms, median (range) | History pack ms (median) | Decisions | Activities | Invitations | Messages | Memories |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| original | 32 | 562.512 (545.566–574.640) | 52.712 | 681 | 20 | 9 | 73 | 14 |
| rust | 32 | 44.295 (44.169–44.791) | 0.000 | 75 | 28 | 7 | 65 | 18 |
| original | 128 | 10,853.952 (10,677.595–10,941.832) | 175.268 | 1959 | 61 | 90 | 551 | 121 |
| rust | 128 | 523.129 (491.997–543.941) | 0.000 | 307 | 84 | 72 | 373 | 150 |

All longer samples completed new destinations, activities, invitations, messages
and memories, and passed the drivers’ final placement checks. Raw CSVs also
include pathfinding attempts and moved-resident counts. This baseline long run validates
the smaller populations. The optimized 10,000-resident lifecycle is reported
separately above.

### Environment and evidence

Two AMD EPYC 7502 CPUs, 128 hardware threads; Rust 1.98.1 release build;
Node 24.21.0. The original checkout is unmodified at
`8e05997f2409275669c8344b84a51692e83f3f33`. This repository was based on
`8e60c408361b89e1a536c317af98dad3a321a856` with the new harness
changes; baseline reproduction instructions preserve the original adapter.
Optimized runs also change the kernel and server; their metadata includes
SHA-256 hashes of kernel/server sources, manifests and harness files. No GPU or database was
used. The original bundle includes the numerical path guard documented above.
Engine/count workloads ran sequentially, Rust before original. The host
is shared and unpinned, with other CPU jobs active (starting one-minute load
about 75 across 128 hardware threads). Optimized Rust measurements also ran
alongside Lean extraction on this shared host. These are measurements of this run, not
isolated-hardware performance guarantees.

[Raw CSVs, summaries, fixture hashes and metadata](../bench/results/2026-10-22/)
preserve all measured samples. Regenerate fixtures using the recorded seed and
source hashes; the generated map JSON is not duplicated in the repository.


## Tick history and playback — 2026-10-23

Rust now records actual positions **after** collision settling on every CPU tick,
plus facing and speed. It uses the upstream version-1 `HistoricalObject` packet
format: quantization (8 fractional bits for x/y/dx/dy, 16 for speed), deltas,
run-length encoding and signed varints. Unchanged residents have no packet.
`MotionHistory` stores only the latest completed batch (at most 60 ticks) in the
snapshot and PostgreSQL singleton table. This is a bounded delivery buffer,
not an archive of all past movement. SQL writes and WebSocket/snapshot JSON
carry that same batch. Existing schemas create the new table at startup.

Both town demo adapters deliver these packets. The browser decodes them once per
batch and replays recorded values, including turns, collision refusals and stops.
Its clock holds at the latest completed tick and skips unrecorded gaps. Duplicate
snapshots do not restart playback; reconnecting to an older world resets it.
Decoded frame retention is bounded to eight batches and a short time window,
with playback lag capped at 1.5 simulated seconds. GPU movement-only relocation
has no intermediate tick trace; it clears the previous history rather than
reporting invented samples. These town results use the full CPU game loops.

The comparison uses the same seed-1 fixtures, native ticks, offline operation
completions, 16 ms tick interval, one warmup and three measured samples for each
engine. Both include history recording and compression. `history_bytes` sums
compressed packets plus a normalized 12-byte player-id/length envelope per
packet across all batches; base64 and JSON overhead are excluded from that
column but included in `snapshot_ms`. Snapshot timing encodes only the final
batch, while compression is paid for **every** batch in a lifecycle run.
Neither benchmark includes persistence, socket delivery, browser decoding or
rendering. As before, native scheduling yields different amounts of completed
work: adding history closes that specific feature gap, not every behavioral
difference. Timing ratios remain inappropriate as feature-equivalent speedups.

Verification includes upstream-generated reference vectors, native-decoder
comparison of Rust demo packets, varied upstream packets decoded by the browser,
confirmed-tick/write persistence tests, and browser checks with Rust 10,000 and
original 128 residents. Overview and neighborhood rendered without console or
HTTP errors; playback held at the completed interval end. Browser validation logs accompany the results. The Lean placement/isolation proof
boundary is unchanged; playback and codec equivalence are tested, not formally
proved.

### History-enabled measurements

All values below are milliseconds. Simulation time includes recording **and**
compression, summed for each sample before taking the median. The JSON column
is a separately measured final frame; add it when comparing preparation of one
UI response. These are full CPU town loops; GPU movement results above keep
their separate scope.

| Residents | Rust simulation + history, median (range) | Original simulation + history, median (range) | Rust final JSON | Original final JSON |
|---:|---:|---:|---:|---:|
| 32 | 0.474 (0.465–0.480) | 12.511 (8.073–14.322) | 0.332 | 0.244 |
| 128 | 1.655 (1.655–2.587) | 63.774 (58.710–67.589) | 1.260 | 0.601 |
| 1,000 | 19.631 (19.524–19.676) | 1,423.206 (1,417.518–1,438.355) | 9.788 | 4.875 |
| 10,000 | 385.462 (374.132–388.643) | 214,883.456 (192,686.720–226,240.675) | 128.595 | 38.914 |

Each row above covers **0.96 simulated seconds**. At 10,000 residents, Rust
compression alone takes a median 8.129 ms and the batch contains 387,258 compressed
bytes including envelopes; original compression/native diff takes 528.393 ms
and produces 351,468 bytes. Snapshot generation remains material in Rust
(128.595 ms), even though it is outside the simulation timing.

The **76.8-second lifecycle** measurements include compression of all 80 batches:

| Engine | Residents | Simulation + history, median (range) | Final JSON, median | Compressed bytes across all batches |
|---|---:|---:|---:|---:|
| original | 32 | 564.656 (503.304–564.988) | 0.301 | 77,690 |
| rust | 32 | 37.843 (37.742–38.129) | 0.411 | 57,970 |
| original | 128 | 9,625.317 (9,353.569–9,699.707) | 0.730 | 362,057 |
| rust | 128 | 237.787 (237.085–238.256) | 1.401 | 311,568 |
| rust | 10,000 | 22,436.236 (22,421.407–22,538.843) | 125.375 | 5,333,142 |

Original at 10,000 over the full lifecycle was **not measured**: even the
0.96-second window takes approximately 215 seconds. No long-window result is
extrapolated from that short window.

Completed work in the lifecycle samples (identical across all three samples):

| Engine | Residents | Decisions | Destinations | Activities | Invitations | Messages | Memories |
|---|---:|---:|---:|---:|---:|---:|---:|
| original | 32 | 681 | 54 | 20 | 9 | 73 | 14 |
| rust | 32 | 75 | 40 | 28 | 7 | 65 | 18 |
| original | 128 | 1,959 | 163 | 61 | 90 | 551 | 121 |
| rust | 128 | 301 | 151 | 79 | 71 | 410 | 148 |
| rust | 10,000 | 18,328 | 9,179 | 4,489 | 4,660 | 5,839 | 8,902 |

In the short 10,000-resident window, Rust moves 3,656 residents, completes
1,909 messages and 440 memories, but schedules no new decisions because its
15-second cooldown has not expired. Original moves 3,670 residents, schedules
320 decisions, completes 1,168 messages and no memories in the same window.
These counters explain why the longer lifecycle and the work columns matter.

The original checkout remains clean at
`8e05997f2409275669c8344b84a51692e83f3f33`, with the documented numerical
path guard applied only to its benchmark bundle. Node 24.21.0 and Rust 1.98.1
were used on the shared, unpinned dual-EPYC host described above. Runs were
sequential; the initial short-window run partly overlapped Lean extraction
and compilation. Rust short samples were rerun after the final codec helper
refactor; source hashes distinguish those runs. Do not read host/runtime
variation against earlier tables as an optimization from adding history.

[Raw samples and reproduction](../bench/results/2026-10-23/README.md#tick-history-comparison),
[aggregated results](../bench/results/2026-10-23/history-summary.json), and
[verification logs](../bench/results/2026-10-23/history-validation/) are preserved.
Validation passed: 59 workspace tests (one live LLM test ignored), six GPU tests,
14 GPU-enabled server tests (one live test ignored), 17 Node tests, the frontend
build, and the Lean gate for 91 existing theorems. The optional captured-frame
interop test also checked 711 real Rust packets against the upstream decoder.
Frontend unit tests require Node 22.18+ for native TypeScript stripping. No
PostgreSQL integration, socket throughput or 10,000-resident production server
capacity is asserted by these in-memory measurements.
