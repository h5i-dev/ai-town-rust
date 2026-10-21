import { Game } from 'upstream-game';
import assert from 'node:assert/strict';
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';

// Same positions, paths, map sizes and timestamps as fair-bench's world().
const now = 1_000_000;
const sizes = (process.env.AI_TOWN_BENCH_AGENTS ?? '1000,10000').split(',').map(Number);
const samples = Number(process.env.AI_TOWN_BENCH_SAMPLES ?? '5');
const scenario = process.env.AI_TOWN_SCENARIO ?? 'east';
assert(['east', 'headon'].includes(scenario));
assert(Number.isInteger(samples) && samples > 0);
const dir = process.env.AI_TOWN_BENCH_DIR ?? '/dev/shm/ai-town-rust/bench';
mkdirSync(dir, { recursive: true });
const output = `${dir}/upstream-${scenario}.csv`;
writeFileSync(output, 'players,sample,ticks,tick_ms,history_pack_ms,moved,warnings\n');

// Count upstream collision/repath warnings without timing terminal I/O.
let warnings = 0;
console.warn = () => { warnings++; };
console.debug = () => {};
console.log = () => {};
// Reproducible backoff when head-on walkers collide; reset for each sample.
let seed = 1;
Math.random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 2 ** 32; };

function world(n: number) {
  let side = 48;
  while (side * side < (scenario === 'headon' ? Math.floor(n * 3 / 2) : n)) side *= 2;
  const players = Array.from({ length: n }, (_, i) => {
    const perRow = Math.floor(side * 2 / 3);
    const col = i % perRow;
    const x = scenario === 'east' ? i % side : 1 + col * 1.5;
    const y = Math.floor(i / (scenario === 'east' ? side : perRow));
    const destX = scenario === 'east' || col % 2 === 0 ? x + 3 : x - 1.5;
    return {
      id: `p:${i + 1}`, position: { x, y }, facing: { dx: 1, dy: 0 }, speed: 0, lastInput: now,
      pathfinding: { destination: { x: destX, y }, started: now,
        state: { kind: 'moving', path: [[x, y, 1, 0, now], [destX, y, 1, 0, now + 4000]] } },
    };
  });
  return new Game({ _id: 'bench', currentTime: now } as any, 'bench' as any, {
    world: { nextId: n + 1, players, conversations: [], agents: [] },
    worldMap: { width: side, height: side, tileSetUrl: '', tileSetDimX: 0,
      tileSetDimY: 0, tileDim: 32, bgTiles: [], animatedSprites: [],
      objectTiles: [Array.from({ length: side }, () => Array(side).fill(-1))] },
    playerDescriptions: [], agentDescriptions: [],
  } as any);
}

for (const n of sizes) {
  assert(Number.isInteger(n) && n > 0);
  for (let sample = -1; sample < samples; sample++) {
    const game = world(n);
    const before = [...game.world.players.values()].map(p => ({ ...p.position }));
    seed = 1;
    warnings = 0;
    const started = performance.now();
    game.beginStep(now + 16);
    for (let tick = 1; tick <= 60; tick++) game.tick(now + tick * 16);
    const tickMs = performance.now() - started;
    const packStarted = performance.now();
    game.takeDiff();
    const packMs = performance.now() - packStarted;
    let moved = 0;
    let i = 0;
    for (const p of game.world.players.values()) {
      if (p.position.x !== before[i].x || p.position.y !== before[i].y) moved++;
      if (scenario === 'east') {
        assert(Math.abs(p.position.x - before[i].x - 0.72) < 1e-8);
        assert.equal(p.position.y, before[i].y);
      }
      assert(p.position.x >= 0 && p.position.x < game.worldMap.width);
      assert(p.position.y >= 0 && p.position.y < game.worldMap.height);
      i++;
    }
    assert.equal(i, n);
    assert(moved > 0);
    if (sample >= 0) {
      appendFileSync(output, `${n},${sample},60,${tickMs.toFixed(3)},${packMs.toFixed(3)},${moved},${warnings}\n`);
      process.stdout.write(`upstream ${scenario} players=${n} sample=${sample} ticks_ms=${tickMs.toFixed(3)} pack_ms=${packMs.toFixed(3)}\n`);
    }
  }
}
