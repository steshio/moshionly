'use strict';

// Shapes: ellipses and arcs, stars, and everything that becomes a path.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection, parsePathData, starBox } = require('../code.js');
const F = require('../fixtures/figma.js');

/** Converts `layers` inside a 400 by 400 frame and returns the layers. */
async function inFrame(...layers) {
  const board = F.frame({ name: 'Board', width: 400, height: 400 }, layers);
  F.page(board);
  const result = await convertSelection([board], { fileKey: 'FILE' });
  return { layers: result.nodes.slice(1), notes: result.notes };
}

const close = (actual, expected, digits = 2) =>
  assert.ok(
    Math.abs(actual - expected) < Math.pow(10, -digits),
    `${actual} is not ${expected}`,
  );

test('a whole ellipse has no arc settings, a ring keeps its hole', async () => {
  const { layers } = await inFrame(
    F.ellipse({ name: 'Disc', x: 10, y: 10, width: 80, height: 40 }),
    F.ellipse({ name: 'Ring', width: 50, height: 50, arcData: { startingAngle: 0, endingAngle: 2 * Math.PI, innerRadius: 0.6 } }),
  );
  assert.deepEqual(layers[0], {
    id: layers[0].id,
    parent: layers[0].parent,
    type: 'ellipse',
    name: 'Disc',
    x: 10,
    y: 10,
    width: 80,
    height: 40,
    source_id: layers[0].source_id,
  });
  assert.equal(layers[1].inner_radius, 0.6);
  assert.equal(layers[1].sweep, undefined);
});

test('arcs keep their start and sweep, in degrees', async () => {
  const arc = { startingAngle: Math.PI / 2, endingAngle: Math.PI, innerRadius: 0 };
  const { layers } = await inFrame(
    F.ellipse({ name: 'Quarter', width: 100, height: 100, arcData: arc }),
    F.ellipse({ name: 'Mirrored', width: 100, height: 100, mirror: 'x', arcData: arc }),
  );
  assert.equal(layers[0].start_angle, 90);
  assert.equal(layers[0].sweep, 90);
  // Mirrored left to right, the quarter from the bottom to the left becomes
  // the quarter from the bottom to the right. The layer is turned by 180
  // degrees, so in its own terms the arc starts at 270 and runs back 90.
  assert.equal(Math.abs(layers[1].rotation), 180);
  assert.equal(layers[1].start_angle, 270);
  assert.equal(layers[1].sweep, -90);
});

test('a star keeps its points and inner radius, in the box Moshionly draws it in', async () => {
  const { layers } = await inFrame(F.star({ name: 'Star', x: 10, y: 20, width: 100, height: 100 }));
  const [star] = layers;
  assert.equal(star.type, 'star');
  assert.equal(star.points, undefined, '5 points is the default');
  assert.equal(star.inner_radius, 0.382);
  // Figma stretches the star to its box; Moshionly puts its points on the
  // ellipse of its box, so that box is wider and reaches lower.
  close(star.x, 10 + 50 - 100 / 1.902113);
  close(star.y, 20);
  close(star.width, 200 / 1.902113);
  close(star.height, 200 / 1.809017);
  // Its top point is where Figma's is, and its lowest points too.
  close(star.x + star.width / 2, 60);
  close(star.y + (star.height / 2) * (1 + Math.cos(Math.PI / 5)), 120);
});

test('a star mirrored upside down is the same star turned over', async () => {
  const { layers } = await inFrame(F.star({ name: 'Down', x: 10, y: 20, width: 100, height: 100, mirror: 'y' }));
  const [star] = layers;
  assert.equal(star.type, 'star');
  assert.equal(Math.abs(star.rotation), 180);
  // Turned about its centre, its single point is now at the bottom, at the
  // bottom of Figma's box.
  close(star.x + star.width / 2, 60);
  close(star.y + star.height, 120);
});

test('a star box follows the corners for any point count', () => {
  const box = starBox(3, 1, { x: 0, y: 0, width: 90, height: 60 });
  // With an inner radius of 1 a triangle star is a hexagon whose bottom
  // corner reaches the full radius.
  close(box.height, 60);
  close(box.y, 0);
});

test('polygons become closed paths from Figma’s geometry', async () => {
  const { layers } = await inFrame(
    F.polygon({
      name: 'Triangle',
      x: 50,
      y: 50,
      width: 100,
      height: 80,
      fillGeometry: [{ windingRule: 'NONZERO', data: 'M 50 0 L 100 80 L 0 80 L 50 0 Z' }],
    }),
    F.polygon({ name: 'Hexagon', width: 100, height: 100, pointCount: 6 }),
  );
  assert.equal(layers[0].type, 'path');
  assert.deepEqual(layers[0].path, {
    subpaths: [{ anchors: [{ point: [0.5, 0] }, { point: [1, 1] }, { point: [0, 1] }], closed: true }],
  });
  // Without geometry from Figma the polygon is worked out.
  const hexagon = layers[1].path.subpaths[0];
  assert.equal(hexagon.anchors.length, 6);
  assert.deepEqual(hexagon.anchors[0].point, [0.5, 0]);
  assert.equal(hexagon.closed, true);
});

