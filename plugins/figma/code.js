// Moshionly for Figma: the part of the plugin that runs in Figma's sandbox.
//
// The file has two halves. The first is the conversion from Figma nodes to
// Moshionly layers. It only reads properties of the objects it is given, so
// it works on real Figma nodes and on plain objects shaped like them, which
// is how the tests in `test/` exercise it with Node. The second half, at the
// end of the file, talks to Figma and to the panel (`ui.html`) and only runs
// inside Figma.
//
// There is no build step: Figma loads this file as it is.

'use strict';

// ---------------------------------------------------------------------------
// Selection summary
// ---------------------------------------------------------------------------

/** Node types that only organise the canvas and are opened up on export. */
const CANVAS_ONLY_TYPES = new Set(['SECTION']);

/**
 * What the panel shows about the selection: how many layers, a few names,
 * and whether there is anything to send.
 *
 * @param {ReadonlyArray<{name: string, type: string}>} selection
 * @returns {{count: number, names: string[], more: number, empty: boolean, sections: number}}
 */
function summarizeSelection(selection) {
  const items = Array.from(selection || []).filter((node) => node && node.type !== 'SLICE');
  const shown = items.slice(0, 3).map((node) => node.name || node.type.toLowerCase());
  return {
    count: items.length,
    names: shown,
    more: Math.max(0, items.length - shown.length),
    empty: items.length === 0,
    sections: items.filter((node) => CANVAS_ONLY_TYPES.has(node.type)).length,
  };
}

// ---------------------------------------------------------------------------
// Inside Figma
// ---------------------------------------------------------------------------

/** The panel's size in Figma, in pixels. */
const PANEL_SIZE = { width: 340, height: 560 };

function postSelection() {
  figma.ui.postMessage({
    type: 'selection',
    fileName: figma.root.name,
    summary: summarizeSelection(figma.currentPage.selection),
  });
}

function startPlugin() {
  figma.showUI(__html__, {
    width: PANEL_SIZE.width,
    height: PANEL_SIZE.height,
    themeColors: true,
    title: 'Moshionly',
  });
  figma.on('selectionchange', postSelection);
  figma.on('currentpagechange', postSelection);
  figma.ui.onmessage = (message) => {
    if (!message || typeof message.type !== 'string') return;
    switch (message.type) {
      case 'ready':
        postSelection();
        break;
      case 'notify':
        figma.notify(String(message.text || ''));
        break;
      case 'close':
        figma.closePlugin();
        break;
      default:
        break;
    }
  };
}

if (typeof figma !== 'undefined') {
  startPlugin();
}

// ---------------------------------------------------------------------------
// For the tests (Figma has no `module`)
// ---------------------------------------------------------------------------

if (typeof module !== 'undefined' && module.exports) {
  module.exports = {
    summarizeSelection,
  };
}
