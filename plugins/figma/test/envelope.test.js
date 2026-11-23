'use strict';

// The envelope Moshionly receives, source ids for sending again, and notes.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection, buildEnvelope, exportSelection } = require('../code.js');
const F = require('../fixtures/figma.js');

const PNG = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 1]);
const loader = {
  async image() {
    return { bytes: PNG, width: 10, height: 10 };
  },
  async render() {
    return PNG;
  },
};

function design() {
  const icon = F.instance({ name: 'Icon', x: 10, y: 10, width: 24, height: 24 }, [
    F.vector({ name: 'Glyph', width: 24, height: 24, vectorPaths: [{ windingRule: 'NONZERO', data: 'M 0 0 L 24 0 L 12 24 Z' }] }),
  ]);
  const card = F.frame({ name: 'Card', x: 100, y: 100, width: 300, height: 200, fills: [F.solid(1, 1, 1)] }, [
    F.rect({ name: 'Photo', width: 300, height: 120, fills: [{ type: 'IMAGE', visible: true, scaleMode: 'FILL', imageHash: 'h' }] }),
    F.text({ name: 'Title', x: 16, y: 136, width: 200, height: 24, characters: 'Hello', textAlignHorizontal: 'JUSTIFIED' }),
    icon,
  ]);
  F.page(card);
  return card;
}

test('the envelope says what it is, where it came from and what was chosen', async () => {
  const card = design();
  const { envelope, json } = await exportSelection([card], {
    fileKey: 'AbC123',
    fileName: 'Launch',
    mode: 'replace_matching',
    loader,
  });
  assert.deepEqual(Object.keys(envelope), ['format', 'version', 'source', 'mode', 'nodes', 'notes']);
  assert.equal(envelope.format, 'moshionly-import');
  assert.equal(envelope.version, 1);
  assert.deepEqual(envelope.source, { app: 'figma', file_name: 'Launch', file_key: 'AbC123' });
  assert.equal(envelope.mode, 'replace_matching');
  assert.equal(envelope.nodes.length, 5);
  assert.ok(envelope.nodes.every((layer) => !String(layer.source || '').startsWith('pending:')));
  assert.deepEqual(JSON.parse(json), envelope);
});

test('an unknown mode is left out, and Moshionly decides', () => {
  const envelope = buildEnvelope({ nodes: [], notes: [] }, { fileKey: 'k', mode: 'merge' });
  assert.equal(envelope.mode, undefined);
  assert.deepEqual(envelope.source, { app: 'figma', file_name: '', file_key: 'k' });
});

test('an envelope is not built while pictures are missing', async () => {
  const result = await convertSelection([design()], { fileKey: 'k' });
  assert.throws(() => buildEnvelope(result, { fileKey: 'k' }), /picture of "Photo" was not fetched/);
});

test('every layer has a source id made of the file key and the Figma node id', async () => {
  const card = design();
  const result = await convertSelection([card], { fileKey: 'AbC123' });
  const ids = result.nodes.map((layer) => layer.source_id);
  assert.equal(new Set(ids).size, ids.length, 'source ids are unique');
  for (const layer of result.nodes) {
    assert.match(layer.source_id, /^figma:AbC123:1:\d+$/);
  }
  assert.equal(result.nodes[0].source_id, 'figma:AbC123:' + card.id);
  // Ids in the design are Figma's, so parents can point at them.
  assert.ok(result.nodes.slice(1).every((layer) => result.nodes.some((other) => other.id === layer.parent)));
});

test('sending the same design twice gives the same layers', async () => {
  const card = design();
  const first = await convertSelection([card], { fileKey: 'k' });
  const second = await convertSelection([card], { fileKey: 'k' });
  assert.deepEqual(second.nodes, first.nodes);
});

test('instances of one component get source ids of their own', async () => {
  const copy = (id) =>
    F.instance({ id, name: 'Button', width: 80, height: 32 }, [F.rect({ id: 'I' + id + ';5:2', name: 'Fill', width: 80, height: 32 })]);
  const row = F.frame({ name: 'Row', width: 300, height: 100 }, [copy('7:1'), copy('7:9')]);
  F.page(row);
  const result = await convertSelection([row], { fileKey: 'k' });
  const fills = result.nodes.filter((layer) => layer.name === 'Fill').map((layer) => layer.source_id);
  assert.deepEqual(fills, ['figma:k:I7:1;5:2', 'figma:k:I7:9;5:2']);
});

test('notes name the layer and its source id', async () => {
  const card = design();
  const result = await convertSelection([card], { fileKey: 'k' });
  const title = card.children[1];
  assert.deepEqual(result.notes, [
    { source_id: 'figma:k:' + title.id, layer_name: 'Title', message: 'Justified text is aligned left; Moshionly does not justify.' },
  ]);
});

test('progress is reported while converting and fetching pictures', async () => {
  const many = F.frame(
    { name: 'Many', width: 1000, height: 1000 },
    Array.from({ length: 250 }, (_, index) => F.rect({ name: 'R' + index, x: index, width: 2, height: 2 })),
  );
  F.page(many);
  const steps = [];
  let pauses = 0;
  await exportSelection([many], {
    fileKey: 'k',
    loader,
    onProgress: (step) => steps.push(step.phase + ' ' + step.done + '/' + step.total),
    pause: async () => {
      pauses += 1;
    },
  });
  assert.deepEqual(steps, ['layers 100/251', 'layers 200/251', 'layers 251/251', 'images 0/0']);
  assert.equal(pauses, 2);
});
