// A reproducible town workload shared by both engines and both entry points.
// Only the initial state and offline operation policy live here; engine ticks
// perform pathfinding, collision checks, invitations and conversation changes.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { START, hash } from './policy.mjs';
export function makeTown(n, seed = 1) {
  if (!Number.isInteger(n) || n < 1 || n > 14400) throw Error('players must be 1..14400');
  const source = JSON.parse(readFileSync(new URL('../../assets/gentle.json', import.meta.url)));
  // Enlarge the town rather than crowding 10,000 people onto the original map.
  // All districts share one game instance, not independently stepped shards.
  const copies = Math.ceil(n / 800), cols = Math.ceil(Math.sqrt(copies)), rows = Math.ceil(copies / cols);
  const width = cols * source.width, height = rows * source.height;
  const map = { ...source, width, height, demo: true, demoScenario: 'town',
    bgTiles: source.bgTiles.map(layer => Array.from({ length: width }, (_, x) =>
      Array.from({ length: height }, (_, y) => layer[x % source.width][y % source.height]))),
    objectTiles: source.objectTiles.map(layer => Array.from({ length: width }, (_, x) =>
      Array.from({ length: height }, (_, y) => layer[x % source.width][y % source.height]))),
    animatedSprites: Array.from({ length: cols * rows }, (_, i) => source.animatedSprites.map(s =>
      ({ ...s, x: s.x + (i % cols) * source.width * source.tileDim,
        y: s.y + Math.floor(i / cols) * source.height * source.tileDim }))).flat(),
  };
  const blocked = (x, y) => x < 0 || y < 0 || x >= width || y >= height || map.objectTiles.some(l => l[x][y] !== -1);
  // Remove tiny isolated walkable components such as lake/island pixels.
  const seen = new Set(), components = [];
  for (let x = 0; x < width; x++) for (let y = 0; y < height; y++) {
    const key = x * height + y;
    if (blocked(x, y) || seen.has(key)) continue;
    const queue = [[x, y]]; seen.add(key);
    for (let i = 0; i < queue.length; i++) {
      const [cx, cy] = queue[i];
      for (const [nx, ny] of [[cx + 1, cy], [cx - 1, cy], [cx, cy + 1], [cx, cy - 1]]) {
        const k = nx * height + ny;
        if (!blocked(nx, ny) && !seen.has(k)) { seen.add(k); queue.push([nx, ny]); }
      }
    }
    components.push(queue);
  }
  // Repeated original maps have blocked edges, so each copy is a district.
  // Population is distributed over districts, while the engine still sees
  // every player (including its global scans). No cross-district routing.
  const free = components.filter(c => c.length > 100).flat();
  free.sort((a, b) => hash(a[0] * height + a[1] + seed) - hash(b[0] * height + b[1] + seed));
  const freeSet = new Set(free.map(([x,y]) => x * height + y));
  if (free.length < n) throw Error(`Town has only ${free.length} spawn tiles`);
  const occupied = new Set(), positions = [];
  // Every eighth pair starts within talking distance, elsewhere positions
  // are shuffled. This avoids an artificial all-idle startup stampede.
  for (const cell of free) {
    if (positions.length >= n) break;
    const [x, y] = cell, key = x * height + y;
    if (occupied.has(key)) continue;
    positions.push(cell); occupied.add(key);
    if (positions.length % 8 === 1 && positions.length < n) {
      const neighbor = [[x+1,y],[x,y+1],[x-1,y],[x,y-1]].find(([nx,ny]) => nx>=0 && ny>=0 && nx<width && ny<height && freeSet.has(nx*height+ny) && !occupied.has(nx*height+ny));
      if (neighbor) { positions.push(neighbor); occupied.add(neighbor[0]*height+neighbor[1]); }
    }
  }
  const players = positions.map(([x,y], i) => {
    const role = i % 8;
    const activity = role === 2 ? 1 : role === 3 ? 2 : role === 7 ? 3 : 0;
    const path = [];
    if (!activity && role >= 4) {
      let cx = x, cy = y;
      const visited = new Set([cx * height + cy]);
      path.push([cx * 1000, cy * 1000]);
      for (let step = 0; step < 8; step++) {
        const choices = [[cx+1,cy],[cx,cy+1],[cx-1,cy],[cx,cy-1]]
          .filter(([nx,ny]) => nx>=0 && ny>=0 && nx<width && ny<height && freeSet.has(nx*height+ny) && !occupied.has(nx*height+ny) && !visited.has(nx*height+ny));
        if (!choices.length) break;
        [cx,cy] = choices[hash(i * 97 + step + seed) % choices.length];
        visited.add(cx*height+cy); path.push([cx*1000,cy*1000]);
      }
    }
    return { id: i + 1, x: x * 1000, y: y * 1000, activity,
      until: activity ? START + 16 + hash(i + seed) % 60000 : 0, path };
  });
  const conversations = [];
  for (let i = 0; i + 1 < n; i += 8) {
    const p = players[i], q = players[i+1];
    if (Math.hypot(p.x-q.x,p.y-q.y) > 1300) continue;
    const phase = Math.floor(i/8) % 3;
    conversations.push({ id: n * 2 + conversations.length + 1, a: p.id, b: q.id,
      status: phase === 0 ? 3 : phase === 1 ? 1 : 2, messages: Math.floor(i/8) % 9 });
  }
  return { version: 1, seed, now: START, map, players, conversations,
    policy: 'offline-town-v1', density: n / free.length, districts: cols * rows };
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const output = process.argv[3];
  if (!output) throw Error('Usage: node bench/scenarios/town.mjs PLAYERS OUTPUT [SEED]');
  writeFileSync(output, JSON.stringify(makeTown(Number(process.argv[2]), Number(process.argv[4] ?? 1))));
}
