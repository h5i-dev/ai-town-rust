// Keep the checkout intact. The town bundle guards upstream's 0/0 path
// interpolation; a collision replan can make two timestamps identical when
// a position is only floating-point epsilon away from an integer tile.
import { readFileSync } from 'node:fs';
export function pathGuard(upstream) {
  return {
    name: 'upstream-zero-duration-path',
    setup(build) {
      build.onLoad({ filter: /\/convex\/util\/geometry\.ts$/ }, ({ path }) => {
        if (!path.startsWith(`${upstream}/`)) return;
        const source = readFileSync(path, 'utf8');
        const needle = 'const interp = (time - segmentStart.t) / (segmentEnd.t - segmentStart.t);';
        if (source.split(needle).length !== 2) throw Error('Upstream geometry changed; review the path guard');
        return { loader: 'ts', contents: source.replace(needle, `
          if (segmentStart.t === segmentEnd.t) {
            return { position: segmentEnd.position, facing: segmentEnd.facing, velocity: 0 };
          }
          ${needle}`) };
      });
    },
  };
}
