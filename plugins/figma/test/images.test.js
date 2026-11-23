'use strict';

// Image fills, layers sent as pictures of themselves, and fetching the
// pictures.

const test = require('node:test');
const assert = require('node:assert/strict');
const { convertSelection, resolveAssets, encodeBase64, imageMime, keptSize } = require('../code.js');
const F = require('../fixtures/figma.js');

const PNG = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);
const JPEG = Uint8Array.from([0xff, 0xd8, 0xff, 0xe0, 9, 9]);

const imagePaint = (spec) =>
  Object.assign({ type: 'IMAGE', visible: true, opacity: 1, blendMode: 'NORMAL', scaleMode: 'FILL', imageHash: 'photo', rotation: 0 }, spec);

/** A loader that makes pictures up and remembers what it was asked. */
function fakeLoader(images) {
  const asked = { image: [], render: [], shrink: [] };
  return {
    asked,
    async image(hash) {
      asked.image.push(hash);
      if (!images[hash]) throw new Error('no image ' + hash);
      return images[hash];
    },
    async render(node, scale) {
      asked.render.push([node.name, scale]);
      return PNG;
    },
    async shrink(bytes, mime, width, height) {
      asked.shrink.push([mime, width, height]);
      return { bytes: JPEG.slice(0, 4), mime: 'image/jpeg' };
    },
  };
}

async function convert(...layers) {
  const board = F.frame({ name: 'Board', width: 400, height: 400 }, layers);
  F.page(board);
  const result = await convertSelection([board], { fileKey: 'FILE' });
  return Object.assign(result, { layers: result.nodes.slice(1) });
}

test('a rectangle filled with an image becomes an image layer', async () => {
  const result = await convert(
    F.rect({
      name: 'Photo',
      x: 10,
      y: 20,
      width: 300,
      height: 200,
      cornerRadius: 12,
      fills: [imagePaint({ opacity: 0.5 })],
      strokes: [F.solid(1, 1, 1)],
      strokeWeight: 2,
    }),
    F.rect({ name: 'Fitted', width: 100, height: 100, fills: [imagePaint({ scaleMode: 'FIT', imageHash: 'logo' })] }),
  );
  const [photo, fitted] = result.layers;
  assert.equal(photo.type, 'image');
  assert.equal(photo.source, 'pending:0');
  assert.equal(photo.opacity, 50, 'the paint’s opacity fades the layer');
  assert.equal(photo.fit, undefined, '"fill" is Moshionly’s default, cover');
  assert.equal(photo.corner_radius, 12);
  assert.deepEqual(photo.stroke, { paint: '#FFFFFF', width: 2 });
  assert.equal(fitted.fit, 'contain');
  assert.deepEqual(result.notes, []);

  await resolveAssets(result, fakeLoader({ photo: { bytes: JPEG, width: 600, height: 400 }, logo: { bytes: PNG, width: 64, height: 64 } }));
  assert.equal(photo.source, 'data:image/jpeg;base64,' + Buffer.from(JPEG).toString('base64'));
  assert.equal(fitted.source, 'data:image/png;base64,' + Buffer.from(PNG).toString('base64'));
});

test('an image covered by an opaque fill is not there', async () => {
  const result = await convert(F.rect({ name: 'Covered', width: 10, height: 10, fills: [imagePaint(), F.solid(0, 0, 0)] }));
  assert.equal(result.layers[0].type, 'rect');
  assert.equal(result.layers[0].fill, '#000000');
});

