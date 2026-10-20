import { PixiComponent, applyDefaultProps } from '@pixi/react';
import * as PIXI from 'pixi.js';
import campfire from '../animations/campfire.json';
import gentlesparkle from '../animations/gentlesparkle.json';
import gentlewaterfall from '../animations/gentlewaterfall.json';
import gentlesplash from '../animations/gentlesplash.json';
import windmill from '../animations/windmill.json';
import type { TownMap } from '../types';

const animations: Record<string, { spritesheet: PIXI.ISpritesheetData; url: string }> = {
  'campfire.json': { spritesheet: campfire as PIXI.ISpritesheetData, url: '/ai-town/assets/spritesheets/campfire.png' },
  'gentlesparkle.json': {
    spritesheet: gentlesparkle as PIXI.ISpritesheetData,
    url: '/ai-town/assets/spritesheets/gentlesparkle32.png',
  },
  'gentlewaterfall.json': {
    spritesheet: gentlewaterfall as PIXI.ISpritesheetData,
    url: '/ai-town/assets/spritesheets/gentlewaterfall32.png',
  },
  'windmill.json': { spritesheet: windmill as PIXI.ISpritesheetData, url: '/ai-town/assets/spritesheets/windmill.png' },
  'gentlesplash.json': {
    spritesheet: gentlesplash as PIXI.ISpritesheetData,
    url: '/ai-town/assets/spritesheets/gentlewaterfall32.png',
  },
};

type MapPointer = {
  screenX: number;
  screenY: number;
  target: unknown;
  currentTarget: unknown;
};

type MapProps = {
  map: TownMap;
  pointerdown?: (event: MapPointer) => void;
  pointerup?: (event: MapPointer) => void;
};

export const PixiStaticMap = PixiComponent<MapProps, PIXI.Container>('StaticMap', {
  create: (props) => {
    const map = props.map;
    const numxtiles = Math.floor(map.tileSetDimX / map.tileDim);
    const numytiles = Math.floor(map.tileSetDimY / map.tileDim);
    const bt = PIXI.BaseTexture.from(map.tileSetUrl, { scaleMode: PIXI.SCALE_MODES.NEAREST });
    const tiles: PIXI.Texture[] = [];
    for (let x = 0; x < numxtiles; x++) {
      for (let y = 0; y < numytiles; y++) {
        tiles[x + y * numxtiles] = new PIXI.Texture(
          bt,
          new PIXI.Rectangle(x * map.tileDim, y * map.tileDim, map.tileDim, map.tileDim),
        );
      }
    }
    const screenxtiles = map.bgTiles[0].length;
    const screenytiles = map.bgTiles[0][0].length;
    const container = new PIXI.Container();
    const allLayers = [...map.bgTiles, ...map.objectTiles];
    for (let i = 0; i < screenxtiles * screenytiles; i++) {
      const x = i % screenxtiles;
      const y = Math.floor(i / screenxtiles);
      for (const layer of allLayers) {
        const col = layer[x];
        if (!col) continue;
        const tileIndex = col[y];
        if (tileIndex === undefined || tileIndex === -1 || !tiles[tileIndex]) continue;
        const sprite = new PIXI.Sprite(tiles[tileIndex]);
        sprite.x = x * map.tileDim;
        sprite.y = y * map.tileDim;
        container.addChild(sprite);
      }
    }

    const spritesBySheet = new Map<string, TownMap['animatedSprites']>();
    for (const sprite of map.animatedSprites) {
      const list = spritesBySheet.get(sprite.sheet) ?? [];
      list.push(sprite);
      spritesBySheet.set(sprite.sheet, list);
    }
    for (const [sheet, sprites] of spritesBySheet) {
      const animation = animations[sheet];
      if (!animation) continue;
      const texture = PIXI.BaseTexture.from(animation.url, { scaleMode: PIXI.SCALE_MODES.NEAREST });
      const spriteSheet = new PIXI.Spritesheet(texture, animation.spritesheet);
      void spriteSheet.parse().then(() => {
        for (const sprite of sprites) {
          const frames = spriteSheet.animations[sprite.animation];
          if (!frames) continue;
          const pixiSprite = new PIXI.AnimatedSprite(frames);
          pixiSprite.animationSpeed = 0.1;
          pixiSprite.autoUpdate = true;
          pixiSprite.x = sprite.x;
          pixiSprite.y = sprite.y;
          pixiSprite.width = sprite.w;
          pixiSprite.height = sprite.h;
          container.addChild(pixiSprite);
          pixiSprite.play();
        }
      });
    }

    container.eventMode = 'static';
    container.hitArea = new PIXI.Rectangle(0, 0, screenxtiles * map.tileDim, screenytiles * map.tileDim);
    return container;
  },
  applyProps: (instance, oldProps, newProps) => {
    applyDefaultProps(instance, oldProps, newProps);
  },
});
