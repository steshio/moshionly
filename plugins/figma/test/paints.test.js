'use strict';

// Paints: colours, gradients and the one-fill rule.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection } = require('../code.js');
const F = require('../fixtures/figma.js');

async function convert(...layers) {
  const board = F.frame({ name: 'Board', width: 400, height: 400 }, layers);
  F.page(board);
  const result = await convertSelection([board], { fileKey: 'FILE' });
  return { board: result.nodes[0], layers: result.nodes.slice(1), notes: result.notes };
}

const gradient = (type, transform, stops) => ({
  type,
  visible: true,
  opacity: 1,
  blendMode: 'NORMAL',
  gradientTransform: transform,
  gradientStops: stops || [
    { position: 0, color: { r: 1, g: 0, b: 0, a: 1 } },
    { position: 1, color: { r: 0, g: 0, b: 1, a: 1 } },
  ],
});

/** A small, repeatable source of numbers between -1 and 1. */
function numbers(seed) {
  let state = seed;
  return () => {
    state = (state * 1103515245 + 12345) % 2147483648;
    return (state / 2147483648) * 2 - 1;
  };
}

/** A random invertible gradient transform. */
function randomTransform(next) {
  const turn = next() * Math.PI;
  const scaleX = 0.6 + Math.abs(next());
  const scaleY = 0.6 + Math.abs(next());
  const shear = next() * 0.5;
  const [cos, sin] = [Math.cos(turn), Math.sin(turn)];
  return [
    [cos * scaleX, cos * shear * scaleY - sin * scaleY, next() * 0.5 + 0.5],
    [sin * scaleX, sin * shear * scaleY + cos * scaleY, next() * 0.5],
  ];
}

/** Figma's gradient-space point for a pixel of a w by h box. */
function figmaSpace(transform, w, h, x, y) {
  const [[a, c, e], [b, d, f]] = transform;
  const [u, v] = [x / w, y / h];
  return [a * u + c * v + e, b * u + d * v + f];
}

test('colours carry their opacity as alpha', async () => {
  const { layers } = await convert(
    F.rect({ name: 'Half', width: 10, height: 10, fills: [F.solid(1, 0, 0, 0.5)] }),
    F.rect({ name: 'Full', width: 10, height: 10, fills: [F.solid(0.05, 0.6, 1)] }),
  );
  assert.equal(layers[0].fill, '#FF000080');
  assert.equal(layers[1].fill, '#0D99FF');
});

test('hidden fills and fills covered by an opaque one are dropped without a note', async () => {
  const hidden = Object.assign(F.solid(0, 1, 0), { visible: false });
  const { layers, notes } = await convert(
    F.rect({ name: 'Covered', width: 10, height: 10, fills: [gradient('GRADIENT_LINEAR', [[1, 0, 0], [0, 1, 0]]), F.solid(0, 0, 0), hidden] }),
  );
  assert.equal(layers[0].fill, '#000000');
  assert.deepEqual(notes, []);
});

test('stacked colours mix into the one colour they show', async () => {
  const { layers, notes } = await convert(
    F.rect({ name: 'Stack', width: 10, height: 10, fills: [F.solid(1, 1, 1), F.solid(0, 0, 0, 0.5)] }),
  );
  assert.equal(layers[0].fill, '#808080');
  assert.deepEqual(notes, []);
});

test('a colour over a gradient keeps the top paint and says so', async () => {
  const { layers, notes } = await convert(
    F.rect({
      name: 'Tinted',
      width: 10,
      height: 10,
      fills: [gradient('GRADIENT_LINEAR', [[1, 0, 0], [0, 1, 0]]), F.solid(1, 1, 1, 0.2)],
    }),
  );
  assert.equal(layers[0].fill, '#FFFFFF33');
  assert.equal(notes.length, 1);
  assert.match(notes[0].message, /2 fills/);
});

test('Figma’s default gradient runs from the top to the bottom', async () => {
  const { layers } = await convert(
    F.rect({ name: 'Down', width: 300, height: 100, fills: [gradient('GRADIENT_LINEAR', [[0, 1, 0], [-1, 0, 1]])] }),
    F.rect({ name: 'Across', width: 300, height: 100, fills: [gradient('GRADIENT_LINEAR', [[1, 0, 0], [0, 1, 0]])] }),
  );
  assert.deepEqual(layers[0].fill, {
    type: 'linear_gradient',
    start: [0.5, 0],
    end: [0.5, 1],
    stops: [
      { offset: 0, color: '#FF0000' },
      { offset: 1, color: '#0000FF' },
    ],
  });
  assert.deepEqual([layers[1].fill.start, layers[1].fill.end], [[0, 0.5], [1, 0.5]]);
});

test('linear gradients give every pixel the offset Figma gives it', async () => {
  const next = numbers(7);
  for (let round = 0; round < 20; round += 1) {
    const [w, h] = [50 + Math.abs(next()) * 400, 50 + Math.abs(next()) * 400];
    const transform = randomTransform(next);
    const { layers } = await convert(F.rect({ name: 'R', width: w, height: h, fills: [gradient('GRADIENT_LINEAR', transform)] }));
    const { start, end } = layers[0].fill;
    const [sx, sy, ex, ey] = [start[0] * w, start[1] * h, end[0] * w, end[1] * h];
    const length = (ex - sx) ** 2 + (ey - sy) ** 2;
    for (let sample = 0; sample < 10; sample += 1) {
      const [x, y] = [Math.abs(next()) * w, Math.abs(next()) * h];
      const expected = figmaSpace(transform, w, h, x, y)[0];
      const actual = ((x - sx) * (ex - sx) + (y - sy) * (ey - sy)) / length;
      assert.ok(Math.abs(actual - expected) < 1e-3, `round ${round}: ${actual} is not ${expected}`);
    }
  }
});

