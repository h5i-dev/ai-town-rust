#!/usr/bin/env bash
# Full offline town game loop; shares fixtures and adapters with demo.sh.
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
counts=32,128,1000,10000
samples=3
ticks=60
steps=1
seed=1
engine=both
out=""
usage() {
  cat <<'HELP'
Usage: bench/town.sh [--players 32,128,1000,10000] [--samples 3] [--ticks 60]
                     [--steps 1] [--seed 1] [--engine both|rust|original] [--out DIR]

Offline town benchmark: terrain, independent routes, activities, invitations,
conversations, messages and memory completions. No API key, LLM, or database.
Each sample starts from the same seeded fixture. One warmup precedes samples.
--ticks 1..60 per step; --steps repeats steps and drains agent operations after
 each step. E.g. --players 32 --steps 80 covers 76.8 simulated seconds.
Large original-engine runs can take minutes per sample. CSVs include actual
work completed: engines have different scheduling and collision behavior.
Original checkout: AI_TOWN_UPSTREAM (default ~/Ref/ai-town). Node.js 20+,
Python 3, npm; Rust engine additionally needs Cargo.
HELP
}
while (($#)); do
  case "$1" in
    --players|--samples|--ticks|--steps|--seed|--engine|--out)
      if (($#<2));then usage >&2;exit 2;fi
      case "$1" in --players) counts=$2;;--samples) samples=$2;;--ticks) ticks=$2;;--steps) steps=$2;;--seed) seed=$2;;--engine) engine=$2;;--out) out=$2;;esac
      shift 2;;
    -h|--help)usage;exit 0;;
    *)usage >&2;exit 2;;
  esac
done
case "$engine" in both|rust|original);;*)echo 'Invalid --engine' >&2;exit 2;;esac
python3 - "$counts" "$samples" "$ticks" "$steps" "$seed" <<'PY'
import sys
counts, samples, ticks, steps, seed=sys.argv[1:]
values=counts.split(',')+[samples,ticks,steps,seed]
if not all(v.isascii() and v.isdecimal() for v in values):sys.exit('Arguments must be unsigned integers')
if not all(1<=int(n)<=14400 for n in counts.split(',')):sys.exit('Counts must be 1..14400')
if len(set(counts.split(',')))!=len(counts.split(',')):sys.exit('Duplicate counts')
if not (int(samples)>0 and 1<=int(ticks)<=60 and int(steps)>0 and int(seed)<=4294967295):sys.exit('Invalid sample/tick/step/seed range')
PY
cd "$repo_dir"
if [[ $engine != rust ]];then
  upstream="${AI_TOWN_UPSTREAM:-$HOME/Ref/ai-town}"
  test -f "$upstream/convex/aiTown/game.ts" || { echo 'Set AI_TOWN_UPSTREAM to an original AI Town checkout' >&2;exit 1; }
  if [[ ! -x bench/upstream/node_modules/.bin/esbuild ]];then npm --prefix bench/upstream ci --ignore-scripts --no-audit --no-fund;fi
fi
if [[ $engine != original ]];then
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/ai-town-rust-demo-${UID}-target}"
  cargo build --locked --release -p ai-town-bench --bin town-bench
fi
if [[ -z $out ]];then out="${TMPDIR:-/tmp}/ai-town-town-$(date -u +%Y%m%dT%H%M%SZ)";fi
mkdir -p "$out"
out="$(cd -- "$out" && pwd)"
python3 - "$out" "$counts" "$samples" "$ticks" "$steps" "$seed" "$engine" <<'PY'
import hashlib,json,os,pathlib,platform,subprocess,sys
out,counts,samples,ticks,steps,seed,engine=sys.argv[1:]
root=pathlib.Path.cwd()
paths=[*root.glob('bench/scenarios/*.mjs'),*root.glob('bench/upstream/*town*'),root/'bench/upstream/path-guard.mjs',root/'bench/src/lib.rs',root/'bench/src/bin/town-bench.rs',root/'assets/gentle.json']
paths += [*root.glob('kernel/src/**/*.rs'), *root.glob('server/src/**/*.rs'), root/'Cargo.toml', root/'Cargo.lock', root/'kernel/Cargo.toml', root/'server/Cargo.toml', root/'bench/Cargo.toml', root/'bench/src/bin/town-profile.rs', root/'bench/town.sh']
def git(*args):return subprocess.check_output(['git',*args],text=True).strip()
meta={'utc':subprocess.check_output(['date','-u','+%FT%TZ'],text=True).strip(),'platform':platform.platform(),
 'cpu':next((l.split(':',1)[1].strip() for l in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if l.startswith('model name')),'unknown'),
 'commit':git('rev-parse','HEAD'),'dirty':bool(git('status','--porcelain')),
 'upstream_commit':git('-C',os.environ.get('AI_TOWN_UPSTREAM',os.path.expanduser('~/Ref/ai-town')),'rev-parse','HEAD') if engine!='rust' else None,
 'node':subprocess.check_output(['node','--version'],text=True).strip(),'players':counts,'samples':int(samples),'ticks_per_step':int(ticks),'steps':int(steps),'seed':int(seed),'engine':engine,
 'rust':subprocess.check_output(['rustc','--version'],text=True).strip() if engine!='original' else None,
 'cpu_threads':os.cpu_count(),'loadavg':os.getloadavg(),'original_path_guard':engine!='rust',
 'sha256':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths if p.is_file()}}
pathlib.Path(out,'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
PY
IFS=',' read -r -a sizes <<< "$counts"
for n in "${sizes[@]}";do
  fixture="$out/town-$n.json"
  node bench/scenarios/town.mjs "$n" "$fixture" "$seed"
  if [[ $engine != original ]];then "$CARGO_TARGET_DIR/release/town-bench" "$fixture" "$out/rust-$n.csv" "$samples" "$ticks" "$steps";fi
  if [[ $engine != rust ]];then node bench/upstream/run-town-bench.mjs "$fixture" "$out/original-$n.csv" "$samples" "$ticks" "$steps";fi
done
printf 'Town benchmark complete: %s\n' "$out"
