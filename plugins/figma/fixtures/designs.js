'use strict';

// Designs like the ones people make in Figma, written as Figma nodes. The
// tests convert them, and `tools/convert-fixtures.js` turns them into the
// envelopes that `acceptance/` checks with Moshionly's own import code.
//
// Every design is a function, so each test gets fresh nodes.

const F = require('./figma.js');

/** A transparent PNG of one pixel: every picture in the fixtures. */
const PICTURE = Uint8Array.from(
  Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=', 'base64'),
);

/** Fetches pictures for the fixtures: every image is the same tiny PNG. */
const loader = {
  async image() {
    return { bytes: PICTURE, width: 1200, height: 800 };
  },
  async render() {
    return PICTURE;
  },
};

const white = F.solid(1, 1, 1);
const ink = F.solid(0.07, 0.07, 0.08);
const blue = F.solid(0.05, 0.6, 1);

const stop = (position, r, g, b, a) => ({ position, color: { r, g, b, a: a === undefined ? 1 : a } });

const gradient = (type, transform, stops) => ({
  type,
  visible: true,
  opacity: 1,
  blendMode: 'NORMAL',
  gradientTransform: transform,
  gradientStops: stops,
});

const image = (hash, scaleMode) => ({
  type: 'IMAGE',
  visible: true,
  opacity: 1,
  blendMode: 'NORMAL',
  scaleMode: scaleMode || 'FILL',
  imageHash: hash,
  rotation: 0,
});

const dropShadow = (y, radius, alpha) => ({
  type: 'DROP_SHADOW',
  visible: true,
  blendMode: 'NORMAL',
  color: { r: 0, g: 0, b: 0, a: alpha },
  offset: { x: 0, y },
  radius,
  spread: 0,
});

const bold = { family: 'Inter', style: 'Bold' };
const regular = { family: 'Inter', style: 'Regular' };

/** A run of text with one style, as `getStyledTextSegments` reports it. */
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

/** A product card: a styled top-level frame with most kinds of layer in it. */
function launchCard() {
  const card = F.frame(
    {
      name: 'Launch card',
      x: 0,
      y: 0,
      width: 360,
      height: 480,
      cornerRadius: 24,
      fills: [white],
      effects: [dropShadow(24, 64, 0.18)],
    },
    [
      F.rect({
        name: 'Photo',
        width: 360,
        height: 220,
        topLeftRadius: 24,
        topRightRadius: 24,
        bottomRightRadius: 0,
        bottomLeftRadius: 0,
        fills: [image('photo')],
      }),
      F.text({
        name: 'Eyebrow',
        x: 24,
        y: 240,
        width: 60,
        height: 16,
        characters: 'New',
        fontName: { family: 'Inter', style: 'Semi Bold' },
        fontWeight: 600,
        fontSize: 12,
        letterSpacing: { unit: 'PERCENT', value: 8 },
        textCase: 'UPPER',
        fills: [blue],
      }),
      F.text({
        name: 'Title',
        x: 24,
        y: 262,
        width: 312,
        height: 40,
        characters: 'Launch day',
        fontName: bold,
        fontWeight: 700,
        fontSize: 32,
        lineHeight: { unit: 'PIXELS', value: 40 },
        textAutoResize: 'HEIGHT',
        fills: [ink],
      }),
      F.text({
        name: 'Raised',
        x: 24,
        y: 320,
        width: 220,
        height: 30,
        characters: '$12,480 raised so far',
        fontName: 'MIXED',
        fontSize: 'MIXED',
        fills: 'MIXED',
        styledTextSegments: [
          run('$12,480', 0, bold, 700, 24, [ink]),
          run(' raised so far', 7, regular, 400, 16, [F.solid(0.4, 0.4, 0.45)]),
        ],
      }),
      F.line({ name: 'Divider', x: 24, y: 400, width: 312, strokes: [F.solid(0.93, 0.93, 0.94)], strokeWeight: 1 }),
      F.instance(
        {
          name: 'Button',
          x: 24,
          y: 416,
          width: 140,
          height: 40,
          cornerRadius: 20,
          clipsContent: true,
          fills: [gradient('GRADIENT_LINEAR', [[1, 0, 0], [0, 1, 0]], [stop(0, 0.05, 0.6, 1), stop(1, 0.48, 0.38, 1)])],
        },
        [
          F.text({
            name: 'Label',
            x: 0,
            y: 12,
            width: 140,
            height: 16,
            characters: 'Back it',
            fontName: { family: 'Inter', style: 'Medium' },
            fontWeight: 500,
            textAlignHorizontal: 'CENTER',
            textAutoResize: 'NONE',
            fills: [white],
          }),
        ],
      ),
      F.group({ name: 'Backers', x: 220, y: 416, width: 112, height: 40 }, [
        F.ellipse({ name: 'Backer 1', width: 40, height: 40, fills: [image('face')] }),
        F.ellipse({
          name: 'Backer 2',
          x: 36,
          width: 40,
          height: 40,
          fills: [F.solid(1, 0.8, 0.2)],
          strokes: [white],
          strokeWeight: 2,
          strokeAlign: 'OUTSIDE',
        }),
        F.ellipse({
          name: 'Backer 3',
          x: 72,
          width: 40,
          height: 40,
          fills: [gradient('GRADIENT_RADIAL', [[1, 0, 0], [0, 1, 0]], [stop(0, 1, 0.5, 0.6), stop(1, 0.6, 0.2, 0.9)])],
        }),
      ]),
      F.star({ name: 'Badge', x: 300, y: 20, width: 40, height: 40, rotation: 15, fills: [F.solid(1, 0.84, 0)] }),
      F.vector({
        name: 'Heart',
        x: 316,
        y: 240,
        width: 20,
        height: 18,
        fills: [F.solid(0.95, 0.25, 0.35)],
        vectorPaths: [
          {
            windingRule: 'NONZERO',
            data: 'M 10 18 C 4 13 0 9 0 5 C 0 2 2 0 5 0 C 7 0 9 1 10 3 C 11 1 13 0 15 0 C 18 0 20 2 20 5 C 20 9 16 13 10 18 Z',
          },
        ],
      }),
    ],
  );
  F.page(card);
  return { slug: 'launch-card', name: 'Launch card', selection: [card] };
}

