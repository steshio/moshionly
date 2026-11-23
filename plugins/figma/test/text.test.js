'use strict';

// Text: content, font, size, spacing in Moshionly's units, layout and the
// one-style rule.

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

test('a text keeps its words, font and layout', async () => {
  const { layers, notes } = await convert(
    F.text({
      name: 'Title',
      x: 24,
      y: 32,
      width: 300,
      height: 80,
      characters: 'Launch day',
      fontName: { family: 'Inter', style: 'Bold' },
      fontWeight: 700,
      fontSize: 32,
      lineHeight: { unit: 'PIXELS', value: 40 },
      letterSpacing: { unit: 'PIXELS', value: -0.64 },
      textAlignHorizontal: 'CENTER',
      textAlignVertical: 'CENTER',
      textAutoResize: 'HEIGHT',
      textCase: 'UPPER',
      fills: [F.solid(0.07, 0.07, 0.07)],
    }),
  );
  const [title] = layers;
  assert.deepEqual(title, {
    id: title.id,
    parent: title.parent,
    type: 'text',
    name: 'Title',
    x: 24,
    y: 32,
    width: 300,
    height: 80,
    source_id: title.source_id,
    fill: '#121212',
    content: 'Launch day',
    font: { family: 'Inter', weight: 700 },
    font_size: 32,
    line_height: 125,
    letter_spacing: -2,
    align: 'center',
    vertical_align: 'middle',
    auto_size: 'height',
    case: 'upper',
  });
  assert.deepEqual(notes, []);
});

test('defaults are left out: auto line height, no spacing, left and top', async () => {
  const { layers } = await convert(F.text({ name: 'Plain', width: 50, height: 20, characters: 'Hi' }));
  const [plain] = layers;
  assert.equal(plain.line_height, undefined);
  assert.equal(plain.letter_spacing, undefined);
  assert.equal(plain.align, undefined);
  assert.equal(plain.auto_size, undefined, 'width and height follow the text, the default');
  assert.deepEqual(plain.font, { family: 'Inter', weight: 400 });
});

test('fixed boxes, percentages and italics', async () => {
  const { layers } = await convert(
    F.text({
      name: 'Quote',
      width: 200,
      height: 100,
      characters: 'Quiet',
      fontName: { family: 'Source Serif', style: 'Semibold Italic' },
      fontWeight: undefined,
      lineHeight: { unit: 'PERCENT', value: 150 },
      letterSpacing: { unit: 'PERCENT', value: 5 },
      textAutoResize: 'NONE',
      textAlignVertical: 'BOTTOM',
    }),
  );
  const [quote] = layers;
  assert.deepEqual(quote.font, { family: 'Source Serif', weight: 600, italic: true });
  assert.equal(quote.line_height, 150);
  assert.equal(quote.letter_spacing, 5);
  assert.equal(quote.auto_size, 'fixed');
  assert.equal(quote.vertical_align, 'bottom');
});

test('soft line breaks become line breaks', async () => {
  const { layers } = await convert(F.text({ name: 'Lines', width: 10, height: 10, characters: 'one two\r\nthree' }));
  assert.equal(layers[0].content, 'one\ntwo\nthree');
});

test('mixed styles use the style of most of the text and say what else there was', async () => {
  const bold = { family: 'Inter', style: 'Bold' };
  const regular = { family: 'Inter', style: 'Regular' };
  const run = (characters, start, fontName, fontWeight, fontSize, fills) => ({
    characters,
    start,
    end: start + characters.length,
    fontName,
    fontWeight,
    fontSize,
    fills,
    lineHeight: { unit: 'AUTO' },
    letterSpacing: { unit: 'PERCENT', value: 0 },
    textCase: 'ORIGINAL',
    textDecoration: 'NONE',
  });
  const { layers, notes } = await convert(
    F.text({
      name: 'Mixed',
      width: 300,
      height: 40,
      characters: 'Total: 1,250 raised',
      fontName: 'MIXED',
      fontSize: 'MIXED',
      fills: 'MIXED',
      styledTextSegments: [
        run('Total: ', 0, regular, 400, 16, [F.solid(0.4, 0.4, 0.4)]),
        run('1,250', 7, bold, 700, 24, [F.solid(0, 0, 0)]),
        run(' raised', 12, regular, 400, 16, [F.solid(0.4, 0.4, 0.4)]),
      ],
    }),
  );
  const [mixed] = layers;
  assert.deepEqual(mixed.font, { family: 'Inter', weight: 400 });
  assert.equal(mixed.font_size, 16);
  assert.equal(mixed.fill, '#666666');
  assert.equal(notes.length, 1);
  assert.match(notes[0].message, /all of it uses Inter Regular 16 px/);
  assert.match(notes[0].message, /Inter Bold 24 px, another colour/);
});

test('what Moshionly text cannot do is noted', async () => {
  const { layers, notes } = await convert(
    F.text({
      name: 'Fancy',
      width: 100,
      height: 100,
      characters: 'One\nTwo',
      textAlignHorizontal: 'JUSTIFIED',
      textTruncation: 'ENDING',
      textCase: 'SMALL_CAPS',
      textDecoration: 'UNDERLINE',
      paragraphSpacing: 12,
      leadingTrim: 'CAP_HEIGHT',
    }),
  );
  const [fancy] = layers;
  assert.equal(fancy.align, undefined);
  assert.equal(fancy.case, undefined);
  const messages = notes.map((note) => note.message).join('\n');
  for (const words of [/Justified/, /ellipsis/, /Small capitals/, /underline/, /between paragraphs/, /trims the space/]) {
    assert.match(messages, words);
  }
});

test('font weights come from the style name when Figma gives no number', async () => {
  const weights = {};
  for (const style of ['Thin', 'ExtraLight', 'Light', 'Regular', 'Medium', 'SemiBold', 'Bold', 'Extra Bold', 'Black']) {
    const { layers } = await convert(
      F.text({ name: style, width: 10, height: 10, characters: 'a', fontName: { family: 'Any', style }, fontWeight: undefined }),
    );
    weights[style] = layers[0].font.weight;
  }
  assert.deepEqual(weights, {
    Thin: 100,
    ExtraLight: 200,
    Light: 300,
    Regular: 400,
    Medium: 500,
    SemiBold: 600,
    Bold: 700,
    'Extra Bold': 800,
    Black: 900,
  });
});

test('a variable font’s weight axis wins', async () => {
  const { layers } = await convert(
    F.text({ name: 'Variable', width: 10, height: 10, characters: 'a', fontName: { family: 'Inter', style: 'Regular', variationSettings: { wght: 550 } } }),
  );
  assert.equal(layers[0].font.weight, 550);
});
