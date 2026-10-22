import { PixiComponent } from '@pixi/react';
import { Application, BaseTexture, Container, ISpritesheetData, Sprite, Spritesheet, SCALE_MODES } from 'pixi.js';
import { Viewport } from 'pixi-viewport';
import { MutableRefObject } from 'react';
import { characters } from '../characters';
import { orientation, poseAt } from '../pose';
import type { World } from '../types';

type Props = {
  app: Application;
  world: World;
  now: number;
  tile: number;
  viewportRef: MutableRefObject<Viewport | undefined>;
  onSelect: (id: number) => void;
};

// Keep the normal character UI for interactive towns. A demo crowd uses one
// Pixi container and shared sheets so React does not reconcile 10,000 figures
// on every animation frame. Simulation and interpolation use the same data.
class Crowd extends Container {
  props: Props;
  sheets: Spritesheet[] = [];
  figures: Sprite[] = [];
  constructor(props: Props) {
    super();
    this.props = props;
    void Promise.all(characters.map(async (character) => {
      const sheet = new Spritesheet(
        BaseTexture.from(character.textureUrl, { scaleMode: SCALE_MODES.NEAREST }),
        character.spritesheetData as unknown as ISpritesheetData,
      );
      await sheet.parse();
      return sheet;
    })).then((sheets) => {
      if (this.destroyed) return;
      this.sheets = sheets;
      this.update();
    });
    props.app.ticker.add(this.update, this);
  }
  update() {
    if (!this.sheets.length) return;
    const { world, now, tile, viewportRef } = this.props;
    const bounds = viewportRef.current?.getVisibleBounds();
    for (let i = 0; i < world.players.length; i++) {
      const player = world.players[i];
      let sprite = this.figures[i];
      if (!sprite) {
        sprite = new Sprite();
        sprite.anchor.set(0.5);
        sprite.eventMode = 'static';
        sprite.cursor = 'pointer';
        sprite.on('pointerdown', (event) => {
          event.stopPropagation();
          this.props.onSelect(player.id);
        });
        this.figures.push(sprite);
        this.addChild(sprite);
      }
      const pose = poseAt(player, now);
      sprite.position.set(pose.x * tile + tile / 2, pose.y * tile + tile / 2);
      sprite.visible = !bounds || (sprite.x >= bounds.x - tile && sprite.x <= bounds.right + tile
        && sprite.y >= bounds.y - tile && sprite.y <= bounds.bottom + tile);
      if (!sprite.visible) continue;
      const direction = ['right', 'down', 'left', 'up'][Math.floor(orientation(pose.faceDx, pose.faceDy) / 90)];
      const frames = this.sheets[player.character].animations[direction];
      const moving = pose.speed > 0.05 || (world.demo?.engine === 'original' && player.speed > 50);
      sprite.texture = frames[moving ? Math.floor(now / 150) % frames.length : 0];
    }
  }
  destroy() {
    this.props.app.ticker.remove(this.update, this);
    super.destroy({ children: true });
  }
}

export const DemoCrowd = PixiComponent<Props, Crowd>('DemoCrowd', {
  create: (props) => new Crowd(props),
  applyProps: (crowd, _old, props) => { crowd.props = props; },
  willUnmount: (crowd) => { crowd.destroy(); },
  config: { destroy: false },
});