test('cropped images, images in other shapes and tinted images are sent as pictures', async () => {
  const result = await convert(
    F.rect({ name: 'Cropped', x: 5, y: 6, width: 100, height: 50, fills: [imagePaint({ scaleMode: 'CROP' })] }),
    F.ellipse({ name: 'Avatar', width: 40, height: 40, fills: [imagePaint()] }),
    F.rect({ name: 'Tinted', width: 40, height: 40, fills: [imagePaint(), F.solid(0, 0, 1, 0.3)] }),
  );
  assert.deepEqual(
    result.layers.map((layer) => [layer.name, layer.type, layer.fit]),
    [
      ['Cropped', 'image', 'stretch'],
      ['Avatar', 'image', 'stretch'],
      ['Tinted', 'image', 'stretch'],
    ],
  );
  assert.deepEqual([result.layers[0].x, result.layers[0].y, result.layers[0].width], [5, 6, 100]);
  assert.deepEqual(
    result.notes.map((note) => note.message),
    [
      'Sent as a picture because its image is cropped. It looks the same, but is one image in Moshionly.',
      'Sent as a picture because Moshionly draws images in rectangles only. It looks the same, but is one image in Moshionly.',
      'Sent as a picture because something is painted over its image. It looks the same, but is one image in Moshionly.',
    ],
  );
  assert.deepEqual(
    result.assets.map((asset) => asset.kind),
    ['render', 'render', 'render'],
  );
  const loader = fakeLoader({});
  await resolveAssets(result, loader);
  assert.deepEqual(loader.asked.render, [
    ['Cropped', 2],
    ['Avatar', 2],
    ['Tinted', 2],
  ]);
  assert.ok(result.layers.every((layer) => layer.source.startsWith('data:image/png;base64,')));
});

test('a picture covers what Figma draws, shadows included, upright on the page', async () => {
  const shadowed = F.rect({
    name: 'Card',
    width: 100,
    height: 100,
    fills: [F.solid(1, 1, 1)],
    complexStrokeProperties: { type: 'BRUSH' },
    strokes: [F.solid(0, 0, 0)],
    strokeWeight: 4,
    absoluteRenderBounds: { x: 90, y: 95, width: 130, height: 140 },
  });
  const holder = F.group({ name: 'Holder', x: 100, y: 100, width: 100, height: 100, rotation: 30 }, [shadowed]);
  const result = await convert(holder);
  const picture = result.layers[1];
  assert.equal(picture.type, 'image');
  // Upright on the page means turned back against the holder's rotation.
  assert.equal(picture.rotation, -30);
  assert.equal(picture.width, 130);
  assert.equal(picture.height, 140);
  assert.match(result.notes[0].message, /brush stroke/);
});

test('mirrored text, unknown layers and video are sent as pictures', async () => {
  const result = await convert(
    F.text({ name: 'Backwards', width: 50, height: 20, characters: 'Hi', mirror: 'x' }),
    F.node('STICKY', { name: 'Sticky note', width: 200, height: 200 }),
    F.rect({ name: 'Clip', width: 160, height: 90, fills: [{ type: 'VIDEO', visible: true, scaleMode: 'FILL', videoHash: 'v' }] }),
  );
  assert.deepEqual(
    result.layers.map((layer) => layer.type),
    ['image', 'image', 'image'],
  );
  const messages = result.notes.map((note) => note.message).join('\n');
  assert.match(messages, /cannot mirror, skew or stretch text/);
  assert.match(messages, /no layer like a sticky/);
  assert.match(messages, /no video fills/);
});

test('a frame’s image fill becomes an image at the back of its group', async () => {
  const hero = F.frame(
    {
      name: 'Hero',
      x: 0,
      y: 0,
      width: 300,
      height: 200,
      cornerRadius: 16,
      fills: [F.solid(0, 0, 0), imagePaint({ scaleMode: 'FIT' }), F.solid(0, 0, 0, 0.4)],
    },
    [F.text({ name: 'Headline', x: 20, y: 20, width: 100, height: 30, characters: 'Hello' })],
  );
  const result = await convert(hero);
  assert.deepEqual(
    result.layers.map((layer) => [layer.id, layer.type, layer.parent]),
    [
      [hero.id, 'group', result.nodes[0].id],
      [hero.id + '/fill', 'image', hero.id],
      [hero.id + '/overlay', 'rect', hero.id],
      [hero.children[0].id, 'text', hero.id],
    ],
  );
  const [group, image, tint] = result.layers;
  assert.equal(group.fill, '#000000', 'the paint under the image');
  assert.equal(image.name, 'Hero image');
  assert.equal(image.fit, 'contain');
  assert.equal(image.corner_radius, 16);
  assert.deepEqual([image.x, image.y, image.width, image.height], [0, 0, 300, 200]);
  assert.equal(image.source_id, 'figma:FILE:' + hero.id + '/fill');
  assert.equal(tint.fill, '#00000066');
  assert.equal(tint.corner_radius, 16);
  assert.deepEqual(result.notes, []);
});

