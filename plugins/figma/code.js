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
// Colours
// ---------------------------------------------------------------------------

function channel(value) {
  const byte = Math.round(clamp(numberOr(value, 0), 0, 1) * 255);
  return byte.toString(16).toUpperCase().padStart(2, '0');
}

/** `#RRGGBB`, or `#RRGGBBAA` when not opaque, from channels between 0 and 1. */
function colorHex(color, alpha) {
  const a = clamp(numberOr(alpha, 1), 0, 1);
  const hex = '#' + channel(color.r) + channel(color.g) + channel(color.b);
  return Math.round(a * 255) === 255 ? hex : hex + channel(a);
}

/**
 * One colour drawn over another (both {r, g, b, a} between 0 and 1), the
 * way Figma stacks two normal fills.
 */
function over(top, bottom) {
  const a = top.a + bottom.a * (1 - top.a);
  if (a < EPSILON) return { r: 0, g: 0, b: 0, a: 0 };
  const mix = (t, b) => (t * top.a + b * bottom.a * (1 - top.a)) / a;
  return { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a };
}

// ---------------------------------------------------------------------------
// Paints
// ---------------------------------------------------------------------------

/** Paint types Moshionly can hold as they are, or close to. */
const GRADIENT_TYPES = new Set(['GRADIENT_LINEAR', 'GRADIENT_RADIAL', 'GRADIENT_ANGULAR', 'GRADIENT_DIAMOND']);

/** Readable names for paint types in notes. */
const PAINT_NAMES = {
  SOLID: 'colour',
  GRADIENT_LINEAR: 'linear gradient',
  GRADIENT_RADIAL: 'radial gradient',
  GRADIENT_ANGULAR: 'angular gradient',
  GRADIENT_DIAMOND: 'diamond gradient',
  IMAGE: 'image',
  VIDEO: 'video',
  PATTERN: 'pattern',
  SHADER: 'shader',
};

function paintOpacity(paint) {
  return clamp(numberOr(paint.opacity, 1), 0, 1);
}

function isVisiblePaint(paint) {
  if (!paint || paint.visible === false || paintOpacity(paint) <= 0) return false;
  if (paint.type === 'SOLID') return true;
  if (GRADIENT_TYPES.has(paint.type)) {
    return Array.from(paint.gradientStops || []).some((stop) => numberOr(stop.color && stop.color.a, 1) > 0);
  }
  return true;
}

function blendsNormally(paint) {
  return !paint.blendMode || paint.blendMode === 'NORMAL' || paint.blendMode === 'PASS_THROUGH';
}

/** Whether nothing below `paint` can show through it. */
function isOpaquePaint(paint) {
  if (paintOpacity(paint) < 1 || !blendsNormally(paint)) return false;
  if (paint.type === 'SOLID') return true;
  if (GRADIENT_TYPES.has(paint.type)) {
    return Array.from(paint.gradientStops || []).every((stop) => numberOr(stop.color && stop.color.a, 1) >= 1);
  }
  return false;
}

/**
 * An affine matrix equal to `map`, a function of (x, y) that is affine:
 * it is read off where `map` sends the origin and the two unit vectors.
 */
function affineOf(map) {
  const [ox, oy] = map(0, 0);
  const [xx, xy] = map(1, 0);
  const [yx, yy] = map(0, 1);
  return { a: xx - ox, b: xy - oy, c: yx - ox, d: yy - oy, e: ox, f: oy };
}

/**
 * Converts a Figma gradient to a Moshionly one.
 *
 * Figma's `gradientTransform` maps the node's box, as fractions ((0, 0) at
 * the top-left corner, (1, 1) at the bottom-right), to "gradient space".
 * There a linear gradient runs along x, from (0, 0.5) at offset 0 to
 * (1, 0.5) at offset 1, and the colour only depends on x. A radial gradient
 * sits on the circle around (0.5, 0.5) with radius 0.5.
 *
 * Moshionly writes a gradient in fractions of the layer's box too, but that
 * box can differ from Figma's (a star's box is larger, a mirror flips it),
 * so everything goes through K, the affine map from gradient space to the
 * Moshionly layer's own pixels:
 *
 *   K = toLocal * scale(width, height) * inverse(gradientTransform)
 *
 * Linear: in the layer's pixels the offset at p is t(p) = n . p + t0,
 * where n is the first row of K's inverse. Moshionly's offset is the
 * projection on the line from `start` to `end`, so the two agree when
 * start = K(0, 0.5) (where t = 0) and end = start + n / |n|^2. This is
 * exact even when the box is stretched, where simply mapping (1, 0.5)
 * would tilt the colour bands.
 *
 * Radial: the circle maps to the ellipse K(0.5, 0.5) + J (0.5 cos a,
 * 0.5 sin a), J being K's linear part. Its semi-axes are the singular
 * values of J / 2 and the larger one points along the main eigenvector of
 * (J / 2)(J / 2)^T; that is Moshionly's radius and rotation, in pixels.
 * Offsets are distances on that ellipse in both programs, so the result is
 * exact.
 *
 * Finally positions are divided by the layer's width and height. On a side
 * of zero length (a line) every position on that axis is 0.
 */
