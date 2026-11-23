'use strict';

// The manifest is what Figma reads to load the plugin, so its rules are
// checked here: the files it names exist, and the plugin may reach the
// Moshionly app on this computer and nothing else.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));

test('the manifest names files that exist', () => {
  for (const file of [manifest.main, manifest.ui]) {
    assert.ok(fs.existsSync(path.join(root, file)), `${file} is missing`);
  }
});

test('the plugin loads pages on demand, as Figma requires of new plugins', () => {
  assert.equal(manifest.documentAccess, 'dynamic-page');
  assert.deepEqual(manifest.editorType, ['figma']);
});

test('network access is limited to the Moshionly app on this computer', () => {
  const access = manifest.networkAccess;
  assert.deepEqual(access.allowedDomains, ['http://localhost:47821', 'http://127.0.0.1:47821']);
  // Figma asks for a reason whenever a local server is listed.
  assert.ok(access.reasoning && access.reasoning.length > 20);
  assert.equal(access.devAllowedDomains, undefined);
});

test('the main code loads outside Figma without touching a figma global', () => {
  const plugin = require('../code.js');
  assert.equal(typeof plugin.summarizeSelection, 'function');
  const summary = plugin.summarizeSelection([
    { name: 'Card', type: 'FRAME' },
    { name: 'Slice', type: 'SLICE' },
    { name: 'Logo', type: 'VECTOR' },
    { name: '', type: 'TEXT' },
    { name: 'Badge', type: 'ELLIPSE' },
  ]);
  assert.deepEqual(summary, {
    count: 4,
    names: ['Card', 'Logo', 'text'],
    more: 1,
    empty: false,
    sections: 0,
  });
  assert.equal(plugin.summarizeSelection([]).empty, true);
});
