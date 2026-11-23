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
// Numbers
// ---------------------------------------------------------------------------

/** Values closer than this count as equal. */
const EPSILON = 1e-6;

/** Rounds to `digits` decimals, and turns -0 into 0. */
function round(value, digits) {
  const factor = Math.pow(10, digits);
  const result = Math.round(value * factor) / factor;
  return Object.is(result, -0) ? 0 : result;
}

/** A length or position in pixels, rounded for a tidy file. */
function px(value) {
  return round(value, 3);
}

/** A fraction of a box (path and gradient coordinates). */
function fraction(value) {
  return round(value, 5);
}

/** An angle in degrees, between -180 (excluded) and 180. */
function angle(degrees) {
  let result = degrees % 360;
  if (result <= -180) result += 360;
  if (result > 180) result -= 360;
  result = round(result, 3);
  return result === -180 ? 180 : result;
}

function clamp(value, low, high) {
  return Math.min(high, Math.max(low, value));
}

/** A number, or `fallback` when the value is missing or not finite. */
function numberOr(value, fallback) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

/**
 * Whether a Figma property has several values at once (`figma.mixed`).
 * Test fixtures, which are JSON, write the string "MIXED" instead.
 */
function isMixed(value) {
  return typeof value === 'symbol' || value === 'MIXED';
}

// ---------------------------------------------------------------------------
// Matrices
// ---------------------------------------------------------------------------
//
// A 2D affine transform is {a, b, c, d, e, f}, mapping (x, y) to
// (a x + c y + e, b x + d y + f). Figma writes the same matrix as
// [[a, c, e], [b, d, f]]. The y axis points down, so a matrix with
// a = cos t, b = sin t, c = -sin t, d = cos t turns things clockwise by t,
// which is the direction Moshionly's `rotation` counts in.

const IDENTITY = Object.freeze({ a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 });

/** A matrix from Figma's [[a, c, e], [b, d, f]] form. */
function matrixFromFigma(transform) {
  if (!Array.isArray(transform) || transform.length !== 2) return IDENTITY;
  const [[a, c, e], [b, d, f]] = transform;
  return { a, b, c, d, e, f };
}

/** The matrix that applies `n` first and then `m`. */
function multiply(m, n) {
  return {
    a: m.a * n.a + m.c * n.b,
    b: m.b * n.a + m.d * n.b,
    c: m.a * n.c + m.c * n.d,
    d: m.b * n.c + m.d * n.d,
    e: m.a * n.e + m.c * n.f + m.e,
    f: m.b * n.e + m.d * n.f + m.f,
  };
}

/** The inverse of `m`, or the identity when `m` flattens the plane. */
function invert(m) {
  const det = m.a * m.d - m.b * m.c;
  if (Math.abs(det) < EPSILON * EPSILON) return IDENTITY;
  return {
    a: m.d / det,
    b: -m.b / det,
    c: -m.c / det,
    d: m.a / det,
    e: (m.c * m.f - m.d * m.e) / det,
    f: (m.b * m.e - m.a * m.f) / det,
  };
}

function applyMatrix(m, x, y) {
  return [m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f];
}

function translation(x, y) {
  return { a: 1, b: 0, c: 0, d: 1, e: x, f: y };
}

function rotationMatrix(radians) {
  const cos = Math.cos(radians);
  const sin = Math.sin(radians);
  return { a: cos, b: sin, c: -sin, d: cos, e: 0, f: 0 };
}

/**
 * Splits the linear part of `m` into a rotation followed by an upper
 * triangular "shape" matrix: m = rotate(radians) * [[sx, skew], [0, sy]].
 *
 * For an ordinary Figma layer sx = 1, sy = 1 and skew = 0. A mirrored layer
 * has sy = -1 (a horizontal mirror shows up as a vertical one turned by
 * 180 degrees, which is the same thing). Anything else is a skew or a
 * stretch that Moshionly cannot express with a box and a rotation.
 */
function decompose(m) {
  const sx = Math.hypot(m.a, m.b);
  if (sx < EPSILON) {
    return { radians: 0, sx: 0, sy: m.d, skew: m.c };
  }
  const det = m.a * m.d - m.b * m.c;
  return {
    radians: Math.atan2(m.b, m.a),
    sx,
    sy: det / sx,
    skew: (m.a * m.c + m.b * m.d) / sx,
  };
}

// ---------------------------------------------------------------------------
// Placement: from a Figma transform to Moshionly's box and rotation
// ---------------------------------------------------------------------------

