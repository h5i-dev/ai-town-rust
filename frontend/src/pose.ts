import type { Player } from './types';

const MILLI = 1000;
const BIAS = 1000;

export type Pose = { x: number; y: number; faceDx: number; faceDy: number; speed: number };

/** Facing stored as a milli-component plus 1000, so -1..1 maps to 0..2000. */
export function orientation(faceDx: number, faceDy: number): number {
  const dx = (faceDx - BIAS) / MILLI;
  const dy = (faceDy - BIAS) / MILLI;
  if (dx * dx + dy * dy < 1e-8) return 0;
  const twoPi = Math.PI * 2;
  const radians = (Math.atan2(dy, dx) + twoPi) % twoPi;
  return (radians / twoPi) * 360;
}

/** Tile position along the path at server time `now`, matching geometry.ts. */
export function poseAt(player: Player, now: number): Pose {
  const path = player.path;
  const still = (x: number, y: number, faceDx: number, faceDy: number): Pose => ({
    x: x / MILLI,
    y: y / MILLI,
    faceDx,
    faceDy,
    speed: 0,
  });
  if (!path || path.length < 2) return still(player.x, player.y, player.faceDx, player.faceDy);
  const first = path[0];
  if (now < first.t) return still(first.x, first.y, first.faceDx, first.faceDy);
  const last = path[path.length - 1];
  if (now > last.t) return still(last.x, last.y, last.faceDx, last.faceDy);
  for (let i = 0; i + 1 < path.length; i++) {
    const a = path[i];
    const b = path[i + 1];
    if (a.t <= now && now <= b.t) {
      const dt = b.t - a.t;
      const u = dt === 0 ? 0 : (now - a.t) / dt;
      const dist = Math.hypot(b.x - a.x, b.y - a.y) / MILLI;
      return {
        x: (a.x + u * (b.x - a.x)) / MILLI,
        y: (a.y + u * (b.y - a.y)) / MILLI,
        faceDx: a.faceDx,
        faceDy: a.faceDy,
        speed: dt === 0 ? 0 : dist / (dt / 1000),
      };
    }
  }
  return still(player.x, player.y, player.faceDx, player.faceDy);
}
