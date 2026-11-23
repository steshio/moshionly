'use strict';

// Masks: Figma's "mask hides what is above it" as Moshionly mask groups.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection } = require('../code.js');
const F = require('../fixtures/figma.js');

async function convert(...layers) {
  const board = F.frame({ name: 'Board', width: 400, height: 400 }, layers);
  F.page(board);
  const result = await convertSelection([board], { fileKey: 'FILE' });
  return Object.assign(result, { board: result.nodes[0], layers: result.nodes.slice(1) });
}

const summary = (result) => result.layers.map((layer) => [layer.name, layer.type, layer.parent]);

test('a group made with "Use as mask" becomes a mask group', async () => {
  const shape = F.ellipse({ name: 'Circle', x: 0, y: 0, width: 80, height: 80, isMask: true, fills: [F.solid(0, 0, 0)] });
  const photo = F.rect({ name: 'Photo', x: -10, y: -10, width: 120, height: 100, fills: [F.solid(1, 0, 0)] });
  const label = F.text({ name: 'Label', x: 10, y: 30, width: 60, height: 20, characters: 'Hi' });
  const masked = F.group({ name: 'Mask group', x: 50, y: 60, width: 80, height: 80 }, [shape, photo, label]);
  const result = await convert(masked);
  assert.deepEqual(summary(result), [
    ['Mask group', 'mask_group', result.board.id],
    ['Circle', 'ellipse', masked.id],
    ['Photo', 'rect', masked.id],
    ['Label', 'text', masked.id],
  ]);
  assert.equal(result.layers[0].source_id, 'figma:FILE:' + masked.id);
  assert.deepEqual([result.layers[2].x, result.layers[2].y], [-10, -10], 'positions stay relative to the group');
  assert.deepEqual(result.notes, []);
});

test('a mask in the middle of a frame groups itself with the layers above it', async () => {
  const background = F.rect({ name: 'Background', width: 400, height: 400 });
  const window = F.rect({ name: 'Window', x: 100, y: 50, width: 200, height: 100, isMask: true, rotation: 90 });
  const scene = F.rect({ name: 'Scene', x: 0, y: 0, width: 400, height: 400 });
  const spot = F.ellipse({ name: 'Spot', x: 20, y: 20, width: 40, height: 40, isMask: true });
  const glow = F.rect({ name: 'Glow', x: 0, y: 0, width: 100, height: 100 });
  const result = await convert(background, window, scene, spot, glow);
  const groups = result.layers.filter((layer) => layer.type === 'mask_group');
  assert.deepEqual(summary(result), [
    ['Background', 'rect', result.board.id],
    ['Window group', 'mask_group', result.board.id],
    ['Window', 'rect', groups[0].id],
    ['Scene', 'rect', groups[0].id],
    ['Spot group', 'mask_group', result.board.id],
    ['Spot', 'ellipse', groups[1].id],
    ['Glow', 'rect', groups[1].id],
  ]);
  // The window is turned by 90 degrees about its corner at (100, 50), so
  // on the board it covers x 0 to 100 and y 50 to 250: that is the box of
  // its mask group, and the layers inside are placed from there.
  assert.deepEqual([groups[0].x, groups[0].y, groups[0].width, groups[0].height], [0, 50, 100, 200]);
  assert.equal(groups[0].id, window.id + '/mask');
  assert.equal(groups[0].source_id, 'figma:FILE:' + window.id + '/mask');
  const scenePlaced = result.layers.find((layer) => layer.name === 'Scene');
  assert.deepEqual([scenePlaced.x, scenePlaced.y], [0, -50]);
});

test('a hidden mask masks nothing', async () => {
  const result = await convert(
    F.rect({ name: 'Off', width: 10, height: 10, isMask: true, visible: false }),
    F.rect({ name: 'Shown', width: 10, height: 10 }),
  );
  assert.ok(result.layers.every((layer) => layer.type === 'rect'));
});

test('brightness masks and masks sent on their own are noted', async () => {
  const shape = F.rect({ name: 'Bright', width: 10, height: 10, isMask: true, maskType: 'LUMINANCE' });
  const result = await convert(F.group({ name: 'G', width: 10, height: 10 }, [shape, F.rect({ name: 'Under', width: 10, height: 10 })]));
  assert.match(result.notes[0].message, /brightness/);

  const lone = F.rect({ name: 'Lone', width: 10, height: 10, isMask: true });
  F.page(lone);
  const alone = await convertSelection([lone], { fileKey: 'FILE' });
  assert.match(alone.notes[0].message, /select what holds it/);
});