test('vectors keep their curves, open ends and fill rule', async () => {
  const { layers } = await inFrame(
    F.vector({
      name: 'Wave',
      x: 0,
      y: 0,
      width: 200,
      height: 100,
      vectorPaths: [
        { windingRule: 'NONE', data: 'M 0 100 C 50 0 150 0 200 100' },
        { windingRule: 'EVENODD', data: 'M 0 0 L 20 0 L 20 20 Z' },
      ],
    }),
  );
  const [wave] = layers;
  assert.equal(wave.path.fill_rule, 'even_odd');
  assert.deepEqual(wave.path.subpaths[0], {
    anchors: [
      { point: [0, 1], out: [0.25, 0] },
      { point: [1, 1], in: [0.75, 0] },
    ],
  });
  assert.equal(wave.path.subpaths[1].closed, true);
});

test('a mirrored vector is drawn mirrored', async () => {
  const { layers } = await inFrame(
    F.vector({
      name: 'Arrow',
      x: 100,
      y: 100,
      width: 100,
      height: 50,
      mirror: 'x',
      vectorPaths: [{ windingRule: 'NONZERO', data: 'M 0 0 L 100 25 L 0 50 Z' }],
    }),
  );
  const [arrow] = layers;
  // The layer is turned by 180 degrees and its path mirrored top to bottom,
  // which together point the arrow left.
  assert.equal(Math.abs(arrow.rotation), 180);
  assert.deepEqual([arrow.x, arrow.y, arrow.width, arrow.height], [100, 100, 100, 50]);
  assert.deepEqual(
    arrow.path.subpaths[0].anchors.map((anchor) => anchor.point),
    [
      [0, 1],
      [1, 0.5],
      [0, 0],
    ],
  );
});

test('lines are two-point paths with no height', async () => {
  const { layers } = await inFrame(F.line({ name: 'Rule', x: 20, y: 30, width: 120, rotation: 90 }));
  const [line] = layers;
  assert.equal(line.type, 'path');
  assert.equal(line.height, 0);
  assert.equal(line.width, 120);
  assert.equal(line.rotation, 90);
  assert.deepEqual(line.path, { subpaths: [{ anchors: [{ point: [0, 0] }, { point: [1, 0] }] }] });
});

test('boolean shapes become their result, not their parts', async () => {
  const { layers } = await inFrame(
    F.boolean(
      {
        name: 'Ring',
        width: 100,
        height: 100,
        booleanOperation: 'SUBTRACT',
        fillGeometry: [{ windingRule: 'EVENODD', data: 'M 0 0 L 100 0 L 100 100 L 0 100 Z M 25 25 L 75 25 L 75 75 L 25 75 Z' }],
      },
      [F.rect({ name: 'Outer', width: 100, height: 100 }), F.rect({ name: 'Hole', x: 25, y: 25, width: 50, height: 50 })],
    ),
  );
  assert.equal(layers.length, 1, 'the parts are not layers of their own');
  assert.equal(layers[0].type, 'path');
  assert.equal(layers[0].path.fill_rule, 'even_odd');
  assert.equal(layers[0].path.subpaths.length, 2);
});

test('a skewed rectangle becomes a path of its exact outline', async () => {
  const { layers } = await inFrame(F.rect({ name: 'Leaning', x: 0, y: 0, width: 100, height: 100, skew: 0.5 }));
  const [leaning] = layers;
  assert.equal(leaning.type, 'path');
  assert.deepEqual([leaning.width, leaning.height], [150, 100]);
  assert.deepEqual(
    leaning.path.subpaths[0].anchors.map((anchor) => anchor.point),
    [
      [0, 0],
      [0.66667, 0],
      [1, 1],
      [0.33333, 1],
    ],
  );
});

test('path data is read like SVG, with Figma’s closing point folded in', () => {
  assert.deepEqual(parsePathData('M 0 0 L 10 0 L 10 10 L 0 0 Z'), [
    { anchors: [{ point: [0, 0] }, { point: [10, 0] }, { point: [10, 10] }], closed: true },
  ]);
  // Relative commands, H and V, commas and implicit line pairs.
  assert.deepEqual(parsePathData('m1,1 h4 v4 l-4,0 4-4'), [
    {
      anchors: [{ point: [1, 1] }, { point: [5, 1] }, { point: [5, 5] }, { point: [1, 5] }, { point: [5, 1] }],
      closed: false,
    },
  ]);
  // A quadratic curve becomes the equivalent cubic.
  assert.deepEqual(parsePathData('M 0 0 Q 3 3 6 0'), [
    { anchors: [{ point: [0, 0], out: [2, 2] }, { point: [6, 0], in: [4, 2] }], closed: false },
  ]);
  // A closing curve hands its arriving handle to the first anchor.
  const [loop] = parsePathData('M 0 0 C 1 0 2 1 2 2 C 2 3 1 0 0 0 Z');
  assert.deepEqual(loop.anchors[0], { point: [0, 0], out: [1, 0], in: [1, 0] });
  assert.equal(loop.anchors.length, 2);
  // Arcs are not something Figma writes, and are refused.
  assert.equal(parsePathData('M 0 0 A 5 5 0 0 1 10 0'), null);
  assert.equal(parsePathData('M 0'), null);
});
