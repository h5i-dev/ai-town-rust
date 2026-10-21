// Bundle the checkout's actual Game class, without modifying upstream sources.
import { build } from 'esbuild';
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const invocationDir = process.env.INIT_CWD ?? process.cwd();
const upstream = resolve(invocationDir, process.env.AI_TOWN_UPSTREAM ?? `${homedir()}/Ref/ai-town`);
const dir = resolve(invocationDir, process.env.AI_TOWN_BENCH_DIR ?? '/dev/shm/ai-town-rust/bench');
process.env.AI_TOWN_BENCH_DIR = dir;
mkdirSync(dir, { recursive: true });
const bundleDir = mkdtempSync(resolve(tmpdir(), 'ai-town-upstream-'));
const outfile = resolve(bundleDir, 'bench.mjs');
try {
  await build({
    entryPoints: [resolve(here, 'ticks.ts')], outfile,
    bundle: true, platform: 'node', format: 'esm', target: 'node20',
    alias: { 'upstream-game': resolve(upstream, 'convex/aiTown/game.ts') },
    nodePaths: [resolve(here, 'node_modules')],
  });
  await import(pathToFileURL(outfile));
} finally {
  rmSync(bundleDir, { recursive: true });
}