/**
 * Where a box of a node lands in its Moshionly parent.
 *
 * `m` maps the node's own coordinates (Figma's, origin at the top-left of
 * its box) to the coordinates of the Moshionly parent. `box` is the part of
 * the node's coordinates that the Moshionly layer covers, usually
 * {x: 0, y: 0, width, height}.
 *
 * With m = rotate(r) * S + t (see `decompose`), a point p of the node lands
 * at rotate(r) * (S p) + t. Moshionly turns a layer about the centre of its
 * box, so the layer's box is B, the bounds of S applied to `box`, turned by
 * r about its centre c = t + rotate(r) * centre(B). Its top-left corner is
 * c minus half its size. A mirror or a stretch stays in S; the shape
 * converters bake it into the geometry they write, through `toLocal`.
 *
 * @returns {{x: number, y: number, width: number, height: number,
 *   rotation: number, mirrored: boolean, skewed: boolean, stretched: boolean,
 *   shape: {sx: number, sy: number, skew: number},
 *   frame: object, toLocal: function(number, number): number[]}}
 */
function place(m, box) {
  const parts = decompose(m);
  const shape = { sx: parts.sx, sy: parts.sy, skew: parts.skew };
  const corners = [
    [box.x, box.y],
    [box.x + box.width, box.y],
    [box.x, box.y + box.height],
    [box.x + box.width, box.y + box.height],
  ].map(([x, y]) => [shape.sx * x + shape.skew * y, shape.sy * y]);
  const xs = corners.map((point) => point[0]);
  const ys = corners.map((point) => point[1]);
  const left = Math.min(...xs);
  const top = Math.min(...ys);
  const width = Math.max(...xs) - left;
  const height = Math.max(...ys) - top;

  const turn = rotationMatrix(parts.radians);
  const [cx, cy] = applyMatrix(turn, left + width / 2, top + height / 2);
  const center = [cx + m.e, cy + m.f];
  const frame = multiply(
    translation(center[0], center[1]),
    multiply(turn, translation(-width / 2, -height / 2)),
  );
  const scaleTolerance = 1e-3;
  return {
    x: center[0] - width / 2,
    y: center[1] - height / 2,
    width,
    height,
    rotation: (parts.radians * 180) / Math.PI,
    mirrored: shape.sy < 0,
    skewed: Math.abs(shape.skew) > scaleTolerance * Math.max(1, shape.sx),
    stretched:
      Math.abs(shape.sx - 1) > scaleTolerance || Math.abs(Math.abs(shape.sy) - 1) > scaleTolerance,
    shape,
    // The layer's own coordinates (origin at its top-left corner) to the
    // parent's.
    frame,
    // A point in the node's coordinates to the layer's own coordinates.
    toLocal(x, y) {
      return [shape.sx * x + shape.skew * y - left, shape.sy * y - top];
    },
  };
}

/** The node's box in its own coordinates. */
function ownBox(node) {
  return { x: 0, y: 0, width: numberOr(node.width, 0), height: numberOr(node.height, 0) };
}

/** The node's transform to the page. */
function absoluteOf(node) {
  return matrixFromFigma(node.absoluteTransform);
}

// ---------------------------------------------------------------------------
// The conversion run
// ---------------------------------------------------------------------------

/** Figma node types that become a group with a box of its own. */
const FRAME_TYPES = new Set(['FRAME', 'COMPONENT', 'COMPONENT_SET', 'INSTANCE', 'SLOT']);

/** Layers are counted for progress in steps of this many. */
const PROGRESS_STEP = 100;

/**
 * Everything one conversion collects: the layers, the notes about what was
 * approximated, and the images still to fetch from Figma.
 */
class Conversion {
  /**
   * @param {{fileKey: string, onProgress?: function(object): void,
   *   pause?: function(): Promise<void>}} options
   */
  constructor(options) {
    this.fileKey = String(options.fileKey || 'unknown');
    this.onProgress = options.onProgress || null;
    this.pause = options.pause || null;
    this.nodes = [];
    this.notes = [];
    this.assets = [];
    this.noted = new Set();
    this.usedIds = new Set();
    this.done = 0;
    this.total = 0;
  }

  /**
   * The `source_id` of a layer made from `node`: `figma:<file key>:<node id>`,
   * which re-imports match on. A layer that the plugin adds for a part of a
   * node (the background image of a frame, say) appends `/<part>`.
   */
  sourceId(node, part) {
    return 'figma:' + this.fileKey + ':' + node.id + (part ? '/' + part : '');
  }

