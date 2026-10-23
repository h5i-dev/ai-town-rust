import type { World } from './types';

export type FieldHistory = { initial: number; times: number[]; values: number[] };
export type MotionTrack = Array<FieldHistory | undefined>;

/** Decode the upstream version-1 locationFields packet, with bounded expansion. */
export function decodeLocation(encoded: string, start: number, end: number): MotionTrack {
  const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
  const view = new DataView(bytes.buffer);
  let offset = 0;
  const require = (n: number) => { if (offset + n > bytes.length) throw Error('Truncated motion history'); };
  require(4);
  if (view.getUint32(0, true) !== 134607547) throw Error('Unknown motion history configuration');
  offset = 4;
  const integers = () => {
    require(2); const length = view.getUint16(offset, true); offset += 2; require(length);
    const limit = offset + length, values: number[] = [];
    while (offset < limit) {
      let n = 0, shift = 0, byte: number;
      do {
        if (offset >= limit || shift > 49) throw Error('Invalid motion varint');
        byte = bytes[offset++]; n += (byte & 127) * 2 ** shift; shift += 7;
      } while (byte & 128);
      if (!Number.isSafeInteger(n)) throw Error('Motion integer overflow');
      values.push(n % 2 ? -(n + 1) / 2 : n / 2);
    }
    return values;
  };
  const runs = (input: number[]) => {
    if (input.length % 2) throw Error('Invalid motion runs');
    const out: number[] = [];
    for (let i = 0; i < input.length; i += 2) {
      const count = input[i + 1];
      if (count < 1 || count > 61 || out.length + count > 61) throw Error('Motion history exceeds tick limit');
      for (let j = 0; j < count; j++) out.push(input[i]);
    }
    return out;
  };
  const track: MotionTrack = [];
  while (offset < bytes.length) {
    require(9); const header = bytes[offset++], field = header & 15;
    if (header & 224 || field > 4 || track[field]) throw Error('Invalid motion field');
    const first = Number(view.getBigUint64(offset, true)); offset += 8;
    const deltas = runs(integers()), times = [first];
    for (const delta of deltas) {
      if (delta <= 0) throw Error('Non-monotonic motion history');
      times.push(times[times.length - 1] + delta);
    }
    if (!Number.isSafeInteger(first) || first < start || times[times.length - 1] > end || times.length > 60) {
      throw Error('Motion samples outside batch');
    }
    let values = integers();
    if (header & 16) values = runs(values);
    if (values.length !== times.length + 1) throw Error('Motion sample count mismatch');
    let value = 0;
    const scale = 2 ** (field === 4 ? 16 : 8);
    values = values.map(delta => { value += delta; return value / scale; });
    track[field] = { initial: values[0], times, values: values.slice(1) };
  }
  return track;
}

export function historicalValue(field: FieldHistory | undefined, now: number, fallback: number): number {
  if (!field) return fallback;
  let lo = 0, hi = field.times.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (field.times[mid] <= now) lo = mid + 1; else hi = mid;
  }
  return lo === 0 ? field.initial : field.values[lo - 1];
}

type Frame = { world: World; start: number; end: number; tracks: Map<number, MotionTrack> };

/** Play completed ticks. Never advance into an uncomputed future path. */
export class TownPlayback {
  private frames: Frame[] = [];
  private latest: World | null = null;
  private cursor = 0;
  private at = 0;
  private legacy = false;

  receive(world: World, at: number) {
    const h = world.motionHistory;
    this.latest = world;
    if (!h || h.endMs !== world.nowMs || h.endMs <= h.startMs) {
      this.frames = []; this.cursor = world.nowMs; this.at = at; this.legacy = true;
      return;
    }
    this.legacy = false;
    const previous = this.frames[this.frames.length - 1];
    const duplicate = previous?.start === h.startMs && previous.end === h.endMs;
    const tracks = duplicate ? previous.tracks : new Map<number, MotionTrack>();
    if (!duplicate) {
      for (const location of h.locations) {
        tracks.set(location.playerId, decodeLocation(location.buffer, h.startMs, h.endMs));
      }
    }
    // Every player gets a track, including stationary residents: no planned path fallback.
    for (const player of world.players) player.motion = tracks.get(player.id) ?? [];
    const frame = {world,start:h.startMs,end:h.endMs,tracks};
    if (duplicate) { this.frames[this.frames.length - 1] = frame; return; }
    if (!previous || h.startMs < previous.end) {
      this.frames = []; this.cursor = h.startMs; this.at = at;
    }
    this.frames.push(frame);
    // Bound latency and retained decoded state, including after a suspended browser tab.
    while (this.frames.length > 8 || (this.frames.length > 1 && h.endMs - this.frames[0].start > 2500)) this.frames.shift();
    this.cursor = Math.max(this.cursor, this.frames[0].start, h.endMs - 1500);
  }

  sample(at: number): { world: World | null; now: number } {
    const dt = Math.max(0, at - this.at); this.at = at;
    if (this.legacy) { this.cursor += dt; return {world:this.latest,now:this.cursor}; }
    const last = this.frames[this.frames.length - 1];
    if (!last) return {world:this.latest,now:this.cursor};
    const buffered = last.end - this.cursor;
    const rate = buffered < 250 ? 0.8 : buffered > 1250 ? 1.2 : 1;
    this.cursor = Math.min(last.end, this.cursor + dt * rate);
    while (this.frames.length > 1 && this.cursor >= this.frames[0].end) this.frames.shift();
    const frame = this.frames[0];
    this.cursor = Math.max(this.cursor, frame.start);
    return {world:frame.world,now:this.cursor};
  }
}
