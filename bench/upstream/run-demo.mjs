// Bundle upstream Game with a small demo transport adapter.
import { build } from 'esbuild';
import { pathGuard } from './path-guard.mjs';
import { mkdtempSync, rmSync, existsSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

const here = dirname(fileURLToPath(import.meta.url));
const upstream = resolve(process.env.AI_TOWN_UPSTREAM ?? `${homedir()}/Ref/ai-town`);
const game = resolve(upstream, 'convex/aiTown/game.ts');
if (!existsSync(game)) {
  console.error(`Upstream Game not found at ${game}. Set AI_TOWN_UPSTREAM to an AI Town checkout.`);
  process.exit(1);
}
const bundleDir = mkdtempSync(resolve(tmpdir(), 'ai-town-original-demo-'));
try {
  const outfile = resolve(bundleDir, 'demo.cjs');
  await build({
    entryPoints: [resolve(here, 'demo.ts')], outfile,
    bundle: true, platform: 'node', format: 'cjs', target: 'node20',
    alias: { 'upstream-game': game },
    nodePaths: [resolve(here, 'node_modules')],
    external: ['bufferutil', 'utf-8-validate'],
    plugins: process.argv[5] ? [pathGuard(upstream)] : [],
  });
  const child = spawn(process.execPath, [outfile, ...process.argv.slice(2)], { stdio: 'inherit' });
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
  await new Promise((resolve, reject) => {
    child.on('error', reject);
    child.on('exit', (code, signal) => { process.exitCode = code ?? (signal === 'SIGINT' ? 130 : 143); resolve(); });
  });
} finally {
  rmSync(bundleDir, { recursive: true, force: true });
}