function convertGradient(paint, node, placement) {
  const width = numberOr(node.width, 0);
  const height = numberOr(node.height, 0);
  const toNode = multiply(
    { a: width, b: 0, c: 0, d: height, e: 0, f: 0 },
    invert(matrixFromFigma(paint.gradientTransform)),
  );
  const K = affineOf((x, y) => placement.toLocal(...applyMatrix(toNode, x, y)));
  const boxWidth = placement.width;
  const boxHeight = placement.height;
  const unitPoint = ([x, y]) => [
    boxWidth > EPSILON ? fraction(x / boxWidth) : 0,
    boxHeight > EPSILON ? fraction(y / boxHeight) : 0,
  ];

  const opacity = paintOpacity(paint);
  const stops = Array.from(paint.gradientStops || [])
    .map((stop) => ({
      offset: clamp(numberOr(stop.position, 0), 0, 1),
      color: colorHex(stop.color || {}, numberOr(stop.color && stop.color.a, 1) * opacity),
    }))
    .sort((a, b) => a.offset - b.offset)
    .map((stop) => ({ offset: fraction(stop.offset), color: stop.color }));
  if (stops.length === 0) return undefined;
  if (stops.length === 1) return stops[0].color;

  if (paint.type === 'GRADIENT_LINEAR') {
    let start;
    let end;
    if (width > EPSILON && height > EPSILON) {
      start = applyMatrix(K, 0, 0.5);
      const inverse = invert(K);
      const normal = [inverse.a, inverse.c];
      const length = normal[0] * normal[0] + normal[1] * normal[1];
      end =
        length > EPSILON * EPSILON
          ? [start[0] + normal[0] / length, start[1] + normal[1] / length]
          : applyMatrix(K, 1, 0.5);
    } else {
      // A box without height (a line) or width: K cannot be inverted, and
      // only positions along the box matter. There the offset is
      // t = T.a u + T.c v + T.e for the fractions (u, v) of the box, with
      // the missing side's fraction at its middle.
      const T = matrixFromFigma(paint.gradientTransform);
      const alongWidth = width > EPSILON;
      const slope = alongWidth ? T.a : T.c;
      const base = (alongWidth ? T.c : T.a) * 0.5 + T.e;
      const from = Math.abs(slope) > EPSILON ? -base / slope : 0;
      const to = Math.abs(slope) > EPSILON ? from + 1 / slope : 1;
      const at = (s) => placement.toLocal(alongWidth ? s * width : width / 2, alongWidth ? height / 2 : s * height);
      start = at(from);
      end = at(to);
    }
    return { type: 'linear_gradient', start: unitPoint(start), end: unitPoint(end), stops };
  }

  // Radial, and the shapes Moshionly approximates with a radial gradient.
  const center = applyMatrix(K, 0.5, 0.5);
  const [a, b, c, d] = [K.a / 2, K.b / 2, K.c / 2, K.d / 2];
  const p = a * a + c * c;
  const q = a * b + c * d;
  const r = b * b + d * d;
  const mean = (p + r) / 2;
  const spread = Math.hypot((p - r) / 2, q);
  const major = Math.sqrt(Math.max(0, mean + spread));
  const minor = Math.sqrt(Math.max(0, mean - spread));
  const rotation = (0.5 * Math.atan2(2 * q, p - r) * 180) / Math.PI;
  const gradient = { type: 'radial_gradient', center: unitPoint(center) };
  gradient.radius = [
    boxWidth > EPSILON ? fraction(major / boxWidth) : 0,
    boxHeight > EPSILON ? fraction(minor / boxHeight) : 0,
  ];
  const turn = angle(rotation);
  if (turn !== 0) gradient.rotation = turn;
  gradient.stops = stops;
  return gradient;
}

