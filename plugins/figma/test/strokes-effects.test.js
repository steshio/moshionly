'use strict';

// Strokes, shadows and blurs.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection } = require('../code.js');
const F = require('../fixtures/figma.js');

async function convert(...layers) {
  const board = F.frame({ name: 'Board', width: 400, height: 400 }, layers);
  F.page(board);
  const result = await convertSelection([board], { fileKey: 'FILE' });
  return { layers: result.nodes.slice(1), notes: result.notes };
}

const shadow = (spec) =>
  Object.assign(
    { type: 'DROP_SHADOW', visible: true, blendMode: 'NORMAL', color: { r: 0, g: 0, b: 0, a: 0.25 }, offset: { x: 0, y: 4 }, radius: 12, spread: 0 },
    spec,
  );

test('a stroke keeps its paint, width, position, ends and corners', async () => {
  const { layers, notes } = await convert(
    F.rect({
      name: 'Outlined',
      width: 50,
      height: 50,
      strokes: [F.solid(0, 0, 0)],
      strokeWeight: 2,
      strokeAlign: 'INSIDE',
      strokeCap: 'ROUND',
      strokeJoin: 'BEVEL',
    }),
  );
  assert.deepEqual(layers[0].stroke, { paint: '#000000', width: 2, position: 'inside', cap: 'round', join: 'bevel' });
  assert.deepEqual(notes, []);
});

test('a centred stroke with flat ends and sharp corners only has paint and width', async () => {
  const { layers } = await convert(
    F.ellipse({ name: 'Ring', width: 50, height: 50, strokes: [F.solid(1, 0, 0)], strokeWeight: 1, strokeAlign: 'CENTER', strokeCap: 'NONE', strokeJoin: 'MITER' }),
  );
  assert.deepEqual(layers[0].stroke, { paint: '#FF0000', width: 1 });
});

test('no visible stroke paint or no width means no stroke', async () => {
  const hidden = Object.assign(F.solid(0, 0, 0), { visible: false });
  const { layers } = await convert(
    F.rect({ name: 'Hidden', width: 5, height: 5, strokes: [hidden], strokeWeight: 3 }),
    F.rect({ name: 'Thin', width: 5, height: 5, strokes: [F.solid(0, 0, 0)], strokeWeight: 0 }),
  );
  assert.equal(layers[0].stroke, undefined);
  assert.equal(layers[1].stroke, undefined);
});

test('sides of different widths become the widest, with a note', async () => {
  const { layers, notes } = await convert(
    F.frame({
      name: 'Card',
      width: 100,
      height: 100,
      strokes: [F.solid(0, 0, 0)],
      strokeWeight: 'MIXED',
      strokeTopWeight: 1,
      strokeRightWeight: 4,
      strokeBottomWeight: 1,
      strokeLeftWeight: 1,
      strokeAlign: 'INSIDE',
    }),
  );
  assert.equal(layers[0].stroke.width, 4);
  assert.match(notes[0].message, /different widths/);
});

test('dashes, arrow ends and varying widths are noted', async () => {
  const { layers, notes } = await convert(
    F.vector({
      name: 'Arrow',
      width: 100,
      height: 0,
      vectorPaths: [{ windingRule: 'NONE', data: 'M 0 0 L 100 0' }],
      strokes: [F.solid(0, 0, 0)],
      strokeWeight: 2,
      strokeCap: 'ARROW_LINES',
      dashPattern: [4, 4],
      variableWidthStrokeProperties: { widthProfile: 'TAPER' },
    }),
  );
  assert.deepEqual(layers[0].stroke, { paint: '#000000', width: 2 });
  const messages = notes.map((note) => note.message).join('\n');
  assert.match(messages, /arrow lines line ends/);
  assert.match(messages, /dashed stroke is drawn as a solid line/);
  assert.match(messages, /varying width/);
});

test('a gradient stroke on a line runs along the line', async () => {
  const across = {
    type: 'GRADIENT_LINEAR',
    visible: true,
    opacity: 1,
    blendMode: 'NORMAL',
    gradientTransform: [[1, 0, 0], [0, 1, 0]],
    gradientStops: [
      { position: 0, color: { r: 1, g: 1, b: 1, a: 1 } },
      { position: 1, color: { r: 0, g: 0, b: 0, a: 1 } },
    ],
  };
  const { layers } = await convert(F.line({ name: 'Rule', width: 100, strokes: [across], strokeWeight: 2 }));
  assert.deepEqual([layers[0].stroke.paint.start, layers[0].stroke.paint.end], [[0, 0], [1, 0]]);
});

test('a drop shadow keeps its colour, offset and blur', async () => {
  const { layers, notes } = await convert(
    F.rect({ name: 'Raised', width: 10, height: 10, effects: [shadow({ offset: { x: 2, y: 8 }, radius: 24 })] }),
  );
  assert.deepEqual(layers[0].shadow, { color: '#00000040', x: 2, y: 8, blur: 24 });
  assert.deepEqual(notes, []);
});