  /** The id of a layer in the design: Figma's node id, unique in the list. */
  designId(node, part) {
    const base = String(node.id) + (part ? '/' + part : '');
    let id = base;
    for (let copy = 2; this.usedIds.has(id); copy += 1) id = base + '#' + copy;
    this.usedIds.add(id);
    return id;
  }

  emit(layer) {
    this.nodes.push(layer);
    return layer;
  }

  /** Records that something about `node` could not be brought over exactly. */
  note(node, message, part) {
    const sourceId = this.sourceId(node, part);
    const key = sourceId + '\n' + message;
    if (this.noted.has(key)) return;
    this.noted.add(key);
    this.notes.push({ source_id: sourceId, layer_name: node.name || '', message });
  }

  /** Counts one more node, and lets Figma breathe now and then. */
  async step() {
    this.done += 1;
    if (this.done % PROGRESS_STEP !== 0) return;
    if (this.onProgress) {
      this.onProgress({ phase: 'layers', done: this.done, total: this.total });
    }
    if (this.pause) await this.pause();
  }
}

/** How many nodes `convertSelection` will visit for `node`. */
function countNodes(node) {
  let count = 1;
  for (const child of node.children || []) count += countNodes(child);
  return count;
}

/** Whether `node` sits directly on the canvas, not inside another layer. */
function isOnCanvas(node) {
  const parent = node.parent;
  return !parent || parent.type === 'PAGE' || parent.type === 'SECTION';
}

/**
 * The nodes to convert, in order: the selection without nodes that are
 * inside another selected node (they come along with it), with sections
 * opened up into what they hold, and without slices.
 */
function selectionRoots(selection) {
  const list = Array.from(selection || []).filter(Boolean);
  const chosen = new Set(list.map((node) => node.id));
  const insideChosen = (node) => {
    for (let parent = node.parent; parent; parent = parent.parent) {
      if (chosen.has(parent.id)) return true;
    }
    return false;
  };
  const roots = [];
  const add = (node) => {
    if (node.type === 'SLICE') return;
    if (CANVAS_ONLY_TYPES.has(node.type)) {
      for (const child of node.children || []) add(child);
      return;
    }
    roots.push(node);
  };
  for (const node of list) {
    if (!insideChosen(node)) add(node);
  }
  return roots;
}

/** The page as a parent: top-level layers keep their canvas position. */
const ON_PAGE = Object.freeze({ id: undefined, frame: IDENTITY });

/** The transform from `node` to the Moshionly layer that will hold it. */
function relativeMatrix(container, node) {
  return multiply(invert(container.frame), absoluteOf(node));
}

/** A parent for the children of `layer`, which was placed by `placement`. */
function containerFor(layer, container, placement) {
  return { id: layer.id, frame: multiply(container.frame, placement.frame) };
}

// ---------------------------------------------------------------------------
// Layer properties every layer has
// ---------------------------------------------------------------------------

/** Figma blend modes and the Moshionly ones, with a word when they differ. */
const BLEND_MODES = {
  PASS_THROUGH: ['normal'],
  NORMAL: ['normal'],
  DARKEN: ['darken'],
  MULTIPLY: ['multiply'],
  LINEAR_BURN: ['multiply', 'linear burn'],
  COLOR_BURN: ['color_burn'],
  LIGHTEN: ['lighten'],
  SCREEN: ['screen'],
  LINEAR_DODGE: ['screen', 'linear dodge (add)'],
  COLOR_DODGE: ['color_dodge'],
  OVERLAY: ['overlay'],
  SOFT_LIGHT: ['soft_light'],
  HARD_LIGHT: ['hard_light'],
  DIFFERENCE: ['difference'],
  EXCLUSION: ['exclusion'],
  HUE: ['hue'],
  SATURATION: ['saturation'],
  COLOR: ['color'],
  LUMINOSITY: ['luminosity'],
};

/** The Moshionly blend mode for a Figma one, noting a substitute. */
function blendMode(node, run, part) {
  const mode = node.blendMode;
  if (!mode) return 'normal';
  const known = BLEND_MODES[mode];
  if (!known) {
    run.note(node, 'Blend mode ' + mode.toLowerCase() + ' is not in Moshionly; it blends normally.', part);
    return 'normal';
  }
  if (known[1]) {
    run.note(
      node,
      'Blend mode ' + known[1] + ' became ' + known[0].replace('_', ' ') + ', the closest Moshionly has.',
      part,
    );
  }
  return known[0];
}

