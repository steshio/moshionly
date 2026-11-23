#!/usr/bin/env node
'use strict';

// Sends an envelope file to a running Moshionly the way the plugin's panel
// does: a CORS preflight from the origin "null" that asks for private
// network access, then the POST. Prints both answers. For checking the
// plugin and the app together by hand.
//
//   node tools/send.js <envelope.json> [url]
//
// The url defaults to http://127.0.0.1:47821/import.

const fs = require('node:fs');

async function main() {
  const [file, url = 'http://127.0.0.1:47821/import'] = process.argv.slice(2);
  if (!file) {
    console.error('usage: node tools/send.js <envelope.json> [url]');
    process.exit(2);
  }
  const body = fs.readFileSync(file, 'utf8');
  const preflight = await fetch(url, {
    method: 'OPTIONS',
    headers: {
      Origin: 'null',
      'Access-Control-Request-Method': 'POST',
      'Access-Control-Request-Headers': 'content-type',
      'Access-Control-Request-Private-Network': 'true',
    },
  });
  console.log('preflight', preflight.status);
  for (const name of [
    'access-control-allow-origin',
    'access-control-allow-methods',
    'access-control-allow-headers',
    'access-control-allow-private-network',
  ]) {
    console.log('  ' + name + ': ' + preflight.headers.get(name));
  }
  const response = await fetch(url, {
    method: 'POST',
    headers: { Origin: 'null', 'Content-Type': 'application/json' },
    body,
  });
  console.log('post', response.status, 'access-control-allow-origin:', response.headers.get('access-control-allow-origin'));
  console.log('  ' + (await response.text()));
  process.exit(response.ok ? 0 : 1);
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