test('in an auto layout frame with its first layer on top, a mask hides what is drawn above it', async () => {
  const stack = F.frame({ name: 'Stack', width: 100, height: 100, layoutMode: 'VERTICAL', itemReverseZIndex: true }, [
    F.rect({ name: 'Top', width: 100, height: 40 }),
    F.ellipse({ name: 'Spot', y: 40, width: 20, height: 20, isMask: true }),
    F.rect({ name: 'Bottom', y: 60, width: 100, height: 40 }),
  ]);
  const result = await convert(stack);
  const group = result.layers.find((layer) => layer.type === 'mask_group');
  assert.deepEqual(summary(result), [
    ['Stack', 'group', result.board.id],
    ['Bottom', 'rect', stack.id],
    ['Spot group', 'mask_group', stack.id],
    ['Spot', 'ellipse', group.id],
    ['Top', 'rect', group.id],
  ]);
});

test('a vector mask ignores its own transparency, as in Figma', async () => {
  const fade = {
    type: 'GRADIENT_LINEAR',
    visible: true,
    opacity: 1,
    blendMode: 'NORMAL',
    gradientTransform: [[1, 0, 0], [0, 1, 0]],
    gradientStops: [
      { position: 0, color: { r: 1, g: 0, b: 0, a: 1 } },
      { position: 1, color: { r: 1, g: 0, b: 0, a: 0 } },
    ],
  };
  const masked = (mask) => F.group({ name: 'G', width: 80, height: 80 }, [mask, F.rect({ name: 'Photo', width: 80, height: 80, fills: [F.solid(0, 0, 1)] })]);
  const result = await convert(
    masked(
      F.ellipse({
        name: 'Faint',
        width: 80,
        height: 80,
        isMask: true,
        maskType: 'VECTOR',
        opacity: 0.5,
        fills: [F.solid(0, 0, 0, 0.4)],
        strokes: [F.solid(1, 1, 1, 0.3)],
        strokeWeight: 2,
      }),
    ),
    masked(F.rect({ name: 'Fading', width: 80, height: 80, isMask: true, maskType: 'VECTOR', fills: [fade] })),
    masked(F.ellipse({ name: 'Alpha', width: 80, height: 80, isMask: true, maskType: 'ALPHA', fills: [F.solid(0, 0, 0, 0.4)] })),
  );
  const layer = (name) => result.layers.find((item) => item.name === name);
  assert.equal(layer('Faint').fill, '#000000');
  assert.deepEqual(layer('Faint').stroke, { paint: '#FFFFFF', width: 2 });
  assert.equal(layer('Faint').opacity, undefined);
  assert.deepEqual(layer('Fading').fill.stops.map((stop) => stop.color), ['#FF0000', '#FF0000']);
  assert.equal(layer('Alpha').fill, '#00000066', 'an alpha mask keeps its transparency');
  assert.deepEqual(result.notes, []);
});

test('a vector mask that cannot be made opaque exactly is noted', async () => {
  const masked = (mask) => F.group({ name: 'G', width: 80, height: 80 }, [mask, F.rect({ name: 'Photo', width: 80, height: 80 })]);
  const shadowed = F.ellipse({
    name: 'Shadowed',
    width: 80,
    height: 80,
    isMask: true,
    maskType: 'VECTOR',
    fills: [F.solid(0, 0, 0, 0.4)],
    effects: [{ type: 'DROP_SHADOW', visible: true, blendMode: 'NORMAL', color: { r: 0, g: 0, b: 0, a: 0.5 }, offset: { x: 0, y: 4 }, radius: 8, spread: 0 }],
  });
  const clear = F.rect({ name: 'Clear', width: 80, height: 80, isMask: true, maskType: 'VECTOR', fills: [F.solid(0, 0, 0, 0)] });
  const solid = F.rect({ name: 'Solid', width: 80, height: 80, isMask: true, maskType: 'VECTOR', fills: [F.solid(0, 0, 0)] });
  const result = await convert(masked(shadowed), masked(clear), masked(solid));
  assert.equal(result.layers.find((item) => item.name === 'Shadowed').fill, '#00000066', 'left as it is');
  assert.deepEqual(
    result.notes.map((note) => [note.layer_name, /vector mask/.test(note.message)]),
    [
      ['Shadowed', true],
      ['Clear', true],
    ],
  );
});