/**
 * A new layer for `node`: id, parent, type, name and box. The caller adds
 * the style with `styleLayer` and the type's own fields.
 */
function newLayer(type, node, run, container, placement, part) {
  const layer = { id: run.designId(node, part) };
  if (container.id !== undefined) layer.parent = container.id;
  layer.type = type;
  layer.name = node.name || '';
  layer.x = px(placement.x);
  layer.y = px(placement.y);
  layer.width = px(placement.width);
  layer.height = px(placement.height);
  const rotation = angle(placement.rotation);
  if (rotation !== 0) layer.rotation = rotation;
  return layer;
}

/**
 * Opacity, visibility, lock and blend mode of `node`, written onto `layer`
 * where they differ from Moshionly's defaults. `layer.source_id` is set too.
 */
function styleLayer(layer, node, run, part) {
  const opacity = round(clamp(numberOr(node.opacity, 1), 0, 1) * 100, 2);
  if (opacity !== 100) layer.opacity = opacity;
  if (node.visible === false) layer.hidden = true;
  if (node.locked === true) layer.locked = true;
  const mode = blendMode(node, run, part);
  if (mode !== 'normal') layer.blend_mode = mode;
  layer.source_id = run.sourceId(node, part);
  return layer;
}

// ---------------------------------------------------------------------------
// Corners
// ---------------------------------------------------------------------------

/** The four corner radii of a rectangle or frame: top-left, top-right, bottom-right, bottom-left. */
function cornerRadii(node) {
  const each = [node.topLeftRadius, node.topRightRadius, node.bottomRightRadius, node.bottomLeftRadius];
  if (each.every((radius) => typeof radius === 'number')) return each.map((radius) => Math.max(0, radius));
  const radius = isMixed(node.cornerRadius) ? 0 : Math.max(0, numberOr(node.cornerRadius, 0));
  return [radius, radius, radius, radius];
}

/**
 * Corner radii as Moshionly writes them (one number, or four), after the
 * mirror and stretch of `placement`. `undefined` when no corner is round.
 */
function cornerRadiusValue(radii, placement) {
  let result = radii.slice();
  if (placement.mirrored) {
    // Mirrored top to bottom: the top corners swap with the bottom ones.
    result = [result[3], result[2], result[1], result[0]];
  }
  const stretch = Math.sqrt(Math.abs(placement.shape.sx * placement.shape.sy));
  result = result.map((radius) => px(radius * (placement.stretched ? stretch : 1)));
  if (result.every((radius) => radius === 0)) return undefined;
  return result.every((radius) => radius === result[0]) ? result[0] : result;
}

/** Notes rounding that Moshionly draws differently. */
function noteCorners(node, run, part) {
  if (numberOr(node.cornerSmoothing, 0) > 0 && cornerRadii(node).some((radius) => radius > 0)) {
    run.note(
      node,
      'Smoothed corners (' + Math.round(node.cornerSmoothing * 100) + '% smoothing) became plain round corners.',
      part,
    );
  }
}

// ---------------------------------------------------------------------------
// Containers: artboards and groups
// ---------------------------------------------------------------------------

/**
 * What of a top-level frame an artboard cannot hold. Artboards are plain
 * rectangles that do not turn, fade or blend; when the frame needs any of
 * that, its look goes on a group just inside the artboard.
 */
function frameStyleReasons(node) {
  const reasons = [];
  if (cornerRadii(node).some((radius) => radius > 0)) reasons.push('round corners');
  if (numberOr(node.opacity, 1) < 1) reasons.push('opacity');
  if (node.blendMode && node.blendMode !== 'NORMAL' && node.blendMode !== 'PASS_THROUGH') {
    reasons.push('blend mode');
  }
  return reasons;
}

/**
 * A frame on the canvas becomes an artboard at the same place on the
 * canvas. Its children keep their positions inside it.
 */
