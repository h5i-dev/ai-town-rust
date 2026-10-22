#!/usr/bin/env bash
# One-command, disposable CPU movement demo. Ctrl-C stops both processes.
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
players=10000
ui_port=5174
api_port=18080
open_browser=1
usage() {
  cat <<'HELP'
Usage: bench/demo.sh [--players N] [--port PORT] [--api-port PORT] [--no-open]

Start the movement demo and UI, then open the browser. No database or LLM API.
  --players N      Number of players, 1..14400 (default: 10000)
  --port PORT      UI port (default: 5174)
  --api-port PORT  Demo backend port (default: 18080)
  --no-open        Print the URL without opening a browser
  -h, --help       Show this help

Requires Rust/Cargo, Node.js/npm, curl, and Python 3. First run builds the
backend and installs frontend dependencies. Ctrl-C cleans up both servers.
HELP
}
while (($#)); do
  case "$1" in
    --players|--port|--api-port)
      if (($# < 2)); then usage >&2; exit 2; fi
      case "$1" in --players) players=$2;; --port) ui_port=$2;; --api-port) api_port=$2;; esac
      shift 2;;
    --no-open) open_browser=0; shift;;
    -h|--help) usage; exit 0;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2;;
  esac
done
for tool in cargo node npm curl python3; do
  command -v "$tool" >/dev/null || { echo "Missing dependency: $tool" >&2; exit 1; }
done
# Validate before building. Binding later still detects any intervening port use.
python3 - "$players" "$ui_port" "$api_port" <<'PY'
import socket, sys
values = sys.argv[1:]
if not all(v.isascii() and v.isdecimal() for v in values):
    sys.exit('Player count and ports must be positive integers')
n, ui, api = map(int, values)
if not 1 <= n <= 14400:
    sys.exit('--players must be between 1 and 14400')
if not all(1 <= p <= 65535 for p in (ui, api)) or ui == api:
    sys.exit('Choose two different ports between 1 and 65535')
for port in (ui, api):
    with socket.socket() as sock:
        try:
            sock.bind(('127.0.0.1', port))
        except OSError:
            sys.exit(f'Port {port} is in use; choose --port / --api-port')
PY
cd "$repo_dir"
# A persistent temporary build cache avoids filling a quota-limited checkout.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/ai-town-rust-demo-${UID}-target}"
echo "Building demo backend…"
cargo build --locked --release -p ai-town-bench --bin demo
if [[ ! -x frontend/node_modules/.bin/vite ]]; then
  npm --prefix frontend ci --no-audit --no-fund
fi
log_dir="$(mktemp -d "${TMPDIR:-/tmp}/ai-town-demo.XXXXXX")"
pids=()
cleanup() {
  trap - EXIT INT TERM
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
  echo "Demo stopped. Logs: $log_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
"$CARGO_TARGET_DIR/release/demo" "$players" "127.0.0.1:$api_port" >"$log_dir/backend.log" 2>&1 &
pids+=("$!")
(cd frontend && AI_TOWN_API_URL="http://127.0.0.1:$api_port" exec node node_modules/vite/bin/vite.js --host 127.0.0.1 --port "$ui_port" --strictPort) >"$log_dir/ui.log" 2>&1 &
pids+=("$!")
url="http://127.0.0.1:$ui_port/ai-town/"
ready=0
for ((attempt=0; attempt<100; attempt++)); do
  for pid in "${pids[@]}"; do
    if ! kill -0 "$pid" 2>/dev/null; then cat "$log_dir/"*.log >&2; exit 1; fi
  done
  if curl -fsS --max-time 1 "http://127.0.0.1:$ui_port/healthz" >/dev/null 2>&1 && curl -fsS --max-time 1 "$url" >/dev/null 2>&1; then ready=1; break; fi
  sleep 0.2
done
if (( ! ready )); then echo "Startup timed out" >&2; cat "$log_dir/"*.log >&2; exit 1; fi
printf '\nDemo ready: %s players, no LLM / no database\n%s\nLogs: %s\nCtrl-C to stop.\n' "$players" "$url" "$log_dir"
if ((open_browser)); then
  if command -v xdg-open >/dev/null && [[ -n ${DISPLAY:-}${WAYLAND_DISPLAY:-} ]]; then
    xdg-open "$url" >"$log_dir/browser.log" 2>&1 &
  elif command -v open >/dev/null; then
    open "$url" >"$log_dir/browser.log" 2>&1 &
  fi
fi
# Return if either server fails, and clean up the remaining one.
wait -n "${pids[@]}"
