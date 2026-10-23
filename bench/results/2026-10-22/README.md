# Town measurements, 2026-10-22

`scaling/` and `lifecycle/` contain the pre-optimization Rust and original-engine
measurements. `optimized-scaling/` and `optimized-lifecycle/` contain the final
Rust measurements. CSVs contain three measured samples after one unrecorded
warmup; fixtures are reproducible from the seed and recorded SHA-256 hashes.
`summary.json` preserves baseline summaries; `optimized-summary.json` contains
the optimized summaries. See ../../../docs/TOWN_BENCHMARKS.md for conditions.

The baseline kernel and server are from commit
`8e60c408361b89e1a536c317af98dad3a321a856`. The baseline town adapter was not yet
committed when measured. To reproduce it, use a **separate checkout of the final
implementation**, then run:

```bash
git restore --source=8e60c408361b89e1a536c317af98dad3a321a856 -- kernel/src server/src
git apply bench/results/2026-10-22/baseline-adapter.patch
./bench/town.sh --out /tmp/town-baseline
./bench/town.sh --players 32,128 --steps 80 --out /tmp/town-baseline-lifecycle
```

Do not apply this to a working tree with changes to keep. Build only `town-bench`
(as town.sh does), since the new optional profiler expects the optimized adapter.
Check the baseline metadata source hashes before comparing timings.

`profiling/` preserves single-run diagnostic phase timings from intermediate
optimization stages. These are not three-sample benchmark results. The before
profile covers 0.96 simulated seconds; the queued-approach profile covers 76.8
seconds, so their totals are not directly comparable. The profiler adds clock
reads around commands; town-bench leaves that instrumentation disabled.

The intermediate diagnostic JSONs predate the profiler counter fix and count
only direct destination commands in `metrics.pathfinds`; use their phase timings,
not that counter, when examining budgeted pathfinding work. Benchmark CSVs include
the batch counter as well.
