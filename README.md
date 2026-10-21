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
| `proofs/` | Lean 4 postconditions: `dispatch_start` exclusivity and membership, `occupied`, and `own_player`. See `docs/PLAN.md` |
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
(`https://integrate.api.nvidia.com/v1`). With no key, agents use the
deterministic stub. The process never prints the key.

## Tests and proofs

```bash
cargo test
h5i app prove
h5i app mutate
```

`h5i app prove` needs Charon, Aeneas, and Lean v4.31.0 on `PATH`
(`h5i app doctor`).

## Benchmarks

```bash
mkdir -p /dev/shm/ai-town-rust/bench
cargo run --release -p ai-town-bench
```

Results are CSV files under `/dev/shm/ai-town-rust/bench/`. The optional GPU
movement backend is described in `docs/PLAN.md`. The CPU batch step and its
equivalence test are in the kernel.
