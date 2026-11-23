'use strict';

// The plugin's Figma side, run against a stand-in for the `figma` global:
// what it tells the panel, and what it sends back when asked to convert.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const F = require('../fixtures/figma.js');

const root = path.join(__dirname, '..');
const code = fs.readFileSync(path.join(root, 'code.js'), 'utf8');
const PNG = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 7, 7, 7]);

/** A plain copy of a value made in the plugin's context, to compare. */
const plain = (value) => JSON.parse(JSON.stringify(value));

/** Runs code.js with a stand-in `figma` and returns it with what was posted to the panel. */
function startWith(selection, overrides) {
  const posted = [];
  const stored = new Map();
  const pluginData = new Map();
  const figma = Object.assign(
    {
      fileKey: 'FileKey1',
      root: {
        name: 'Launch',
        getPluginData: (key) => pluginData.get(key) || '',
        setPluginData: (key, value) => pluginData.set(key, value),
      },
      currentPage: { selection },
      shownWith: null,
      showUI(html, options) {
        this.shownWith = options;
      },
      on() {},
      notify() {},
      closePlugin() {},
      ui: { postMessage: (message) => posted.push(message), onmessage: null },
      clientStorage: {
        getAsync: async (key) => stored.get(key),
        setAsync: async (key, value) => stored.set(key, value),
      },
      getImageByHash: () => ({
        getBytesAsync: async () => PNG,
        getSizeAsync: async () => ({ width: 4000, height: 4000 }),
      }),
      base64Encode: (bytes) => Buffer.from(bytes).toString('base64'),
    },
    overrides,
  );
  vm.runInContext(code, vm.createContext({ figma, __html__: '<html></html>', setTimeout, clearTimeout }));
  return { figma, posted, stored, pluginData };
}

function card() {
  const frame = F.frame({ name: 'Card', width: 200, height: 100, fills: [F.solid(1, 1, 1)] }, [
    F.rect({ name: 'Photo', width: 200, height: 60, fills: [{ type: 'IMAGE', visible: true, scaleMode: 'FILL', imageHash: 'h' }] }),
  ]);
  F.page(frame);
  return frame;
}

test('the panel opens with Figma’s theme colours and learns the selection and mode', async () => {
  const { figma, posted, stored } = startWith([card()]);
  assert.equal(figma.shownWith.themeColors, true);
  stored.set('moshionly.mode', 'replace_matching');
  await figma.ui.onmessage({ type: 'ready' });
  assert.deepEqual(plain(posted[0]), {
    type: 'selection',
    fileName: 'Launch',
    summary: { count: 1, names: ['Card'], more: 0, empty: false, sections: 0 },
  });
  assert.deepEqual(plain(posted[1]), { type: 'mode', mode: 'replace_matching' });

  await figma.ui.onmessage({ type: 'set-mode', mode: 'new_artboards' });
  assert.equal(stored.get('moshionly.mode'), 'new_artboards');
  await figma.ui.onmessage({ type: 'set-mode', mode: 'something else' });
  assert.equal(stored.get('moshionly.mode'), 'new_artboards');
});

test('converting sends the panel the envelope, after the panel shrinks a large picture', async () => {
  const { figma, posted } = startWith([card()]);
  // The panel answers requests to shrink pictures, as ui.html does.
  const panel = figma.ui.postMessage;
  figma.ui.postMessage = (message) => {
    panel(message);
    if (message.type === 'shrink') {
      setTimeout(() => figma.ui.onmessage({ type: 'shrunk', id: message.id, bytes: PNG.slice(0, 5), mime: 'image/png' }), 0);
    }
  };
  await figma.ui.onmessage({ type: 'convert', purpose: 'send', mode: 'replace_matching' });
  const shrink = posted.find((message) => message.type === 'shrink');
  assert.deepEqual([shrink.width, shrink.height], [1024, 1024]);
  const done = posted.find((message) => message.type === 'converted');
  assert.ok(done, JSON.stringify(posted.filter((message) => message.type === 'failed')));
  assert.equal(done.purpose, 'send');
  assert.equal(done.layers, 2);
  const envelope = JSON.parse(done.json);
  assert.equal(envelope.mode, 'replace_matching');
  assert.deepEqual(envelope.source, { app: 'figma', file_name: 'Launch', file_key: 'FileKey1' });
  assert.equal(envelope.nodes[1].source, 'data:image/png;base64,' + Buffer.from(PNG.slice(0, 5)).toString('base64'));
});

test('without a selection the panel is told to select something', async () => {
  const { figma, posted } = startWith([]);
  await figma.ui.onmessage({ type: 'convert', purpose: 'copy' });
  assert.deepEqual(plain(posted.pop()), { type: 'failed', purpose: 'copy', message: 'Select frames or layers to send first.' });
});

test('without a file key from Figma the plugin keeps one in the file', async () => {
  const { figma, posted, pluginData } = startWith([card()], { fileKey: undefined });
  const panel = figma.ui.postMessage;
  figma.ui.postMessage = (message) => {
    panel(message);
    if (message.type === 'shrink') figma.ui.onmessage({ type: 'shrunk', id: message.id, bytes: null });
  };
  await figma.ui.onmessage({ type: 'convert', purpose: 'copy' });
  const key = pluginData.get('moshionly-file-id');
  assert.match(key, /^local-[a-z0-9]{16}$/);
  const envelope = JSON.parse(posted.find((message) => message.type === 'converted').json);
  assert.equal(envelope.source.file_key, key);
  assert.ok(envelope.nodes.every((layer) => layer.source_id.startsWith('figma:' + key + ':')));
});

test('every script in the panel compiles', () => {
  const html = fs.readFileSync(path.join(root, 'ui.html'), 'utf8');
  const scripts = Array.from(html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g), (match) => match[1]);
  assert.equal(scripts.length, 2);
  for (const script of scripts) {
    assert.doesNotThrow(() => new vm.Script(script));
  }
});