/** Where a layer's shadow falls on the page: its offset turned with the layer and its parents. */
function onPage(shadowOf, ...rotations) {
  const radians = (rotations.reduce((sum, degrees) => sum + (degrees || 0), 0) * Math.PI) / 180;
  const [x, y] = [shadowOf.x, shadowOf.y];
  const turned = [x * Math.cos(radians) - y * Math.sin(radians), x * Math.sin(radians) + y * Math.cos(radians)];
  return turned.map((value) => Math.round(value * 100) / 100 + 0);
}

// Figma keeps a shadow's direction fixed on the page (from user reports;
// see the README), while Moshionly turns it with the layer.

test('a turned layer’s shadow still falls the same way on the page', async () => {
  const { layers } = await convert(
    F.rect({ name: 'Turned', x: 100, y: 100, width: 50, height: 50, rotation: 90, effects: [shadow({ offset: { x: 0, y: 10 } })] }),
  );
  assert.equal(layers[0].rotation, 90);
  assert.deepEqual([layers[0].shadow.x, layers[0].shadow.y], [10, 0]);
  assert.deepEqual(onPage(layers[0].shadow, layers[0].rotation), [0, 10], 'below the layer, as in Figma');
});

test('a mirrored layer’s shadow still falls the same way on the page', async () => {
  const { layers } = await convert(
    F.rect({ name: 'Flipped', width: 10, height: 10, mirror: 'x', effects: [shadow({ offset: { x: 4, y: 6 } })] }),
  );
  // The mirror is a vertical flip turned by 180 degrees: the flip is in the
  // layer's own coordinates, the turn takes the offset round to (4, 6).
  assert.equal(Math.abs(layers[0].rotation), 180);
  assert.deepEqual([layers[0].shadow.x, layers[0].shadow.y], [-4, -6]);
  assert.deepEqual(onPage(layers[0].shadow, layers[0].rotation), [4, 6]);
});

test('the shadow of a layer in a turned group still falls the same way on the page', async () => {
  const child = F.rect({ name: 'Child', x: 20, y: 0, width: 20, height: 10, rotation: 30, effects: [shadow({ offset: { x: 3, y: 8 } })] });
  const holder = F.group({ name: 'Holder', x: 100, y: 100, width: 100, height: 100, rotation: 45 }, [child]);
  const { layers } = await convert(holder);
  const [group, placed] = layers;
  assert.deepEqual([group.rotation, placed.rotation], [45, 30]);
  assert.deepEqual(onPage(placed.shadow, group.rotation, placed.rotation), [3, 8]);
});

test('extra shadows, spread, inner shadows and background blur are noted', async () => {
  const { layers, notes } = await convert(
    F.rect({
      name: 'Busy',
      width: 10,
      height: 10,
      effects: [
        shadow({ color: { r: 0, g: 0, b: 0, a: 0.1 } }),
        shadow({ color: { r: 0, g: 0, b: 0, a: 0.5 }, spread: 3 }),
        Object.assign(shadow({}), { type: 'INNER_SHADOW' }),
        { type: 'BACKGROUND_BLUR', visible: true, radius: 10 },
        { type: 'LAYER_BLUR', visible: false, radius: 50 },
      ],
    }),
  );
  assert.equal(layers[0].shadow.color, '#00000080', 'the strongest shadow is kept');
  assert.equal(layers[0].blur, undefined, 'a hidden blur is not applied');
  const messages = notes.map((note) => note.message);
  assert.equal(messages.length, 4);
  assert.match(messages[0], /2 drop shadows/);
  assert.match(messages[1], /spread/);
  assert.match(messages[2], /inner shadow/);
  assert.match(messages[3], /background blur/);
});

test('a layer blur carries over; a progressive one becomes even', async () => {
  const { layers, notes } = await convert(
    F.rect({ name: 'Soft', width: 10, height: 10, effects: [{ type: 'LAYER_BLUR', blurType: 'NORMAL', visible: true, radius: 8 }] }),
    F.rect({
      name: 'Fading',
      width: 10,
      height: 10,
      effects: [{ type: 'LAYER_BLUR', blurType: 'PROGRESSIVE', visible: true, radius: 20, startRadius: 0 }],
    }),
  );
  assert.equal(layers[0].blur, 8);
  assert.equal(layers[1].blur, 10);
  assert.equal(notes.length, 1);
  assert.match(notes[0].message, /evenly by 10 px/);
});

test('a top-level frame’s stroke and shadow go on the group inside its artboard', async () => {
  const card = F.frame({
    name: 'Card',
    width: 200,
    height: 100,
    fills: [F.solid(1, 1, 1)],
    strokes: [F.solid(0, 0, 0)],
    strokeWeight: 1,
    effects: [shadow({})],
  });
  F.page(card);
  const result = await convertSelection([card], { fileKey: 'FILE' });
  const [board, body] = result.nodes;
  assert.equal(board.fill, undefined);
  assert.equal(board.shadow, undefined);
  assert.equal(body.fill, '#FFFFFF');
  assert.deepEqual(body.stroke, { paint: '#000000', width: 1 });
  assert.equal(body.shadow.blur, 12);
  assert.match(result.notes[0].message, /cuts it off/);
});
