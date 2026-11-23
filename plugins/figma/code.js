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
  if (placement.skewed) {
    return convertOutline(node, container, m, run, fillOutline(node) || rectangleOutline(node));
  }
  const rect = newLayer('rect', node, run, container, placement);
  styleLayer(rect, node, run);
  const radius = cornerRadiusValue(cornerRadii(node), placement);
  if (radius !== undefined) rect.corner_radius = radius;
  noteCorners(node, run);
  run.emit(rect);
}

/**
 * The same transform followed by a mirror about the vertical line
 * x = `axis` of the node. For a shape that is symmetric about that line, the
 * result draws exactly the same thing without mirroring, so a mirrored
 * star can stay a star.
 */
function withoutMirror(m, axis) {
  if (decompose(m).sy >= 0) return { m, mirror: (x, y) => [x, y] };
  return {
    m: multiply(m, { a: -1, b: 0, c: 0, d: 1, e: 2 * axis, f: 0 }),
    mirror: (x, y) => [2 * axis - x, y],
  };
}

/** `place` for a shape symmetric about x = `axis`: never mirrored. */
function placeSymmetric(m, box, axis) {
  const unmirrored = withoutMirror(m, axis);
  const placement = place(unmirrored.m, box);
  const toLocal = placement.toLocal;
  placement.toLocal = (x, y) => toLocal(...unmirrored.mirror(x, y));
  return placement;
}

function convertEllipse(node, container, m, run) {
  const placement = place(m, ownBox(node));
  if (placement.skewed) {
    return convertOutline(node, container, m, run, fillOutline(node) || ellipseOutline(node));
  }
  const ellipse = newLayer('ellipse', node, run, container, placement);
  styleLayer(ellipse, node, run);
  // Figma and Moshionly both measure arcs clockwise from 3 o'clock; Figma
  // in radians, Moshionly in degrees.
  const arc = node.arcData || {};
  const start = (numberOr(arc.startingAngle, 0) * 180) / Math.PI;
  let sweep = ((numberOr(arc.endingAngle, 2 * Math.PI) - numberOr(arc.startingAngle, 0)) * 180) / Math.PI;
  sweep = clamp(sweep, -360, 360);
  const whole = Math.abs(sweep) >= 360 - 1e-6;
  if (!whole) {
    // A mirror runs the arc the other way round.
    const first = placement.mirrored ? -start : start;
    const startAngle = round(((first % 360) + 360) % 360, 3);
    if (startAngle !== 0 && startAngle !== 360) ellipse.start_angle = startAngle;
    ellipse.sweep = round(placement.mirrored ? -sweep : sweep, 3);
  }
  const inner = clamp(numberOr(arc.innerRadius, 0), 0, 1);
  if (inner > 0) ellipse.inner_radius = round(inner, 5);
  run.emit(ellipse);
}

/** The corners of a star with `points` points, first point up, outer radius 1. */
function starCorners(points, inner) {
  const corners = [];
  for (let corner = 0; corner < points * 2; corner += 1) {
    const turn = -Math.PI / 2 + (corner * Math.PI) / points;
    const reach = corner % 2 === 0 ? 1 : inner;
    corners.push([reach * Math.cos(turn), reach * Math.sin(turn)]);
  }
  return corners;
}

/**
 * The box of the Moshionly star that draws the same shape as a Figma star.
 *
 * Figma stretches a star so its corners touch all four sides of its box
 * (`bounds`, the box itself unless the star's geometry says otherwise).
 * Moshionly puts the points of a star on the ellipse inscribed in its box,
 * so its box is the box of that ellipse: wider than the star, and lower,
 * because a star's lower points do not reach as far down as its top point.
 */
function starBox(points, inner, bounds) {
  const corners = starCorners(points, inner);
  const xs = corners.map((corner) => corner[0]);
  const ys = corners.map((corner) => corner[1]);
  const spanX = Math.max(...xs) - Math.min(...xs);
  const spanY = Math.max(...ys) - Math.min(...ys);
  const radiusX = spanX > EPSILON ? bounds.width / spanX : 0;
  const radiusY = spanY > EPSILON ? bounds.height / spanY : 0;
  const centerX = bounds.x - Math.min(...xs) * radiusX;
  const centerY = bounds.y - Math.min(...ys) * radiusY;
  return { x: centerX - radiusX, y: centerY - radiusY, width: 2 * radiusX, height: 2 * radiusY };
}

