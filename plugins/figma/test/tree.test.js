'use strict';

// The tree: which Figma nodes become artboards and groups, how children are
// placed relative to them, and which nodes of a selection are converted.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection } = require('../code.js');
const { page, frame, group, instance, section, rect, node } = require('../fixtures/figma.js');

const convert = (selection) => convertSelection(selection, { fileKey: 'FILE' });
const byName = (result, name) => result.nodes.find((layer) => layer.name === name);

test('a frame on the canvas becomes an artboard at the same place', async () => {
  const card = frame({ name: 'Card', x: 400, y: -200, width: 300, height: 200 }, [
    rect({ name: 'Background', x: 0, y: 0, width: 300, height: 200 }),
    rect({ name: 'Badge', x: 240, y: 20, width: 40, height: 40, cornerRadius: 8 }),
  ]);
  page(card);
  const result = await convert([card]);

  assert.deepEqual(result.nodes[0], {
    id: card.id,
    type: 'artboard',
    name: 'Card',
    x: 400,
    y: -200,
    width: 300,
    height: 200,
    source_id: 'figma:FILE:' + card.id,
  });
  const badge = byName(result, 'Badge');
  assert.equal(badge.parent, card.id);
  assert.equal(badge.type, 'rect');
  assert.deepEqual([badge.x, badge.y, badge.width, badge.height], [240, 20, 40, 40]);
  assert.equal(badge.corner_radius, 8);
  assert.equal(result.nodes.length, 3);
  assert.deepEqual(result.notes, []);
});

test('frames inside frames become groups that clip', async () => {
  const inner = frame({ name: 'Inner', x: 20, y: 30, width: 100, height: 80, topLeftRadius: 4, topRightRadius: 4, bottomRightRadius: 0, bottomLeftRadius: 0 }, [
    rect({ name: 'Dot', x: 10, y: 10, width: 5, height: 5 }),
  ]);
  const outer = frame({ name: 'Outer', x: 0, y: 0, width: 400, height: 300 }, [inner]);
  page(outer);
  const result = await convert([outer]);
  const group = byName(result, 'Inner');
  assert.equal(group.type, 'group');
  assert.equal(group.clip, true);
  assert.deepEqual(group.corner_radius, [4, 4, 0, 0]);
  assert.deepEqual([group.x, group.y], [20, 30]);
  const dot = byName(result, 'Dot');
  assert.equal(dot.parent, inner.id);
  assert.deepEqual([dot.x, dot.y], [10, 10], 'children are placed inside their group');
});

test('groups and instances become plain groups', async () => {
  const icon = instance({ name: 'Icon', x: 50, y: 60, width: 24, height: 24 }, [
    rect({ name: 'Glyph', x: 2, y: 2, width: 20, height: 20 }),
  ]);
  const cluster = group({ name: 'Cluster', x: 10, y: 10, width: 100, height: 100 }, [icon]);
  const screen = frame({ name: 'Screen', width: 375, height: 812 }, [cluster]);
  page(screen);
  const result = await convert([screen]);
  assert.equal(byName(result, 'Cluster').type, 'group');
  assert.equal(byName(result, 'Cluster').clip, undefined);
  assert.equal(byName(result, 'Icon').type, 'group');
  assert.deepEqual([byName(result, 'Icon').x, byName(result, 'Icon').y], [50, 60]);
  assert.equal(byName(result, 'Glyph').parent, icon.id);
});

test('a top-level frame with round corners keeps them on a group inside the artboard', async () => {
  const card = frame({ name: 'Card', x: 0, y: 0, width: 200, height: 100, cornerRadius: 16, opacity: 0.5 }, [
    rect({ name: 'Line', x: 0, y: 50, width: 200, height: 1 }),
  ]);
  page(card);
  const result = await convert([card]);
  const [board, body, line] = result.nodes;
  assert.equal(board.type, 'artboard');
  assert.equal(board.opacity, undefined, 'artboards cannot fade');
  assert.equal(body.type, 'group');
  assert.equal(body.parent, board.id);
  assert.equal(body.id, card.id + '/frame');
  assert.equal(body.source_id, 'figma:FILE:' + card.id + '/frame');
  assert.deepEqual([body.x, body.y, body.width, body.height], [0, 0, 200, 100]);
  assert.equal(body.corner_radius, 16);
  assert.equal(body.opacity, 50);
  assert.equal(body.clip, true);
  assert.equal(line.parent, body.id);
});