/** Two posters in a section: masks, path shapes, effects and pictures. */
function posters() {
  const posterA = F.frame(
    {
      name: 'Poster A',
      x: 100,
      y: 100,
      width: 600,
      height: 600,
      fills: [gradient('GRADIENT_LINEAR', [[0.5, 0.5, 0], [-0.5, 0.5, 0.5]], [stop(0, 0.1, 0.1, 0.3), stop(1, 0.9, 0.3, 0.5)])],
    },
    [
      F.group({ name: 'Masked photo', x: 60, y: 60, width: 240, height: 240 }, [
        F.ellipse({ name: 'Circle', width: 240, height: 240, isMask: true, fills: [F.solid(0, 0, 0)] }),
        F.rect({ name: 'Photo', x: -40, y: 0, width: 320, height: 240, fills: [image('poster-photo')] }),
      ]),
      F.polygon({ name: 'Triangle', x: 380, y: 80, width: 120, height: 100, rotation: -20, mirror: 'x', fills: [F.solid(1, 0.9, 0.3)] }),
      F.boolean(
        {
          name: 'Ring',
          x: 400,
          y: 300,
          width: 120,
          height: 120,
          booleanOperation: 'SUBTRACT',
          fills: [white],
          fillGeometry: [
            {
              windingRule: 'EVENODD',
              data:
                'M 120 60 C 120 93.1 93.1 120 60 120 C 26.9 120 0 93.1 0 60 C 0 26.9 26.9 0 60 0 C 93.1 0 120 26.9 120 60 Z ' +
                'M 90 60 C 90 76.6 76.6 90 60 90 C 43.4 90 30 76.6 30 60 C 30 43.4 43.4 30 60 30 C 76.6 30 90 43.4 90 60 Z',
            },
          ],
        },
        [
          F.ellipse({ name: 'Outer', width: 120, height: 120 }),
          F.ellipse({ name: 'Inner', x: 30, y: 30, width: 60, height: 60 }),
        ],
      ),
      F.ellipse({
        name: 'Arc',
        x: 80,
        y: 380,
        width: 160,
        height: 160,
        arcData: { startingAngle: Math.PI / 4, endingAngle: (3 * Math.PI) / 2, innerRadius: 0.6 },
        fills: [F.solid(0.3, 0.9, 0.7)],
      }),
      F.vector({
        name: 'Squiggle',
        x: 300,
        y: 480,
        width: 240,
        height: 60,
        vectorPaths: [{ windingRule: 'NONE', data: 'M 0 30 C 40 0 80 0 120 30 C 160 60 200 60 240 30' }],
        strokes: [white],
        strokeWeight: 6,
        strokeCap: 'ROUND',
        strokeJoin: 'ROUND',
        dashPattern: [12, 8],
      }),
      F.rect({
        name: 'Glass',
        x: 320,
        y: 40,
        width: 220,
        height: 120,
        cornerRadius: 16,
        fills: [F.solid(1, 1, 1, 0.2)],
        effects: [
          { type: 'BACKGROUND_BLUR', visible: true, radius: 20 },
          Object.assign(dropShadow(2, 4, 0.3), { type: 'INNER_SHADOW' }),
          { type: 'LAYER_BLUR', blurType: 'NORMAL', visible: true, radius: 2 },
        ],
      }),
      F.text({ name: 'Mirrored', x: 40, y: 540, width: 200, height: 30, characters: 'Mirror', fontSize: 24, mirror: 'x' }),
    ],
  );
  const posterB = F.frame({ name: 'Poster B', x: 800, y: 100, width: 600, height: 600, fills: [image('pattern', 'TILE')] }, [
    F.node('STICKY', { name: 'Reminder', x: 40, y: 40, width: 200, height: 200 }),
    F.rect({ name: 'Leaning', x: 300, y: 300, width: 100, height: 100, skew: 0.4, fills: [F.solid(0.2, 0.2, 0.9)] }),
    F.frame({ name: 'Tilted', x: 120, y: 320, width: 200, height: 160, rotation: 12, cornerRadius: 12, fills: [white] }, [
      F.rect({ name: 'Stripe', x: -20, y: 60, width: 240, height: 40, fills: [blue] }),
      F.text({ name: 'Caption', x: 16, y: 16, width: 168, height: 20, characters: 'Tilted card', textAutoResize: 'NONE' }),
    ]),
  ]);
  const area = F.section({ name: 'Posters', x: -100, y: -100, width: 1600, height: 800 }, [posterA, posterB]);
  F.page(area);
  return { slug: 'posters', name: 'Posters', selection: [area] };
}