async function convertArtboard(node, run) {
  await run.step();
  const transform = absoluteOf(node);
  const parts = decompose(transform);
  if (Math.abs(angle((parts.radians * 180) / Math.PI)) > 0.001 || parts.sy < 0) {
    run.note(node, 'Artboards cannot be turned or mirrored, so this frame arrives upright.');
  }
  const board = { id: run.designId(node), type: 'artboard', name: node.name || '' };
  board.x = px(transform.e);
  board.y = px(transform.f);
  board.width = px(Math.max(1, numberOr(node.width, 1)));
  board.height = px(Math.max(1, numberOr(node.height, 1)));
  if (node.visible === false) board.hidden = true;
  if (node.locked === true) board.locked = true;
  board.source_id = run.sourceId(node);
  run.emit(board);

  // The frame's children are laid out in the frame's own coordinates.
  let inside = { id: board.id, frame: transform };
  if (frameStyleReasons(node).length > 0) {
    const placement = place(IDENTITY, ownBox(node));
    const body = newLayer('group', node, run, inside, placement, 'frame');
    styleLayer(body, node, run, 'frame');
    styleFrame(body, node, run, placement, 'frame');
    run.emit(body);
    inside = { id: body.id, frame: transform };
  }
  await convertChildren(node, inside, run);
}

/** The look of a frame on the group that stands for it. */
function styleFrame(group, node, run, placement, part) {
  const radius = cornerRadiusValue(cornerRadii(node), placement);
  if (radius !== undefined) group.corner_radius = radius;
  noteCorners(node, run, part);
  if (node.clipsContent) group.clip = true;
}

/** A frame, component or instance inside something else: a group. */
async function convertFrame(node, container, m, run) {
  const placement = place(m, ownBox(node));
  const group = newLayer('group', node, run, container, placement);
  styleLayer(group, node, run);
  styleFrame(group, node, run, placement);
  run.emit(group);
  await convertChildren(node, containerFor(group, container, placement), run);
}

/** A Figma group: a group whose box is the bounds of its children. */
async function convertGroup(node, container, m, run) {
  const placement = place(m, ownBox(node));
  const group = newLayer('group', node, run, container, placement);
  styleLayer(group, node, run);
  run.emit(group);
  await convertChildren(node, containerFor(group, container, placement), run);
}

async function convertChildren(node, container, run) {
  for (const child of node.children || []) {
    await convertLayer(child, container, run);
  }
}

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

function convertRectangle(node, container, m, run) {
  const placement = place(m, ownBox(node));
  const rect = newLayer('rect', node, run, container, placement);
  styleLayer(rect, node, run);
  const radius = cornerRadiusValue(cornerRadii(node), placement);
  if (radius !== undefined) rect.corner_radius = radius;
  noteCorners(node, run);
  run.emit(rect);
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/** How each Figma node type is converted. */
const CONVERTERS = {
  FRAME: convertFrame,
  COMPONENT: convertFrame,
  COMPONENT_SET: convertFrame,
  INSTANCE: convertFrame,
  SLOT: convertFrame,
  SECTION: convertFrame,
  GROUP: convertGroup,
  RECTANGLE: convertRectangle,
};

async function convertLayer(node, container, run) {
  if (node.type === 'SLICE') return;
  const converter = CONVERTERS[node.type];
  if (!converter) {
    await run.step();
    // TODO: flatten what has no Moshionly layer into an image (next step).
    run.note(node, 'Moshionly has no layer like this ' + node.type.toLowerCase() + ', so it was left out.');
    return;
  }
  await run.step();
  await converter(node, container, relativeMatrix(container, node), run);
}

/**
 * Converts a Figma selection into Moshionly layers.
 *
 * The nodes may be real Figma nodes or plain objects with the same
 * properties. Frames that sit directly on the canvas become artboards;
 * everything else keeps its place relative to what holds it.
 *
 * @param {ReadonlyArray<object>} selection
 * @param {{fileKey: string, onProgress?: function(object): void,
 *   pause?: function(): Promise<void>}} options
 * @returns {Promise<{nodes: object[], notes: object[], assets: object[]}>}
 */
async function convertSelection(selection, options) {
  const run = new Conversion(options || {});
  const roots = selectionRoots(selection);
  run.total = roots.reduce((sum, node) => sum + countNodes(node), 0);
  for (const node of roots) {
    if (FRAME_TYPES.has(node.type) && isOnCanvas(node)) {
      await convertArtboard(node, run);
    } else {
      await convertLayer(node, ON_PAGE, run);
    }
  }
  if (run.onProgress) run.onProgress({ phase: 'layers', done: run.total, total: run.total });
  return { nodes: run.nodes, notes: run.notes, assets: run.assets };
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
    convertSelection,
    selectionRoots,
    decompose,
    place,
    multiply,
    invert,
    matrixFromFigma,
    angle,
  };
}
