import { build } from 'esbuild';
import { pathGuard } from './path-guard.mjs';
import { mkdtempSync, rmSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const here=dirname(fileURLToPath(import.meta.url));
const upstream=resolve(process.env.AI_TOWN_UPSTREAM??`${homedir()}/Ref/ai-town`);
const dir=mkdtempSync(resolve(tmpdir(),'ai-town-life-bench-'));
try {
  const outfile=resolve(dir,'bench.mjs');
  await build({entryPoints:[resolve(here,'town-bench.ts')],outfile,bundle:true,platform:'node',format:'esm',target:'node20',
    alias:{'upstream-game':resolve(upstream,'convex/aiTown/game.ts')},nodePaths:[resolve(here,'node_modules')],plugins:[pathGuard(upstream)]});
  await import(pathToFileURL(outfile));
} finally {rmSync(dir,{recursive:true,force:true});}
