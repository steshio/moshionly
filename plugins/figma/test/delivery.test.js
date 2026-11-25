'use strict';

// The panel's delivery code, run outside a browser: sending to a stand-in
// for the Moshionly app on a free port of this computer, and copying. The
// whole panel also runs against a stand-in page, for its Copy button.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const vm = require('node:vm');

/** The delivery script from ui.html, run in a fresh context. */
function loadDelivery() {
  const html = fs.readFileSync(path.join(__dirname, '..', 'ui.html'), 'utf8');
  const match = html.match(/<script id="delivery">([\s\S]*?)<\/script>/);
  assert.ok(match, 'ui.html has a delivery script');
  const context = vm.createContext({ fetch, AbortController, TextEncoder, setTimeout, clearTimeout });
  vm.runInContext(match[1] + '\nthis.MoshionlyDelivery = MoshionlyDelivery;', context);
  return context.MoshionlyDelivery;
}

/**
 * A stand-in for the app's `/import`: it answers like the real one, 202
 * with `{ ok: true, status: "pending" }` for an envelope and 400 with an
 * error otherwise, and remembers what it received.
 */
async function standIn() {
  const received = [];
  const server = http.createServer((request, response) => {
    let body = '';
    request.on('data', (chunk) => {
      body += chunk;
    });
    request.on('end', () => {
      received.push({ method: request.method, url: request.url, type: request.headers['content-type'], body });
      let design = null;
      try {
        design = JSON.parse(body);
      } catch (error) {
        design = null;
      }
      const accepted = design && design.format === 'moshionly-import';
      response.writeHead(accepted ? 202 : 400, { 'Content-Type': 'application/json' });
      response.end(
        JSON.stringify(
          accepted
            ? { ok: true, status: 'pending', layers: design.nodes.length, notes: 0 }
            : { ok: false, error: 'This is not a Moshionly import: "format" should be "moshionly-import"' },
        ),
      );
    });
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = 'http://127.0.0.1:' + server.address().port + '/import';
  return { server, url, received };
}

/** An address where nothing listens: a port that was free a moment ago. */
async function deadUrl() {
  const server = http.createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = 'http://127.0.0.1:' + server.address().port + '/import';
  await new Promise((resolve) => server.close(resolve));
  return url;
}

const envelope = JSON.stringify({ format: 'moshionly-import', version: 1, source: { app: 'figma' }, nodes: [{ id: 'a', type: 'rect' }], notes: [] });

test('the panel sends to the Moshionly app on this computer only', () => {
  const delivery = loadDelivery();
  assert.deepEqual(Array.from(delivery.IMPORT_URLS), ['http://localhost:47821/import']);
  assert.equal(delivery.MAX_BODY_BYTES, 64 * 1024 * 1024);
});

test('a design the app takes is being added in Moshionly', async () => {
  const delivery = loadDelivery();
  const { server, url, received } = await standIn();
  try {
    const outcome = await delivery.sendDesign(envelope, { urls: [url] });
    assert.equal(outcome.ok, true);
    assert.equal(outcome.kind, 'sent');
    assert.match(outcome.message, /Moshionly is adding it/);
    assert.equal(received.length, 1);
    assert.equal(received[0].method, 'POST');
    assert.equal(received[0].type, 'application/json');
    assert.equal(received[0].body, envelope);
  } finally {
    server.close();
  }
});

test('a design the app refuses shows the app’s reason', async () => {
  const delivery = loadDelivery();
  const { server, url } = await standIn();
  try {
    const outcome = await delivery.sendDesign('{"format":"other"}', { urls: [url] });
    assert.equal(outcome.ok, false);
    assert.equal(outcome.kind, 'refused');
    assert.match(outcome.message, /"format" should be "moshionly-import"/);
  } finally {
    server.close();
  }
});

test('when nothing listens, the person hears that Moshionly is not running', async () => {
  const delivery = loadDelivery();
  const outcome = await delivery.sendDesign(envelope, { urls: [await deadUrl()] });
  assert.equal(outcome.kind, 'offline');
  assert.match(outcome.message, /Moshionly is not running/);
});

test('the second address is tried when the first does not answer', async () => {
  const delivery = loadDelivery();
  const { server, url, received } = await standIn();
  try {
    const outcome = await delivery.sendDesign(envelope, { urls: [await deadUrl(), url] });
    assert.equal(outcome.kind, 'sent');
    assert.equal(received.length, 1);
  } finally {
    server.close();
  }
});

test('a design over the app’s limit is not sent', async () => {
  const delivery = loadDelivery();
  let called = false;
  const outcome = await delivery.sendDesign(envelope, {
    size: 70 * 1024 * 1024,
    fetch: async () => {
      called = true;
    },
  });
  assert.equal(outcome.kind, 'too-large');
  assert.match(outcome.message, /70\.0 MB/);
  assert.equal(called, false);
});

test('copying goes through a copy event, then the clipboard API', async () => {
  const delivery = loadDelivery();
  const page = (works) => {
    const listeners = [];
    return {
      clipboard: null,
      addEventListener: (type, listener) => listeners.push(listener),
      removeEventListener: (type, listener) => listeners.splice(listeners.indexOf(listener), 1),
      execCommand(command) {
        if (!works || command !== 'copy') return false;
        const self = this;
        const event = { clipboardData: { setData: (type, text) => (self.clipboard = [type, text]) }, preventDefault() {} };
        listeners.forEach((listener) => listener(event));
        return true;
      },
    };
  };

  const working = page(true);
  assert.equal(await delivery.copyText('{"a":1}', { document: working, navigator: {} }), true);
  assert.deepEqual(working.clipboard, ['text/plain', '{"a":1}']);

  let written = null;
  const fallback = { clipboard: { writeText: async (text) => (written = text) } };
  assert.equal(await delivery.copyText('x', { document: page(false), navigator: fallback }), true);
  assert.equal(written, 'x');

  const refused = { clipboard: { writeText: async () => Promise.reject(new Error('NotAllowedError')) } };
  assert.equal(await delivery.copyText('x', { document: page(false), navigator: refused }), false);
});

test('sizes read well', () => {
  const delivery = loadDelivery();
  assert.equal(delivery.describeSize(200), '1 KB');
  assert.equal(delivery.describeSize(5 * 1024), '5 KB');
  assert.equal(delivery.describeSize(3.25 * 1024 * 1024), '3.3 MB');
  assert.equal(delivery.byteLength('é'), 2);
});

/**
 * Both of the panel's scripts from ui.html, run against a small stand-in
 * for the page, where copying to the clipboard is not allowed.
 */
function loadPanel() {
  const html = fs.readFileSync(path.join(__dirname, '..', 'ui.html'), 'utf8');
  const scripts = Array.from(html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g), (match) => match[1]);
  const element = (extra) =>
    Object.assign(
      {
        textContent: '',
        hidden: false,
        disabled: false,
        className: '',
        style: {},
        listeners: {},
        addEventListener(type, listener) {
          (this.listeners[type] = this.listeners[type] || []).push(listener);
        },
        fire(type) {
          return Promise.all((this.listeners[type] || []).map((listener) => listener({})));
        },
        append() {},
      },
      extra,
    );
  const elements = new Map();
  const document = {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, element());
      return elements.get(id);
    },
    querySelector: () => null,
    querySelectorAll: () => [],
    createElement: () => element(),
    createTextNode: (text) => ({ text }),
    addEventListener() {},
    removeEventListener() {},
    execCommand: () => false,
  };
  const posted = [];
  const window = {};
  const context = vm.createContext({
    document,
    navigator: {},
    window,
    parent: { postMessage: (data) => posted.push(JSON.parse(JSON.stringify(data.pluginMessage))) },
    TextEncoder,
    setTimeout,
    clearTimeout,
  });
  vm.runInContext(scripts.join('\n'), context);
  const view = (id) => document.getElementById(id);
  return {
    copy: view('copy'),
    notes: view('notes'),
    status: view('status'),
    posted,
    receive: (message) => window.onmessage({ data: { pluginMessage: message } }),
  };
}