function convertStar(node, container, m, run) {
  const points = Math.round(clamp(numberOr(node.pointCount, 5), 3, 64));
  const inner = clamp(numberOr(node.innerRadius, 0.382), 0, 1);
  const rounded = !isMixed(node.cornerRadius) && numberOr(node.cornerRadius, 0) > 0;
  if (rounded || place(m, ownBox(node)).skewed) {
    // Round points or a skew: only the outline itself is exact.
    return convertOutline(node, container, m, run, fillOutline(node) || starOutline(node, points, inner));
  }
  const corners = outlineBounds(fillOutline(node)) || ownBox(node);
  const box = starBox(points, inner, corners);
  const placement = placeSymmetric(m, box, box.x + box.width / 2);
  const star = newLayer('star', node, run, container, placement);
  styleLayer(star, node, run);
  if (points !== 5) star.points = points;
  if (Math.abs(inner - 0.4) > 1e-9) star.inner_radius = round(inner, 5);
  run.emit(star);
}

function convertPolygon(node, container, m, run) {
  const points = Math.round(clamp(numberOr(node.pointCount, 3), 3, 64));
  convertOutline(node, container, m, run, fillOutline(node) || polygonOutline(node, points));
}

function convertLine(node, container, m, run) {
  const outline = {
    subpaths: [{ anchors: [{ point: [0, 0] }, { point: [numberOr(node.width, 0), 0] }], closed: false }],
    fillRule: 'non_zero',
    open: 1,
  };
  convertOutline(node, container, m, run, outline);
}

function convertVector(node, container, m, run) {
  const paths = Array.from(node.vectorPaths || []);
  const rounded = !isMixed(node.cornerRadius) && numberOr(node.cornerRadius, 0) > 0;
  const closed = paths.length > 0 && paths.every((path) => path.windingRule !== 'NONE');
  let outline = readOutline(paths, node, run);
  if (rounded) {
    // The vector paths are the points before rounding; the fill geometry
    // has the rounding in it, but only closed shapes have one.
    const filled = closed ? fillOutline(node) : null;
    if (filled) outline = filled;
    else run.note(node, 'The rounding of this vector’s corners was left out; its points are sharp.');
  }
  convertOutline(node, container, m, run, outline);
}

function convertBoolean(node, container, m, run) {
  const outline = fillOutline(node);
  if (!outline) {
    // TODO: flatten a boolean result Figma gives no geometry for into an image.
    run.note(node, 'Figma gave no outline for this boolean shape, so it was left out.');
    return;
  }
  convertOutline(node, container, m, run, outline);
}

/**
 * A `path` layer for `node` from an outline in the node's coordinates
 * (from `readOutline` and friends). The path is normalised to the layer's
 * box, which is the node's box after any mirror or stretch, so a mirrored
 * or skewed shape is drawn exactly.
 */