test('a top-level frame with an image fill keeps it on its artboard', async () => {
  const poster = F.frame({ name: 'Poster', width: 1080, height: 1350, fills: [imagePaint({ scaleMode: 'TILE' })] });
  F.page(poster);
  const result = await convertSelection([poster], { fileKey: 'FILE' });
  assert.deepEqual(
    result.nodes.map((layer) => [layer.type, layer.parent]),
    [
      ['artboard', undefined],
      ['image', poster.id],
    ],
  );
  assert.match(result.notes[0].message, /background image is tiled/);
});

test('pictures that cannot be fetched arrive empty, with a note', async () => {
  const result = await convert(F.rect({ name: 'Missing', width: 10, height: 10, fills: [imagePaint({ imageHash: 'gone' })] }));
  await resolveAssets(result, fakeLoader({}));
  assert.match(result.layers[0].source, /^data:image\/png;base64,/);
  assert.match(result.notes[0].message, /could not export this picture \(no image gone\)/);
});

test('large images are shrunk to what the layer needs, and fetched once', async () => {
  const result = await convert(
    F.rect({ name: 'Thumb', width: 300, height: 200, fills: [imagePaint({ imageHash: 'big' })] }),
    F.rect({ name: 'Thumb again', width: 300, height: 200, fills: [imagePaint({ imageHash: 'big' })] }),
  );
  const loader = fakeLoader({ big: { bytes: JPEG, width: 4000, height: 3000 } });
  const progress = [];
  await resolveAssets(result, loader, { onProgress: (step) => progress.push(step.done + '/' + step.total) });
  assert.deepEqual(loader.asked.image, ['big']);
  assert.deepEqual(loader.asked.shrink, [['image/jpeg', 1024, 768]]);
  assert.equal(result.layers[0].source, result.layers[1].source);
  assert.deepEqual(progress, ['0/2', '1/2', '2/2']);
});

test('the kept size is twice what is shown, within limits', () => {
  // Shown at 7.5% of its size; twice that is under the 1024 pixel floor.
  assert.deepEqual(keptSize(4000, 3000, { fit: 'cover', width: 300, height: 200 }), { width: 1024, height: 768 });
  // A big layer keeps a big picture, up to 4096 pixels.
  assert.deepEqual(keptSize(10000, 5000, { fit: 'cover', width: 3000, height: 1500 }), { width: 4096, height: 2048 });
  // Close to what is needed: left alone.
  assert.equal(keptSize(800, 600, { fit: 'cover', width: 400, height: 300 }), null);
});

test('base64 and image types are worked out without help', () => {
  for (const length of [0, 1, 2, 3, 4, 5, 255]) {
    const bytes = Uint8Array.from({ length }, (_, index) => (index * 37) % 256);
    assert.equal(encodeBase64(bytes), Buffer.from(bytes).toString('base64'));
  }
  assert.equal(imageMime(PNG), 'image/png');
  assert.equal(imageMime(JPEG), 'image/jpeg');
  assert.equal(imageMime(Uint8Array.from([0x47, 0x49, 0x46, 0x38])), 'image/gif');
  assert.equal(imageMime(Uint8Array.from([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50])), 'image/webp');
});