test('a design waiting for “Copy now” is dropped when the selection changes', async () => {
  const panel = loadPanel();
  const selection = (name) => ({ type: 'selection', summary: { count: 1, names: [name], more: 0, empty: false, sections: 0 } });
  const prepare = async () => {
    await panel.copy.fire('click');
    assert.equal(panel.posted.at(-1).type, 'convert');
    await panel.receive({ type: 'converted', purpose: 'copy', json: '{"nodes":[]}', layers: 1, notes: [{ layer_name: 'A', message: 'A note.' }] });
    assert.equal(panel.copy.textContent, 'Copy now', 'the clipboard refused, so the design waits');
    assert.equal(panel.notes.hidden, false);
  };
  const assertDropped = (why) => {
    assert.equal(panel.copy.textContent, 'Copy', why);
    assert.equal(panel.notes.hidden, true, why);
    assert.equal(panel.status.textContent, '', why);
  };

  await panel.receive(selection('Frame A'));
  await prepare();
  await panel.receive(selection('Frame B'));
  assertDropped('after the selection changed');
  await panel.copy.fire('click');
  assert.deepEqual(panel.posted.at(-1), { type: 'convert', purpose: 'copy' }, 'Copy converts the new selection instead of copying the old design');
});

test('a design read while the selection changed is not offered for copying', async () => {
  const panel = loadPanel();
  const selection = (name) => ({ type: 'selection', summary: { count: 1, names: [name], more: 0, empty: false, sections: 0 } });
  const converted = { type: 'converted', purpose: 'copy', json: '{"nodes":[]}', layers: 1, notes: [{ layer_name: 'A', message: 'A note.' }] };

  await panel.receive(selection('Frame A'));
  await panel.copy.fire('click');
  await panel.receive(selection('Frame B'));
  await panel.receive(converted);
  assert.equal(panel.copy.textContent, 'Copy', 'Frame A’s design is not waiting next to Frame B');
  assert.equal(panel.notes.hidden, true);
  assert.match(panel.status.textContent, /changed while the design was being read/);

  await panel.copy.fire('click');
  await panel.receive(converted);
  assert.equal(panel.copy.textContent, 'Copy now', 'with nothing changed, the design waits as before');
});