/** A Moshionly paint for one Figma solid or gradient paint. */
function convertPaint(paint, node, run, placement, what, part) {
  if (!blendsNormally(paint)) {
    run.note(
      node,
      'The ' + what + ' blends as ' + String(paint.blendMode).toLowerCase().replace('_', ' ') +
        ' in Figma; Moshionly paints it normally.',
      part,
    );
  }
  if (paint.type === 'SOLID') return colorHex(paint.color || {}, paintOpacity(paint));
  if (paint.type === 'GRADIENT_ANGULAR' || paint.type === 'GRADIENT_DIAMOND') {
    run.note(
      node,
      'Moshionly has no ' + PAINT_NAMES[paint.type] + ', so the ' + what + ' became a radial gradient with the same colours.',
      part,
    );
  }
  return convertGradient(paint, node, placement);
}

/**
 * What a Figma paint list (fills or strokes) becomes. Moshionly layers have
 * one fill and one stroke.
 *
 * Paints that are hidden or covered by an opaque paint above them cannot be
 * seen and are dropped. Stacked plain colours are mixed into one, which is
 * exact. Otherwise the top paint is kept and the rest noted.
 *
 * @returns {{paint?: (string|object), image?: object, special?: string}}
 *   `paint` is ready for Moshionly; `image` is an image paint the caller
 *   handles; `special` names a paint type that cannot be a Moshionly paint.
 */
function readPaints(paints, node, run, placement, what, part) {
  if (isMixed(paints)) return {};
  const visible = Array.from(paints || []).filter(isVisiblePaint);
  let first = 0;
  for (let index = visible.length - 1; index >= 0; index -= 1) {
    if (isOpaquePaint(visible[index])) {
      first = index;
      break;
    }
  }
  const seen = visible.slice(first);
  if (seen.length === 0) return {};
  const top = seen[seen.length - 1];
  if (seen.every((paint) => paint.type === 'SOLID' && blendsNormally(paint))) {
    const mixed = seen.reduce(
      (below, paint) => over(Object.assign({}, paint.color, { a: paintOpacity(paint) }), below),
      { r: 0, g: 0, b: 0, a: 0 },
    );
    return { paint: colorHex(mixed, mixed.a) };
  }
  if (seen.length > 1) {
    run.note(
      node,
      'Has ' + seen.length + ' ' + what + 's; Moshionly layers have one, so only the top one (' +
        (PAINT_NAMES[top.type] || 'paint') + ') was kept.',
      part,
    );
  }
  if (top.type === 'SOLID' || GRADIENT_TYPES.has(top.type)) {
    return { paint: convertPaint(top, node, run, placement, what, part) };
  }
  if (top.type === 'IMAGE') return { image: top };
  return { special: top.type };
}

/**
 * The fill of `node` written onto `layer`. Image and other special fills
 * are the caller's business; here they are noted and left out.
 */
function fillLayer(layer, node, run, placement, part) {
  const fill = readPaints(node.fills, node, run, placement, 'fill', part);
  if (fill.paint !== undefined) layer.fill = fill.paint;
  const other = fill.image ? 'IMAGE' : fill.special;
  if (other) {
    run.note(node, 'Its ' + (PAINT_NAMES[other] || 'special') + ' fill has no Moshionly equivalent and was left out.', part);
  }
  return fill;
}

// ---------------------------------------------------------------------------
// Strokes
// ---------------------------------------------------------------------------

const STROKE_POSITIONS = { INSIDE: 'inside', OUTSIDE: 'outside', CENTER: 'center' };
const STROKE_CAPS = { NONE: 'butt', ROUND: 'round', SQUARE: 'square' };
const STROKE_JOINS = { MITER: 'miter', ROUND: 'round', BEVEL: 'bevel' };

/** The widths of the four sides of a frame or rectangle, when Figma has them. */
function sideWidths(node) {
  const sides = [node.strokeTopWeight, node.strokeRightWeight, node.strokeBottomWeight, node.strokeLeftWeight];
  return sides.every((side) => typeof side === 'number') ? sides : null;
}

/**
 * The stroke of `node` as Moshionly writes it, or `undefined` for none.
 * Moshionly strokes have one paint, one width, a position, a cap and a join.
 */
