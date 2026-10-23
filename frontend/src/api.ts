import { useEffect, useRef, useState } from 'react';
import { TownPlayback } from './history';
import type { TownMap, World } from './types';

const TOKEN = 'ai-town-token';
const USER = 'ai-town-user';

export function storedUser(): number | null {
  const raw = localStorage.getItem(USER);
  if (!raw) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}

export async function ensureSession(): Promise<number> {
  const existing = storedUser();
  if (existing !== null && localStorage.getItem(TOKEN)) return existing;
  const res = await fetch('/session', { method: 'POST' });
  const body = (await res.json()) as { token?: string; user?: number; error?: string };
  if (!res.ok || !body.token || body.user === undefined) {
    throw new Error(body.error ?? 'session failed');
  }
  localStorage.setItem(TOKEN, body.token);
  localStorage.setItem(USER, String(body.user));
  return body.user;
}

export function forgetSession() {
  localStorage.removeItem(TOKEN);
  localStorage.removeItem(USER);
}

export async function rpc(cmd: Record<string, unknown>): Promise<Record<string, unknown>> {
  const token = localStorage.getItem(TOKEN);
  const headers: Record<string, string> = { 'content-type': 'application/json' };
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch('/rpc', { method: 'POST', headers, body: JSON.stringify(cmd) });
  const body = (await res.json()) as Record<string, unknown>;
  if (!res.ok) throw new Error(typeof body.error === 'string' ? body.error : 'request failed');
  return body;
}

export function useTown(): { world: World | null; map: TownMap | null; now: number; connected: boolean } {
  const [world, setWorld] = useState<World | null>(null);
  const playback = useRef(new TownPlayback());
  const [map, setMap] = useState<TownMap | null>(null);
  const [now, setNow] = useState(0);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    let cancelled = false;
    fetch('/map')
      .then((r) => r.json())
      .then((m: TownMap) => {
        if (!cancelled) setMap(m);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let ws: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stop = false;
    const connect = () => {
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      ws = new WebSocket(`${proto}://${location.host}/ws`);
      ws.onopen = () => setConnected(true);
      ws.onmessage = (ev) => {
        const next = JSON.parse(String(ev.data)) as World;
        try {
          playback.current.receive(next, performance.now());
        } catch (error) {
          console.error('Invalid motion history', error);
          playback.current.receive({...next,motionHistory:null,
            players:next.players.map(player => ({...player,path:[]}))}, performance.now());
        }
      };
      ws.onclose = () => {
        setConnected(false);
        if (!stop) timer = setTimeout(connect, 1000);
      };
    };
    connect();
    return () => {
      stop = true;
      if (timer) clearTimeout(timer);
      ws?.close();
    };
  }, []);

  useEffect(() => {
    let frame = 0;
    const tick = () => {
      const sample = playback.current.sample(performance.now());
      setWorld(sample.world);
      setNow(sample.now);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, []);

  return { world, map, now, connected };
}
