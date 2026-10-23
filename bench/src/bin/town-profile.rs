//! Wall-clock phase profiler; works on hosts that disable perf events.
use ai_town_bench::Town;
use ai_town_kernel::{apply, diff, run_ticks, transition, Command, Principal};
use serde_json::json;
use std::{hint::black_box, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mut town = Town::load(args.get(1).expect("town-profile FIXTURE [TICKS] [STEPS]"));
    town.command_ms = Some([0.; 6]);
    let ticks: u64 = args.get(2).map(|v| v.parse().unwrap()).unwrap_or(60);
    let steps: usize = args.get(3).map(|v| v.parse().unwrap()).unwrap_or(1);
    assert!((1..=60).contains(&ticks) && steps > 0);
    let mut tick_ms = 0.;
    let mut drain_ms = 0.;
    let mut tick_samples = Vec::new();
    let mut drain_samples = Vec::new();
    for step in 0..steps {
        let start = Instant::now();
        let end = town.state.meta.now_ms + ticks * 16;
        run_ticks(&mut town.state, end);
        town.metrics.pathfinds += town.state.meta.pathfinds as u64;
        let tick = start.elapsed().as_secs_f64() * 1000.;
        tick_ms += tick;
        let start = Instant::now();
        town.drain();
        let drain = start.elapsed().as_secs_f64() * 1000.;
        drain_ms += drain;
        tick_samples.push(tick);
        drain_samples.push(drain);
        eprintln!("step={step} ticks_ms={tick:.3} drain_ms={drain:.3} command_cumulative_ms={:?}", town.command_ms.unwrap());
    }
    // Diagnostic probes are outside the measured game loop. An unchanged
    // whole-world diff exposes scan cost without conflating command behavior.
    let start = Instant::now();
    black_box(town.state.clone());
    let clone_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    black_box(diff(&town.state, &town.state));
    let diff_ms = start.elapsed().as_secs_f64() * 1000.;
    let actor = Principal { org: 1, user: town.state.agents[0].owner, now_ms: town.state.meta.now_ms };
    let start = Instant::now();
    let (writes, _) = transition(&actor, &town.state, &Command::FinishOp).unwrap();
    let finish_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    black_box(apply(&town.state, &writes));
    let apply_ms = start.elapsed().as_secs_f64() * 1000.;
    println!("{}", json!({"players":town.state.players.len(),"simulated_ms":ticks * 16 * steps as u64,
        "ticks_ms":tick_ms,"drain_ms":drain_ms,"clone_probe_ms":clone_ms,"unchanged_diff_probe_ms":diff_ms,
        "finish_transition_probe_ms":finish_ms,"apply_probe_ms":apply_ms,"metrics":town.metrics.json(),
        "tick_samples_ms":tick_samples,"drain_samples_ms":drain_samples,"command_cumulative_ms":town.command_ms}));
}