function readStroke(node, run, placement, part) {
  const result = readPaints(node.strokes, node, run, placement, 'stroke', part);
  const other = result.image ? 'IMAGE' : result.special;
  if (other) {
    run.note(node, 'Its ' + (PAINT_NAMES[other] || 'special') + ' stroke has no Moshionly equivalent and was left out.', part);
  }
  if (result.paint === undefined) return undefined;

  let width = node.strokeWeight;
  const sides = sideWidths(node);
  if (isMixed(width) || (sides && sides.some((side) => side !== sides[0]))) {
    width = sides ? Math.max(...sides) : 1;
    run.note(
      node,
      'Its sides have strokes of different widths; Moshionly draws the widest (' + px(width) + ' px) all round.',
      part,
    );
  }
  width = Math.max(0, numberOr(width, 1));
  if (placement.stretched) width *= Math.sqrt(Math.abs(placement.shape.sx * placement.shape.sy));
  if (width <= 0) return undefined;

  const stroke = { paint: result.paint, width: px(width) };
  const position = STROKE_POSITIONS[node.strokeAlign] || 'center';
  if (position !== 'center') stroke.position = position;

  const cap = node.strokeCap;
  if (isMixed(cap)) {
    run.note(node, 'Its line ends differ from point to point; Moshionly draws them all flat.', part);
  } else if (cap && !STROKE_CAPS[cap]) {
    run.note(
      node,
      'Its ' + String(cap).toLowerCase().replace(/_/g, ' ') + ' line ends are not in Moshionly; they are drawn flat.',
      part,
    );
  } else if (cap && STROKE_CAPS[cap] !== 'butt') {
    stroke.cap = STROKE_CAPS[cap];
  }

  const join = node.strokeJoin;
  if (isMixed(join)) {
    run.note(node, 'Its corners are joined differently from point to point; Moshionly uses sharp joins.', part);
  } else if (join && STROKE_JOINS[join] && STROKE_JOINS[join] !== 'miter') {
    stroke.join = STROKE_JOINS[join];
  }

  if (Array.from(node.dashPattern || []).some((dash) => dash > 0)) {
    run.note(node, 'Its dashed stroke is drawn as a solid line.', part);
  }
  const variable = node.variableWidthStrokeProperties;
  if (variable && variable.widthProfile && variable.widthProfile !== 'UNIFORM') {
    run.note(node, 'Its stroke of varying width became an even ' + stroke.width + ' px stroke.', part);
  }
  const complex = node.complexStrokeProperties;
  if (complex && complex.type && complex.type !== 'BASIC') {
    run.note(node, 'Its ' + String(complex.type).toLowerCase() + ' stroke became a plain stroke.', part);
  }
  return stroke;
}

