'use strict';

// Whole designs, end to end without Figma: every layer the plugin sends must
// follow Moshionly's rules (see fixtures/check-layers.js; acceptance/ runs
// the same designs through core's own import code).

const test = require('node:test');
const assert = require('node:assert/strict');
const { exportSelection } = require('../code.js');
const { designs, loader } = require('../fixtures/designs.js');
const { checkLayers } = require('../fixtures/check-layers.js');

async function exported(make) {
  const design = make();
  const { envelope } = await exportSelection(design.selection, { fileKey: 'FixtureFile', fileName: design.name, loader });
  return envelope;
}

const outline = (envelope) => envelope.nodes.map((layer) => layer.type + ' ' + layer.name);

test('every fixture design follows Moshionly’s rules', async () => {
  for (const make of designs) {
    const envelope = await exported(make);
    assert.deepEqual(checkLayers(envelope.nodes), [], make.name);
  }
});

test('the rules check catches what core would refuse', () => {
  const problems = checkLayers([
    { id: 'a', type: 'artboard', width: 0, height: 10, opacity: 50 },
    { id: 'b', parent: 'a', type: 'rect', colour: '#FFF' },
    { id: 'c', parent: 'b', type: 'text', font: { family: '', weight: 400 } },
    { id: 'd', parent: 'a', type: 'image', source: 'photo.png' },
    { id: 'e', parent: 'a', type: 'ellipse', sweep: 400, fill: { type: 'linear_gradient', stops: [{ offset: 0, color: '#000' }] } },
  ]);
  assert.deepEqual(problems, [
    'artboard "" (a): "width" is 0, below 1',
    'artboard "" (a): an artboard cannot have "opacity"',
    'rect "" (b): "colour" is not a field of rect',
    'text "" (c): its parent is a rect, which cannot hold layers',
    'text "" (c): "content" is not text',
    'text "" (c): the font has no family',
    'image "" (d): "source" is not an embedded picture',
    'ellipse "" (e): "fill" has fewer than two stops',
    'ellipse "" (e): "sweep" is 400, above 360',
  ]);
});

test('a product card keeps its structure', async () => {
  const envelope = await exported(designs[0]);
  assert.deepEqual(outline(envelope), [
    'artboard Launch card',
    'group Launch card',
    'image Photo',
    'text Eyebrow',
    'text Title',
    'text Raised',
    'path Divider',
    'group Button',
    'text Label',
    'group Backers',
    'image Backer 1',
    'ellipse Backer 2',
    'ellipse Backer 3',
    'star Badge',
    'path Heart',
  ]);
  assert.equal(envelope.notes.length, 3);
});

test('posters in a section become two artboards with masks, paths and pictures', async () => {
  const envelope = await exported(designs[1]);
  assert.deepEqual(outline(envelope), [
    'artboard Poster A',
    'mask_group Masked photo',
    'ellipse Circle',
    'image Photo',
    'path Triangle',
    'path Ring',
    'ellipse Arc',
    'path Squiggle',
    'rect Glass',
    'image Mirrored',
    'artboard Poster B',
    'image Poster B image',
    'image Reminder',
    'path Leaning',
    'group Tilted',
    'rect Stripe',
    'text Caption',
  ]);
  assert.equal(envelope.notes.length, 6);
});

test('a hero and loose layers come over without notes', async () => {
  const envelope = await exported(designs[2]);
  assert.deepEqual(outline(envelope), [
    'artboard Hero',
    'image Hero image',
    'rect Hero tint',
    'text Headline',
    'group Nav',
    'text Logo',
    'rect Menu',
    'rect Loose shape',
    'text Loose note',
  ]);
  assert.deepEqual(envelope.notes, []);
});