/** A hero frame with a tinted photo behind it, and two loose layers. */
function hero() {
  const frame = F.frame(
    {
      name: 'Hero',
      x: 0,
      y: 0,
      width: 1440,
      height: 720,
      fills: [F.solid(0.1, 0.1, 0.1), image('hero'), F.solid(0, 0, 0, 0.35)],
    },
    [
      F.text({
        name: 'Headline',
        x: 120,
        y: 260,
        width: 800,
        height: 120,
        characters: 'Make things move',
        fontName: bold,
        fontWeight: 700,
        fontSize: 96,
        lineHeight: { unit: 'PERCENT', value: 110 },
        letterSpacing: { unit: 'PIXELS', value: -2 },
        fills: [white],
      }),
      F.frame({ name: 'Nav', width: 1440, height: 80, clipsContent: false }, [
        F.text({ name: 'Logo', x: 40, y: 28, width: 120, height: 24, characters: 'Moshi', fontSize: 20, fills: [white] }),
        F.rect({ name: 'Menu', x: 1360, y: 28, width: 40, height: 24, cornerRadius: 4, fills: [white] }),
      ]),
    ],
  );
  const shape = F.rect({
    name: 'Loose shape',
    x: 1600,
    y: 100,
    width: 200,
    height: 120,
    rotation: 30,
    fills: [blue],
    strokes: [ink],
    strokeWeight: 4,
    strokeAlign: 'INSIDE',
  });
  const note = F.text({ name: 'Loose note', x: 1600, y: 400, width: 200, height: 40, characters: 'Two\nlines' });
  F.page(frame, shape, note);
  return { slug: 'hero', name: 'Hero', selection: [frame, shape, note] };
}

module.exports = { designs: [launchCard, posters, hero], loader, PICTURE };