test('rotation is relative to the parent', async () => {
  const child = rect({ name: 'Child', x: 50, y: 0, width: 20, height: 10, rotation: 30 });
  const holder = group({ name: 'Holder', x: 100, y: 100, width: 100, height: 100, rotation: 45 }, [child]);
  const board = frame({ name: 'Board', width: 500, height: 500 }, [holder]);
  page(board);
  const result = await convert([board]);
  const placed = byName(result, 'Child');
  assert.equal(placed.rotation, 30);
  // Figma turns the child about its top-left corner, at (50, 0) in the
  // holder's turned coordinates, so its centre is at (50, 0) plus (10, 5)
  // turned by 30 degrees. Moshionly describes the unturned box around that
  // centre.
  const cos = Math.cos(Math.PI / 6);
  const sin = Math.sin(Math.PI / 6);
  const center = [50 + 10 * cos - 5 * sin, 10 * sin + 5 * cos];
  assert.deepEqual(
    [placed.x, placed.y, placed.width, placed.height],
    [+(center[0] - 10).toFixed(3), +(center[1] - 5).toFixed(3), 20, 10],
  );
  assert.equal(byName(result, 'Holder').rotation, 45);
});

test('loose layers on the canvas keep their canvas position', async () => {
  const loose = rect({ name: 'Loose', x: 12, y: 34, width: 56, height: 78 });
  page(loose);
  const result = await convert([loose]);
  assert.equal(result.nodes.length, 1);
  assert.equal(result.nodes[0].parent, undefined);
  assert.deepEqual([result.nodes[0].x, result.nodes[0].y], [12, 34]);
});

test('a nested frame selected on its own is a group, not an artboard', async () => {
  const inner = frame({ name: 'Inner', x: 20, y: 20, width: 50, height: 50 });
  const outer = frame({ name: 'Outer', width: 200, height: 200 }, [inner]);
  page(outer);
  const result = await convert([inner]);
  assert.equal(result.nodes[0].type, 'group');
  assert.deepEqual([result.nodes[0].x, result.nodes[0].y], [20, 20]);
});

test('a selected layer inside another selected layer is sent once', async () => {
  const child = rect({ name: 'Child', width: 10, height: 10 });
  const parent = frame({ name: 'Parent', width: 100, height: 100 }, [child]);
  page(parent);
  const result = await convert([child, parent]);
  assert.deepEqual(
    result.nodes.map((layer) => layer.name),
    ['Parent', 'Child'],
  );
});

test('sections open up into the frames they hold, which become artboards', async () => {
  const first = frame({ name: 'First', x: 0, y: 0, width: 100, height: 100 });
  const second = frame({ name: 'Second', x: 200, y: 0, width: 100, height: 100 });
  const area = section({ name: 'Area', x: -50, y: -50, width: 500, height: 300 }, [first, second]);
  page(area);
  const result = await convert([area]);
  assert.deepEqual(
    result.nodes.map((layer) => [layer.name, layer.type, layer.x]),
    [
      ['First', 'artboard', -50],
      ['Second', 'artboard', 150],
    ],
  );
});

test('slices are not layers', async () => {
  const slice = node('SLICE', { name: 'Export area', width: 10, height: 10 });
  page(slice);
  const result = await convert([slice]);
  assert.deepEqual(result.nodes, []);
});

test('hidden and locked layers stay hidden and locked', async () => {
  const shy = rect({ name: 'Shy', width: 10, height: 10, visible: false, locked: true });
  const board = frame({ name: 'Board', width: 100, height: 100 }, [shy]);
  page(board);
  const result = await convert([board]);
  assert.equal(byName(result, 'Shy').hidden, true);
  assert.equal(byName(result, 'Shy').locked, true);
});