function convertOutline(node, container, m, run, outline) {
  const placement = place(m, ownBox(node));
  const layer = newLayer('path', node, run, container, placement);
  styleLayer(layer, node, run);
  layer.path = normalisedPath(outline || { subpaths: [], fillRule: 'non_zero' }, placement);
  run.emit(layer);
  return layer;
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/**
 * Reads SVG path data into subpaths of anchors in the node's coordinates.
 *
 * Figma writes absolute `M`, `L`, `C`, `Q` and `Z` commands separated by
 * spaces; `H`, `V`, relative commands and commas are read as well.
 * Returns `null` for anything else (arcs, smooth shorthands, bad numbers).
 *
 * Each anchor is `{point, in, out}` with absolute handle positions, the way
 * Moshionly stores them. A segment's control points are the `out` handle of
 * the anchor it leaves and the `in` handle of the anchor it reaches.
 */
function parsePathData(data) {
  const tokens = String(data || '').match(/[a-zA-Z]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g) || [];
  const subpaths = [];
  let current = null;
  let x = 0;
  let y = 0;
  let startX = 0;
  let startY = 0;
  let command = null;
  let index = 0;
  const isCommand = (token) => /^[a-zA-Z]$/.test(token);
  const numbers = (count) => {
    const values = [];
    for (let n = 0; n < count; n += 1) {
      const token = tokens[index];
      if (token === undefined || isCommand(token)) return null;
      values.push(parseFloat(token));
      index += 1;
    }
    return values.every(Number.isFinite) ? values : null;
  };
  const begin = (px0, py0) => {
    current = { anchors: [{ point: [px0, py0] }], closed: false };
    subpaths.push(current);
    startX = px0;
    startY = py0;
  };
  const ensure = () => {
    if (!current) begin(x, y);
  };

  while (index < tokens.length) {
    if (isCommand(tokens[index])) {
      command = tokens[index];
      index += 1;
    } else if (!command) {
      return null;
    }
    const relative = command !== command.toUpperCase();
    const dx = relative ? x : 0;
    const dy = relative ? y : 0;
    switch (command.toUpperCase()) {
      case 'M': {
        const values = numbers(2);
        if (!values) return null;
        x = values[0] + dx;
        y = values[1] + dy;
        begin(x, y);
        // More pairs after a move are lines.
        command = relative ? 'l' : 'L';
        break;
      }
      case 'L':
      case 'H':
      case 'V': {
        const upper = command.toUpperCase();
        const values = numbers(upper === 'L' ? 2 : 1);
        if (!values) return null;
        ensure();
        if (upper === 'L') {
          x = values[0] + dx;
          y = values[1] + dy;
        } else if (upper === 'H') {
          x = values[0] + dx;
        } else {
          y = values[0] + dy;
        }
        current.anchors.push({ point: [x, y] });
        break;
      }
      case 'C': {
        const values = numbers(6);
        if (!values) return null;
        ensure();
        const previous = current.anchors[current.anchors.length - 1];
        previous.out = [values[0] + dx, values[1] + dy];
        x = values[4] + dx;
        y = values[5] + dy;
        current.anchors.push({ point: [x, y], in: [values[2] + dx, values[3] + dy] });
        break;
      }
      case 'Q': {
        const values = numbers(4);
        if (!values) return null;
        ensure();
        // A quadratic curve is the cubic with handles two thirds of the way
        // from each end to the quadratic's control point.
        const previous = current.anchors[current.anchors.length - 1];
        const [qx, qy] = [values[0] + dx, values[1] + dy];
        const [ex, ey] = [values[2] + dx, values[3] + dy];
        previous.out = [x + (2 / 3) * (qx - x), y + (2 / 3) * (qy - y)];
        current.anchors.push({ point: [ex, ey], in: [ex + (2 / 3) * (qx - ex), ey + (2 / 3) * (qy - ey)] });
        x = ex;
        y = ey;
        break;
      }
      case 'Z': {
        if (current) closeSubpath(current);
        current = null;
        x = startX;
        y = startY;
        break;
      }
      default:
        return null;
    }
  }
  return subpaths.filter((subpath) => subpath.anchors.length > 0);
}

/**
 * Marks a subpath closed. Figma usually draws the last segment back to the
 * first point before `Z`; that repeated point is folded into the first
 * anchor, keeping the handle that arrives at it.
 */
function closeSubpath(subpath) {
  const anchors = subpath.anchors;
  if (anchors.length > 1) {
    const first = anchors[0];
    const last = anchors[anchors.length - 1];
    if (Math.abs(first.point[0] - last.point[0]) < 1e-9 && Math.abs(first.point[1] - last.point[1]) < 1e-9) {
      if (last.in) first.in = last.in;
      else delete first.in;
      anchors.pop();
    }
  }
  subpath.closed = true;
}

/**
 * An outline from a list of Figma vector paths: its subpaths, the fill rule
 * and how many of the paths are open (have no inside).
 */
function readOutline(paths, node, run) {
  const outline = { subpaths: [], fillRule: 'non_zero', open: 0 };
  const rules = new Set();
  for (const path of paths) {
    const subpaths = parsePathData(path.data);
    if (!subpaths) {
      run.note(node, 'Part of this shape’s outline could not be read and was left out.');
      continue;
    }
    outline.subpaths.push(...subpaths);
    if (path.windingRule === 'NONE') outline.open += 1;
    else rules.add(path.windingRule);
  }
  if (rules.has('EVENODD')) outline.fillRule = 'even_odd';
  if (rules.size > 1) {
    run.note(node, 'This shape mixes fill rules; Moshionly uses one per layer, so all of it uses even-odd.');
  }
  return outline;
}

/** The outline Figma fills for `node` (`fillGeometry`), or `null`. */
function fillOutline(node) {
  const geometry = Array.from(node.fillGeometry || []);
  if (geometry.length === 0) return null;
  const outline = { subpaths: [], fillRule: 'non_zero', open: 0 };
  for (const path of geometry) {
    const subpaths = parsePathData(path.data);
    if (!subpaths) return null;
    outline.subpaths.push(...subpaths);
    if (path.windingRule === 'EVENODD') outline.fillRule = 'even_odd';
  }
  return outline.subpaths.length > 0 ? outline : null;
}

/** The box around every anchor of an outline, or `null`. */
function outlineBounds(outline) {
  if (!outline) return null;
  const points = [];
  for (const subpath of outline.subpaths) {
    for (const anchor of subpath.anchors) points.push(anchor.point);
  }
  if (points.length === 0) return null;
  const xs = points.map((point) => point[0]);
  const ys = points.map((point) => point[1]);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

/** A closed outline through `corners` given as fractions of the node's box. */
function cornerOutline(node, corners) {
  const width = numberOr(node.width, 0);
  const height = numberOr(node.height, 0);
  return {
    subpaths: [{ anchors: corners.map(([u, v]) => ({ point: [u * width, v * height] })), closed: true }],
    fillRule: 'non_zero',
    open: 0,
  };
}

/** A unit-radius shape stretched until it touches all sides of the box. */
function stretchedToBox(corners) {
  const xs = corners.map((corner) => corner[0]);
  const ys = corners.map((corner) => corner[1]);
  const [left, top] = [Math.min(...xs), Math.min(...ys)];
  const [spanX, spanY] = [Math.max(...xs) - left || 1, Math.max(...ys) - top || 1];
  return corners.map(([x, y]) => [(x - left) / spanX, (y - top) / spanY]);
}

function starOutline(node, points, inner) {
  return cornerOutline(node, stretchedToBox(starCorners(points, inner)));
}

function polygonOutline(node, points) {
  const corners = [];
  for (let corner = 0; corner < points; corner += 1) {
    const turn = -Math.PI / 2 + (corner * 2 * Math.PI) / points;
    corners.push([Math.cos(turn), Math.sin(turn)]);
  }
  return cornerOutline(node, stretchedToBox(corners));
}

/** A whole ellipse filling the node's box, as four cubic curves. */
function ellipseOutline(node) {
  const width = numberOr(node.width, 0);
  const height = numberOr(node.height, 0);
  const [rx, ry] = [width / 2, height / 2];
  // Handles at this fraction of the radius make a cubic curve stay within
  // 0.03% of a quarter circle.
  const k = 0.5522847498;
  const anchor = (x, y, inX, inY, outX, outY) => ({ point: [x, y], in: [inX, inY], out: [outX, outY] });
  return {
    subpaths: [
      {
        anchors: [
          anchor(width, ry, width, ry - k * ry, width, ry + k * ry),
          anchor(rx, height, rx + k * rx, height, rx - k * rx, height),
          anchor(0, ry, 0, ry + k * ry, 0, ry - k * ry),
          anchor(rx, 0, rx - k * rx, 0, rx + k * rx, 0),
        ],
        closed: true,
      },
    ],
    fillRule: 'non_zero',
    open: 0,
  };
}

function rectangleOutline(node) {
  return cornerOutline(node, [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ]);
}

/**
 * An outline as Moshionly's `path` value: every point and handle as a
 * fraction of the layer's box. A side of zero length (a horizontal line has
 * no height) maps every coordinate on that axis to 0.
 */
function normalisedPath(outline, placement) {
  const width = placement.width;
  const height = placement.height;
  const map = (point) => {
    const [x, y] = placement.toLocal(point[0], point[1]);
    return [width > EPSILON ? fraction(x / width) : 0, height > EPSILON ? fraction(y / height) : 0];
  };
  const same = (a, b) => a[0] === b[0] && a[1] === b[1];
  const subpaths = outline.subpaths.map((subpath) => {
    const anchors = subpath.anchors.map((anchor) => {
      const point = map(anchor.point);
      const result = { point };
      // A handle on its anchor is the same as no handle.
      if (anchor.in) {
        const handle = map(anchor.in);
        if (!same(handle, point)) result.in = handle;
      }
      if (anchor.out) {
        const handle = map(anchor.out);
        if (!same(handle, point)) result.out = handle;
      }
      return result;
    });
    return subpath.closed ? { anchors, closed: true } : { anchors };
  });
  const path = { subpaths };
  if (outline.fillRule === 'even_odd') path.fill_rule = 'even_odd';
  return path;
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
  ELLIPSE: convertEllipse,
  STAR: convertStar,
  POLYGON: convertPolygon,
  LINE: convertLine,
  VECTOR: convertVector,
  BOOLEAN_OPERATION: convertBoolean,
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
    parsePathData,
    starBox,
  };
}
