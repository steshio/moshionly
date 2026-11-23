#!/usr/bin/env node
'use strict';

// Converts the fixture designs into import envelopes, one JSON file each, in
// the folder given (a new temporary folder when none is). `acceptance/`
// then checks them with Moshionly's own import code.
//
//   node tools/convert-fixtures.js [folder]

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { exportSelection } = require('../code.js');
const { designs, loader } = require('../fixtures/designs.js');

async function main() {
  const folder = process.argv[2] || fs.mkdtempSync(path.join(os.tmpdir(), 'moshionly-figma-'));
  fs.mkdirSync(folder, { recursive: true });
  for (const make of designs) {
    const design = make();
    const { envelope, json } = await exportSelection(design.selection, {
      fileKey: 'FixtureFile',
      fileName: design.name,
      mode: 'new_artboards',
      loader,
    });
    const file = path.join(folder, design.slug + '.json');
    fs.writeFileSync(file, json);
    console.log(`${file}: ${envelope.nodes.length} layers, ${envelope.notes.length} notes`);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
