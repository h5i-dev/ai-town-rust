use ai_town_bench::Town;
use ai_town_kernel::{members_consistent, placement_ok};
use std::{fs, time::Instant};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let fixture = args
        .get(1)
        .expect("town-bench FIXTURE OUTPUT [SAMPLES] [TICKS] [STEPS]");
    let output = args.get(2).expect("output CSV");
    let samples: u32 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(3);
    let ticks: u64 = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(60);
    let steps: u64 = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(1);
    assert!(samples > 0 && ticks > 0 && ticks <= 60 && steps > 0);
    fs::write(output,"engine,players,sample,ticks,simulated_ms,loop_ms,history_pack_ms,snapshot_ms,moved,decisions,destinations,activities,invitations,messages,memories,pathfinds,history_bytes\n").unwrap();
    for sample in -1..samples as i32 {
        let mut town = Town::load(fixture);
        let before = town.state.players.clone();
        let mut elapsed = 0.0;
        for _ in 0..steps { elapsed += town.step(ticks); }
        assert!(placement_ok(&town.state));
        assert!(members_consistent(&town.state));
        let moved = town
            .state
            .players
            .iter()
            .zip(&before)
            .filter(|(a, b)| a.x != b.x || a.y != b.y)
            .count();
        let clock = Instant::now();
        let frame = town.snapshot(elapsed, ticks * steps * 16);
        std::hint::black_box(frame);
        let snapshot_ms = clock.elapsed().as_secs_f64() * 1000.0;
        let m = &town.metrics;
        let row = format!(
            "rust,{},{sample},{},{},{elapsed:.3},{:.3},{snapshot_ms:.3},{moved},{},{},{},{},{},{},{},{}\n",
            before.len(),
            ticks * steps,
            ticks * steps * 16,
            town.history_pack_ms,
            m.decisions,
            m.destinations,
            m.activities,
            m.invitations,
            m.messages,
            m.memories,
            m.pathfinds,
            town.history_bytes
        );
        eprint!("{row}");
        if sample >= 0 {
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(output)
                .unwrap()
                .write_all(row.as_bytes())
                .unwrap();
        }
    }
}
