'use strict';

// From Figma's transforms to Moshionly's box, rotation about the centre,
// and coordinates inside the box.

const test = require('node:test');
const assert = require('node:assert/strict');
const { decompose, place, matrixFromFigma, multiply, invert, angle } = require('../code.js');

const near = (actual, expected, message) =>
  assert.ok(Math.abs(actual - expected) < 1e-6, `${message || ''} ${actual} is not ${expected}`);

const box = (width, height) => ({ x: 0, y: 0, width, height });

test('a plain transform is a rotation and an unchanged shape', () => {
  const turned = matrixFromFigma([
    [Math.cos(Math.PI / 6), -Math.sin(Math.PI / 6), 5],
    [Math.sin(Math.PI / 6), Math.cos(Math.PI / 6), 7],
  ]);
  const parts = decompose(turned);
  near(parts.radians, Math.PI / 6);
  near(parts.sx, 1);
  near(parts.sy, 1);
  near(parts.skew, 0);
});

test('a mirror shows up as a negative vertical scale', () => {
  const mirrored = matrixFromFigma([
    [-1, 0, 110],
    [0, 1, 20],
  ]);
  const parts = decompose(mirrored);
  near(Math.abs(parts.radians), Math.PI);
  near(parts.sy, -1);
  near(parts.skew, 0);
});

test('an unturned layer keeps its position and size', () => {
  const placement = place(matrixFromFigma([[1, 0, 10], [0, 1, 20]]), box(100, 50));
  assert.deepEqual(
    [placement.x, placement.y, placement.width, placement.height, placement.rotation],
    [10, 20, 100, 50, 0],
  );
  assert.equal(placement.mirrored, false);
  assert.equal(placement.skewed, false);
  assert.deepEqual(placement.toLocal(30, 40), [30, 40]);
});

test('a turned layer is described by its unturned box and its centre', () => {
  // Turned 90 degrees clockwise about its top-left corner, which Figma
  // reports at (100, 0). On the page it covers x 50 to 100, y 0 to 100.
  const placement = place(matrixFromFigma([[0, -1, 100], [1, 0, 0]]), box(100, 50));
  near(placement.rotation, 90);
  near(placement.x, 25);
  near(placement.y, 25);
  near(placement.width, 100);
  near(placement.height, 50);
  // The layer's own top-left corner lands where Figma's does.
  const [x, y] = [placement.frame.e, placement.frame.f];
  near(x, 100);
  near(y, 0);
});

test('a mirrored layer turns by 180 degrees and flips what it draws', () => {
  // Mirrored left to right in place: the box still covers x 10 to 110.
  const placement = place(matrixFromFigma([[-1, 0, 110], [0, 1, 20]]), box(100, 50));
  assert.equal(placement.mirrored, true);
  near(Math.abs(placement.rotation), 180);
  near(placement.x, 10);
  near(placement.y, 20);
  // Figma's top-left corner of the node is drawn at the box's top-right
  // corner on the page.
  const local = placement.toLocal(0, 0);
  const onPage = [
    placement.frame.a * local[0] + placement.frame.c * local[1] + placement.frame.e,
    placement.frame.b * local[0] + placement.frame.d * local[1] + placement.frame.f,
  ];
  near(onPage[0], 110);
  near(onPage[1], 20);
});

test('a skew is reported', () => {
  const placement = place(matrixFromFigma([[1, 0.5, 0], [0, 1, 0]]), box(10, 10));
  assert.equal(placement.skewed, true);
});

test('matrices compose and invert', () => {
  const m = matrixFromFigma([[0, -1, 100], [1, 0, 30]]);
  const back = multiply(invert(m), m);
  for (const [key, value] of Object.entries({ a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 })) {
    near(back[key], value, key);
  }
});

test('angles are kept between -180 and 180', () => {
  assert.equal(angle(270), -90);
  assert.equal(angle(-180), 180);
  assert.equal(angle(-0.0000001), 0);
  assert.equal(angle(720.5), 0.5);
});
