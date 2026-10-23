import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { homedir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pathGuard } from './path-guard.mjs';

test('native path interpolation stays finite at equal timestamps', async () => {
  const upstream = resolve(process.env.AI_TOWN_UPSTREAM ?? `${homedir()}/Ref/ai-town`);
  const result = await build({
    entryPoints: [resolve(upstream, 'convex/util/geometry.ts')],
    bundle: true, platform: 'node', format: 'esm', write: false,
    nodePaths: [fileURLToPath(new URL('./node_modules', import.meta.url))],
    plugins: [pathGuard(upstream)],
  });
  const { pathPosition } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);
  // Captured from a native conversation replan at 49.184 simulated seconds.
  const pose = pathPosition([[13.999999999999957,15,1,0,1049184],[14,15,1,0,1049184]],1049184);
  assert.deepEqual(pose, { position: {x:14,y:15}, facing: {dx:1,dy:0}, velocity:0 });
  assert.deepEqual(pathPosition([[0,0,1,0,0],[1,0,1,0,1000]],500),
    { position:{x:0.5,y:0}, facing:{dx:1,dy:0}, velocity:0.001 });
});