test('radial gradients give every pixel the offset Figma gives it', async () => {
  const next = numbers(11);
  for (let round = 0; round < 20; round += 1) {
    const [w, h] = [50 + Math.abs(next()) * 400, 50 + Math.abs(next()) * 400];
    const transform = randomTransform(next);
    const { layers } = await convert(F.rect({ name: 'R', width: w, height: h, fills: [gradient('GRADIENT_RADIAL', transform)] }));
    const fill = layers[0].fill;
    const [cx, cy] = [fill.center[0] * w, fill.center[1] * h];
    const [rx, ry] = [fill.radius[0] * w, fill.radius[1] * h];
    const turn = ((fill.rotation || 0) * Math.PI) / 180;
    for (let sample = 0; sample < 10; sample += 1) {
      const [x, y] = [Math.abs(next()) * w, Math.abs(next()) * h];
      const [gx, gy] = figmaSpace(transform, w, h, x, y);
      const expected = Math.hypot(gx - 0.5, gy - 0.5) / 0.5;
      // Moshionly: undo the move, the turn and the stretch of the unit circle.
      const [dx, dy] = [x - cx, y - cy];
      const [ux, uy] = [dx * Math.cos(turn) + dy * Math.sin(turn), -dx * Math.sin(turn) + dy * Math.cos(turn)];
      const actual = Math.hypot(ux / rx, uy / ry);
      assert.ok(Math.abs(actual - expected) < 2e-3 * Math.max(1, expected), `round ${round}: ${actual} is not ${expected}`);
    }
  }
});

test('a plain radial gradient fills the box', async () => {
  const { layers } = await convert(
    F.ellipse({ name: 'Glow', width: 200, height: 100, fills: [gradient('GRADIENT_RADIAL', [[1, 0, 0], [0, 1, 0]])] }),
  );
  assert.deepEqual(layers[0].fill.center, [0.5, 0.5]);
  assert.deepEqual(layers[0].fill.radius, [0.5, 0.5]);
  assert.equal(layers[0].fill.rotation, undefined);
});

test('a gradient on a mirrored layer turns with it', async () => {
  const { layers } = await convert(
    F.rect({ name: 'Flipped', width: 100, height: 50, mirror: 'x', fills: [gradient('GRADIENT_LINEAR', [[1, 0, 0], [0, 1, 0]])] }),
  );
  // Turned by 180 degrees, the layer's left edge is on the page's right,
  // which is where the mirrored gradient starts.
  assert.equal(Math.abs(layers[0].rotation), 180);
  assert.deepEqual([layers[0].fill.start, layers[0].fill.end], [[0, 0.5], [1, 0.5]]);
});

test('gradient stops take the paint’s opacity', async () => {
  const faded = Object.assign(gradient('GRADIENT_LINEAR', [[1, 0, 0], [0, 1, 0]]), { opacity: 0.5 });
  const { layers } = await convert(F.rect({ name: 'Faded', width: 10, height: 10, fills: [faded] }));
  assert.deepEqual(
    layers[0].fill.stops.map((stop) => stop.color),
    ['#FF000080', '#0000FF80'],
  );
});

test('angular and diamond gradients become radial ones, with a note', async () => {
  const { layers, notes } = await convert(
    F.rect({ name: 'Cone', width: 10, height: 10, fills: [gradient('GRADIENT_ANGULAR', [[1, 0, 0], [0, 1, 0]])] }),
  );
  assert.equal(layers[0].fill.type, 'radial_gradient');
  assert.match(notes[0].message, /angular gradient/);
});

test('a frame’s fill goes on its group, a top-level frame’s on its artboard', async () => {
  const inner = F.frame({ name: 'Inner', width: 10, height: 10, fills: [F.solid(1, 1, 1)] });
  const { board, layers } = await convert(inner);
  assert.equal(layers[0].fill, '#FFFFFF');
  assert.equal(board.fill, undefined);

  const painted = F.frame({ name: 'Painted', width: 10, height: 10, fills: [F.solid(0, 0, 0)] });
  F.page(painted);
  const result = await convertSelection([painted], { fileKey: 'FILE' });
  assert.equal(result.nodes[0].fill, '#000000');
});

test('paths that are all open have no fill, as in Figma', async () => {
  const { layers, notes } = await convert(
    F.vector({ name: 'Scribble', width: 10, height: 10, fills: [F.solid(1, 0, 0)], vectorPaths: [{ windingRule: 'NONE', data: 'M 0 0 L 10 10' }] }),
    F.vector({
      name: 'Mixed',
      width: 10,
      height: 10,
      fills: [F.solid(1, 0, 0)],
      vectorPaths: [
        { windingRule: 'NONE', data: 'M 0 0 L 10 10' },
        { windingRule: 'NONZERO', data: 'M 0 0 L 10 0 L 10 10 Z' },
      ],
    }),
  );
  assert.equal(layers[0].fill, undefined);
  assert.equal(layers[1].fill, '#FF0000');
  assert.equal(notes.length, 1);
  assert.match(notes[0].message, /Open lines/);
});

test('a paint that blends differently is noted', async () => {
  const blended = Object.assign(F.solid(1, 0, 0), { blendMode: 'MULTIPLY' });
  const { layers, notes } = await convert(F.rect({ name: 'Blended', width: 10, height: 10, fills: [blended] }));
  assert.equal(layers[0].fill, '#FF0000');
  assert.equal(notes.length, 1);
  assert.match(notes[0].message, /blends as multiply/);
});