test('blend modes map onto Moshionly’s, and substitutes are noted', async () => {
  const dodge = rect({ name: 'Dodge', width: 10, height: 10, blendMode: 'LINEAR_DODGE' });
  const multiplied = rect({ name: 'Multiply', width: 10, height: 10, blendMode: 'MULTIPLY' });
  const board = frame({ name: 'Board', width: 100, height: 100 }, [dodge, multiplied]);
  page(board);
  const result = await convert([board]);
  assert.equal(byName(result, 'Multiply').blend_mode, 'multiply');
  assert.equal(byName(result, 'Dodge').blend_mode, 'screen');
  assert.equal(result.notes.length, 1);
  assert.equal(result.notes[0].layer_name, 'Dodge');
  assert.match(result.notes[0].message, /linear dodge/);
});

test('a turned top-level frame arrives upright, with a note', async () => {
  const board = frame({ name: 'Tilted', x: 0, y: 0, width: 100, height: 100, rotation: 10 });
  page(board);
  const result = await convert([board]);
  assert.equal(result.nodes[0].rotation, undefined);
  assert.match(result.notes[0].message, /upright/);
});

test('an auto layout frame with its first layer on top keeps that stacking', async () => {
  const avatars = (spec) =>
    frame(Object.assign({ name: 'Avatars', width: 100, height: 40, itemSpacing: -10 }, spec), [
      rect({ name: 'First', x: 0, width: 40, height: 40 }),
      rect({ name: 'Second', x: 30, width: 40, height: 40 }),
      rect({ name: 'Third', x: 60, width: 40, height: 40 }),
    ]);
  const firstOnTop = avatars({ layoutMode: 'HORIZONTAL', itemReverseZIndex: true });
  const plain = avatars({ name: 'Plain', layoutMode: 'NONE', itemReverseZIndex: true });
  const board = frame({ name: 'Board', width: 400, height: 400 }, [firstOnTop, plain]);
  page(board);
  const result = await convert([board]);
  const order = (holder) => result.nodes.filter((layer) => layer.parent === holder.id).map((layer) => layer.name);
  assert.deepEqual(order(firstOnTop), ['Third', 'Second', 'First'], 'drawn back to front');
  assert.deepEqual(order(plain), ['First', 'Second', 'Third'], 'Figma ignores the setting without auto layout');
});

test('a top-level frame that does not clip gets a note when something sticks out of it', async () => {
  const notesFor = async (clipsContent, children) => {
    const board = frame({ name: 'Open', x: 100, y: 100, width: 400, height: 400, clipsContent }, children);
    page(board);
    return (await convert([board])).notes.map((note) => note.message);
  };
  const out = () => rect({ name: 'Out', x: 380, y: 20, width: 50, height: 50 });
  const sticking = await notesFor(false, [out()]);
  assert.equal(sticking.length, 1);
  assert.match(sticking[0], /Clip content is off/);
  assert.deepEqual(await notesFor(true, [out()]), [], 'a frame that clips cuts it off in Figma too');
  assert.deepEqual(await notesFor(false, [rect({ name: 'In', x: 10, y: 10, width: 380, height: 380 })]), []);
  assert.deepEqual(await notesFor(false, [rect({ name: 'Edge', x: 0, y: 0, width: 400.3, height: 400 })]), [], 'a fraction of a pixel does not count');
  assert.deepEqual(await notesFor(false, [Object.assign(out(), { visible: false })]), [], 'hidden layers show nowhere');
  assert.deepEqual(
    await notesFor(false, [frame({ name: 'Window', x: 300, y: 0, width: 100, height: 100 }, [out()])]),
    [],
    'a frame inside that clips cuts it off first',
  );
  assert.equal(
    (await notesFor(false, [group({ name: 'Cluster', x: 300, y: 0, width: 100, height: 100 }, [rect({ name: 'Far', x: 90, y: 0, width: 50, height: 50 })])])).length,
    1,
    'layers inside a group count',
  );
  // A shadow reaching past the edge, as Figma reports it in the render bounds.
  const shadowed = rect({ name: 'Shadowed', x: 350, y: 10, width: 40, height: 40, absoluteRenderBounds: { x: 440, y: 105, width: 70, height: 60 } });
  assert.equal((await notesFor(false, [shadowed])).length, 1);
});