function strokeLayer(layer, node, run, placement, part) {
  const stroke = readStroke(node, run, placement, part);
  if (stroke) layer.stroke = stroke;
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/** Readable names of effects Moshionly does not have. */
const EFFECT_NAMES = {
  INNER_SHADOW: 'inner shadow',
  BACKGROUND_BLUR: 'background blur',
  NOISE: 'noise',
  TEXTURE: 'texture',
  GLASS: 'glass',
  SHADER: 'shader',
};

function visibleEffects(node) {
  return Array.from(node.effects || []).filter((effect) => effect && effect.visible !== false);
}

/**
 * The drop shadow and layer blur of `node` written onto `layer`.
 *
 * Figma's shadow blur and layer blur radius are the values a designer
 * types, which are twice the standard deviation of the Gaussian, as in
 * CSS's box-shadow; Moshionly's `blur` means the same, so they carry over
 * unchanged. Both programs draw a shadow in the layer's own coordinates, so
 * it turns with the layer; a mirror flips its offset.
 */
function effectLayer(layer, node, run, placement, part) {
  const effects = visibleEffects(node);
  const shadows = effects.filter((effect) => effect.type === 'DROP_SHADOW');
  if (shadows.length > 0) {
    const strength = (effect) => numberOr(effect.color && effect.color.a, 1);
    const shadow = shadows.reduce((best, effect) => (strength(effect) > strength(best) ? effect : best));
    if (shadows.length > 1) {
      run.note(node, 'Has ' + shadows.length + ' drop shadows; Moshionly layers have one, so the strongest was kept.', part);
    }
    const offset = shadow.offset || { x: 0, y: 0 };
    const [ox, oy] = [numberOr(offset.x, 0), numberOr(offset.y, 0)];
    const { sx, sy, skew } = placement.shape;
    layer.shadow = {
      color: colorHex(shadow.color || {}, numberOr(shadow.color && shadow.color.a, 1)),
      x: px(sx * ox + skew * oy),
      y: px(sy * oy),
      blur: px(Math.max(0, numberOr(shadow.radius, 0))),
    };
    if (numberOr(shadow.spread, 0) !== 0) {
      run.note(node, 'Its shadow’s spread (' + px(shadow.spread) + ' px) was left out; Moshionly shadows have none.', part);
    }
    if (!blendsNormally(shadow)) {
      run.note(node, 'Its shadow’s blend mode was left out; the shadow blends normally.', part);
    }
  }

  const blurs = effects.filter((effect) => effect.type === 'LAYER_BLUR');
  if (blurs.length > 0) {
    const amount = (effect) =>
      effect.blurType === 'PROGRESSIVE'
        ? (numberOr(effect.radius, 0) + numberOr(effect.startRadius, 0)) / 2
        : numberOr(effect.radius, 0);
    const blur = Math.max(...blurs.map(amount));
    if (blur > 0) layer.blur = px(blur);
    if (blurs.some((effect) => effect.blurType === 'PROGRESSIVE')) {
      run.note(node, 'Its blur fades across the layer in Figma; Moshionly blurs it evenly by ' + px(blur) + ' px.', part);
    }
  }

  const missing = new Map();
  for (const effect of effects) {
    const name = EFFECT_NAMES[effect.type];
    if (name) missing.set(name, (missing.get(name) || 0) + 1);
  }
  for (const [name, count] of missing) {
    run.note(node, (count > 1 ? count + ' ' + name + 's were' : 'Its ' + name + ' was') + ' left out; Moshionly has no ' + name + '.', part);
  }
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
 * Opacity, visibility, lock, blend mode, shadow and blur of `node`, written
 * onto `layer` where they differ from Moshionly's defaults.
 * `layer.source_id` is set too.
 */
function styleLayer(layer, node, run, placement, part) {
  const opacity = round(clamp(numberOr(node.opacity, 1), 0, 1) * 100, 2);
  if (opacity !== 100) layer.opacity = opacity;
  if (node.visible === false) layer.hidden = true;
  if (node.locked === true) layer.locked = true;
  const mode = blendMode(node, run, part);
  if (mode !== 'normal') layer.blend_mode = mode;
  effectLayer(layer, node, run, placement, part);
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
  if (Array.from(node.strokes || []).some(isVisiblePaint)) reasons.push('stroke');
  if (visibleEffects(node).length > 0) reasons.push('effects');
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
  const placement = place(IDENTITY, ownBox(node));
  if (frameStyleReasons(node).length > 0) {
    const body = newLayer('group', node, run, inside, placement, 'frame');
    styleLayer(body, node, run, placement, 'frame');
    fillLayer(body, node, run, placement, 'frame');
    strokeLayer(body, node, run, placement, 'frame');
    styleFrame(body, node, run, placement, 'frame');
    if (body.shadow) {
      run.note(node, 'Its shadow reaches past the artboard, which cuts it off at the edge.', 'frame');
    }
    run.emit(body);
    inside = { id: body.id, frame: transform };
  } else {
    fillLayer(board, node, run, placement);
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
  styleLayer(group, node, run, placement);
  fillLayer(group, node, run, placement);
  strokeLayer(group, node, run, placement);
  styleFrame(group, node, run, placement);
  run.emit(group);
  await convertChildren(node, containerFor(group, container, placement), run);
}

/** A Figma group: a group whose box is the bounds of its children. */
async function convertGroup(node, container, m, run) {
  const placement = place(m, ownBox(node));
  const group = newLayer('group', node, run, container, placement);
  styleLayer(group, node, run, placement);
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
  styleLayer(rect, node, run, placement);
  fillLayer(rect, node, run, placement);
  strokeLayer(rect, node, run, placement);
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
  styleLayer(ellipse, node, run, placement);
  fillLayer(ellipse, node, run, placement);
  strokeLayer(ellipse, node, run, placement);
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
  styleLayer(star, node, run, placement);
  fillLayer(star, node, run, placement);
  strokeLayer(star, node, run, placement);
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
    filled: 0,
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
  styleLayer(layer, node, run, placement);
  strokeLayer(layer, node, run, placement);
  const shape = outline || { subpaths: [], fillRule: 'non_zero', open: 0, filled: 0 };
  // Figma does not fill paths that are all open; Moshionly would.
  const filled = shape.filled === undefined ? shape.subpaths.length > 0 : shape.filled > 0;
  if (filled) {
    const fill = fillLayer(layer, node, run, placement);
    if (shape.open > 0 && fill.paint !== undefined) {
      run.note(node, 'Open lines in this shape are filled in Moshionly; Figma leaves them unfilled.');
    }
  }
  layer.path = normalisedPath(shape, placement);
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
  const outline = { subpaths: [], fillRule: 'non_zero', open: 0, filled: 0 };
  const rules = new Set();
  for (const path of paths) {
    const subpaths = parsePathData(path.data);
    if (!subpaths) {
      run.note(node, 'Part of this shape’s outline could not be read and was left out.');
      continue;
    }
    outline.subpaths.push(...subpaths);
    if (path.windingRule === 'NONE') {
      outline.open += 1;
    } else {
      outline.filled += 1;
      rules.add(path.windingRule);
    }
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
