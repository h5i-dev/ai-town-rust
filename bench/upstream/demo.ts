// Only the fixture, JSON adapter, and local transport live here. All movement
// agent/conversation rules and history come from upstream Game. Town bundles
// guard zero-duration path interpolation (see path-guard.mjs).
import { Game } from 'upstream-game';
import { Town } from './town';
import { isMainThread, parentPort, workerData, Worker } from 'node:worker_threads';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { WebSocket, WebSocketServer } from 'ws';

const TICK_MS = 16;
const LEG_MS = 3840;
const START = 1_000_000;

function createGame(n: number) {
  const columns = Math.ceil(Math.sqrt(n));
  const side = Math.max(16, columns + 8);
  const players = Array.from({ length: n }, (_, i) => ({
    id: `p:${i + 1}`, position: { x: 4 + i % columns, y: 4 + Math.floor(i / columns) },
    facing: { dx: 1, dy: 0 }, speed: 0, lastInput: START,
  }));
  return new Game({ _id: 'demo', currentTime: START } as any, 'demo' as any, {
    world: { nextId: n + 1, players, conversations: [], agents: [] },
    worldMap: { width: side, height: side, tileSetUrl: '', tileSetDimX: 0,
      tileSetDimY: 0, tileDim: 32, bgTiles: [], animatedSprites: [],
      objectTiles: [Array.from({ length: side }, () => Array(side).fill(-1))] },
    playerDescriptions: [], agentDescriptions: [],
  } as any);
}

function route(game: Game, now: number, east: boolean) {
  for (const player of game.world.players.values()) {
    const { x, y } = player.position;
    const dx = east ? 1 : -1;
    const destX = x + dx * 2.88;
    player.facing = { dx, dy: 0 };
    player.pathfinding = { destination: { x: destX, y }, started: now,
      state: { kind: 'moving', path: [[x, y, dx, 0, now], [destX, y, dx, 0, now + LEG_MS]] } };
  }
}

function snapshot(game: Game, now: number, tickMs: number, stepMs: number) {
  return JSON.stringify({
    nowMs: now, width: game.worldMap.width, height: game.worldMap.height, frozen: false,
    demo: { engine: 'original', tickMs, stepMs },
    players: [...game.world.players.values()].map((p, i) => ({
      id: i + 1, owner: 0, human: false, name: `Player ${i + 1}`, character: i % 8,
      x: Math.round(p.position.x * 1000), y: Math.round(p.position.y * 1000),
      faceDx: Math.round((p.facing.dx + 1) * 1000), faceDy: Math.round((p.facing.dy + 1) * 1000),
      speed: Math.round(p.speed * 1000), destX: Math.round((p.pathfinding?.destination.x ?? p.position.x) * 1000),
      destY: Math.round(p.position.y * 1000), pf: p.pathfinding ? 3 : 0,
      activity: 0, activityUntil: 0,
      // Render only computed positions. Extrapolating the full route would
      // make a slow original engine appear to move at real-time speed.
      path: [],
    })),
    agents: [], conversations: [], messages: [],
  });
}

if (!isMainThread) {
  if (workerData.fixture) {
    console.log = console.debug = console.warn = () => {};
    const town = new Town(workerData.fixture);
    let now = START, batch = 1;
    parentPort!.postMessage(JSON.stringify(town.snapshot(now)));
    const step = () => {
      const started = performance.now();
      const result = town.step(now, batch);
      now += batch * TICK_MS;
      const elapsed = result.tickMs + result.packMs;
      parentPort!.postMessage(JSON.stringify(town.snapshot(now, elapsed, batch*TICK_MS)));
      const delay = Math.max(0,batch*TICK_MS-(performance.now()-started));
      batch = Math.max(1,Math.min(60,Math.floor(500/Math.max(elapsed/batch,0.1))));
      setTimeout(step,delay);
    };
    setTimeout(step,0);
  } else {
    const game = createGame(workerData.players);
    let now = START;
    let east = true;
    let turnAt = now + LEG_MS;
    route(game, now, east);
    // Keep initial rendering instant even when the first tick takes seconds.
    parentPort!.postMessage(snapshot(game, now, 0, 0));
    let batch = 1;
    const step = () => {
      const ticks = Math.min(batch, (turnAt - now) / TICK_MS);
      const started = performance.now();
      game.beginStep(now + TICK_MS);
      for (let i = 0; i < ticks; i++) { now += TICK_MS; game.tick(now); }
      game.takeDiff(); // Pack and release upstream's real location history.
      if (now >= turnAt) { east = !east; route(game, now, east); turnAt = now + LEG_MS; }
      const elapsed = performance.now() - started;
      parentPort!.postMessage(snapshot(game, now, elapsed, ticks * TICK_MS));
      console.error(`demo: engine=original players=${workerData.players} ticks=${ticks} tick_ms=${elapsed.toFixed(1)} now_ms=${now}`);
      // Bound update latency: do one tick for large worlds; batch small worlds.
      batch = Math.max(1, Math.min(60, Math.floor(500 / Math.max(elapsed / ticks, 0.1))));
      setTimeout(step, Math.max(0, ticks * TICK_MS - (performance.now() - started)));
    };
    setTimeout(step, 0);
  }
} else {
  const players = Number(process.argv[2] ?? 10_000);
  const port = Number(process.argv[3] ?? 18080);
  if (!Number.isInteger(players) || players < 1 || players > 14_400
      || !Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Invalid demo count or port');
  const side = Math.max(16, Math.ceil(Math.sqrt(players)) + 8);
  const fixture = process.argv[5] || undefined;
  const map = fixture ? JSON.parse(readFileSync(fixture,'utf8')).map : JSON.parse(readFileSync(process.argv[4], 'utf8'));
  if (!fixture) Object.assign(map, { width: side, height: side, demo: true, demoEngine: 'original',
    bgTiles: [Array.from({ length: side }, () => Array(side).fill(271))], objectTiles: [], animatedSprites: [] });
  map.demoEngine = 'original';
  const mapJson = JSON.stringify(map);
  let latest: string | undefined;
  const server = createServer((req, res) => {
    if (req.url === '/healthz') { res.writeHead(latest ? 200 : 503); res.end(latest ? 'ok' : 'starting'); }
    else if (req.url === '/snapshot' && latest) { res.setHeader('content-type', 'application/json'); res.end(latest); }
    else if (req.url === '/map') { res.setHeader('content-type', 'application/json'); res.end(mapJson); }
    else { res.writeHead(404); res.end(); }
  });
  const sockets = new WebSocketServer({ server, path: '/ws' });
  sockets.on('connection', socket => { if (latest) socket.send(latest); });
  const worker = new Worker(__filename, { workerData: { players, fixture } });
  worker.on('message', (frame: string) => {
    latest = frame;
    for (const socket of sockets.clients) {
      if (socket.readyState === WebSocket.OPEN) {
        // Bound memory for a viewer whose connection cannot keep up.
        if (socket.bufferedAmount > 16 * 1024 * 1024) socket.terminate();
        else socket.send(frame);
      }
    }
  });
  worker.on('error', err => { console.error(err); process.exit(1); });
  worker.on('exit', code => { if (code !== 0) process.exit(code); });
  server.on('error', err => { console.error(err); process.exit(1); });
  server.listen(port, '127.0.0.1', () => console.error(`Original AI Town ${fixture ? 'town' : 'movement'} demo on port ${port}`));
}
