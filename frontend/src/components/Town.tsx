import { Stage, useApp } from '@pixi/react';
import { Viewport } from 'pixi-viewport';
import { useEffect, useRef, useState } from 'react';
import type { ISpritesheetData } from 'pixi.js';
import { characters } from '../characters';
import { orientation, poseAt } from '../pose';
import type { Player, TownMap, World } from '../types';
import { Character } from './Character';
import { DemoCrowd } from './DemoCrowd';
import { PixiStaticMap } from './PixiStaticMap';
import PixiViewport from './PixiViewport';

const EMOJI = ['', '🌱', '📖', '✨'];

export function Town({
  map,
  world,
  now,
  meId,
  onMove,
  onSelect,
}: {
  map: TownMap;
  world: World;
  now: number;
  meId: number | null;
  onMove: (x: number, y: number) => void;
  onSelect: (id: number) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 800, h: 600 });
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return (
    <div ref={box} className="absolute inset-0">
      <Stage width={size.w} height={size.h} options={{ backgroundAlpha: 0, antialias: false }}>
        <Scene map={map} world={world} now={now} meId={meId} width={size.w} height={size.h} onMove={onMove} onSelect={onSelect} />
      </Stage>
    </div>
  );
}

function Scene({
  map,
  world,
  now,
  meId,
  width,
  height,
  onMove,
  onSelect,
}: {
  map: TownMap;
  world: World;
  now: number;
  meId: number | null;
  width: number;
  height: number;
  onMove: (x: number, y: number) => void;
  onSelect: (id: number) => void;
}) {
  const app = useApp();
  const viewportRef = useRef<Viewport | undefined>(undefined);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const tile = map.tileDim;
  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || meId === null) return;
    const me = world.players.find((p) => p.id === meId);
    if (!me) return;
    const pose = poseAt(me, now);
    viewport.animate({ position: { x: pose.x * tile, y: pose.y * tile } as never, scale: 1.5 });
    // Only when the viewer appears, not on every clock tick.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meId]);

  return (
    <PixiViewport
      app={app}
      viewportRef={viewportRef}
      overview={map.demo}
      screenWidth={width}
      screenHeight={height}
      worldWidth={map.width * tile}
      worldHeight={map.height * tile}
    >
      <PixiStaticMap
        map={map}
        pointerdown={(e) => {
          // Characters are drawn above the map and the event bubbles. A click on one
          // must not also walk there.
          if (e.target !== e.currentTarget) return;
          drag.current = { x: e.screenX, y: e.screenY };
        }}
        pointerup={(e) => {
          const start = drag.current;
          drag.current = null;
          if (!start || e.target !== e.currentTarget) return;
          const dx = start.x - e.screenX;
          const dy = start.y - e.screenY;
          if (Math.hypot(dx, dy) > 10) return;
          if (meId === null) return;
          const viewport = viewportRef.current;
          if (!viewport) return;
          const point = viewport.toWorld(e.screenX, e.screenY);
          onMove(Math.floor(point.x / tile), Math.floor(point.y / tile));
        }}
      />
      {map.demo ? <DemoCrowd app={app} world={world} now={now} tile={tile} viewportRef={viewportRef} onSelect={onSelect} /> : world.players.map((player) => (
        <Figure key={player.id} player={player} now={now} tile={tile} viewer={player.id === meId} onSelect={onSelect} world={world} />
      ))}
    </PixiViewport>
  );
}

function Figure({
  player,
  now,
  tile,
  viewer,
  onSelect,
  world,
}: {
  player: Player;
  now: number;
  tile: number;
  viewer: boolean;
  onSelect: (id: number) => void;
  world: World;
}) {
  const sheet = characters[player.character];
  if (!sheet) return null;
  const pose = poseAt(player, now);
  const talking = world.conversations.some((c) => c.typingPlayer === player.id);
  const thinking = !talking && world.agents.some((a) => a.player === player.id && a.op !== 0);
  const emoji = player.activityUntil > now ? EMOJI[player.activity] ?? '' : '';
  return (
    <Character
      textureUrl={sheet.textureUrl}
      spritesheetData={sheet.spritesheetData as unknown as ISpritesheetData}
      x={pose.x * tile + tile / 2}
      y={pose.y * tile + tile / 2}
      orientation={orientation(pose.faceDx, pose.faceDy)}
      isMoving={pose.speed > 0.05}
      isThinking={thinking}
      isSpeaking={talking}
      emoji={emoji}
      isViewer={viewer}
      speed={sheet.speed}
      onClick={() => onSelect(player.id)}
    />
  );
}
