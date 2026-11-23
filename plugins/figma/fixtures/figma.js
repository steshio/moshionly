'use strict';

// Plain objects shaped like Figma nodes, for the tests.
//
// A node is written with its position relative to its parent's own
// coordinates (`x`, `y`, `rotation` in degrees clockwise, `mirror`), and
// `page()` works out each node's `absoluteTransform` the way Figma reports
// it: the transform from the node's own coordinates (origin at the top-left
// corner of its box) to the page. Parents are linked through a
// non-enumerable `parent`, as in Figma, so the trees still print as JSON.

/** Figma's [[a, c, e], [b, d, f]] for a matrix {a, b, c, d, e, f}. */
function toFigma(m) {
  return [
    [m.a, m.c, m.e],
    [m.b, m.d, m.f],
  ];
}

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

/** The transform of a node inside its parent, from its friendly fields. */
function local(spec) {
  const radians = ((spec.rotation || 0) * Math.PI) / 180;
  const cos = Math.cos(radians);
  const sin = Math.sin(radians);
  let m = { a: cos, b: sin, c: -sin, d: cos, e: spec.x || 0, f: spec.y || 0 };
  // A mirror keeps the box where it is and flips what is inside.
  if (spec.mirror === 'x') m = multiply(m, { a: -1, b: 0, c: 0, d: 1, e: spec.width || 0, f: 0 });
  if (spec.mirror === 'y') m = multiply(m, { a: 1, b: 0, c: 0, d: -1, e: 0, f: spec.height || 0 });
  if (spec.skew) m = multiply(m, { a: 1, b: 0, c: spec.skew, d: 1, e: 0, f: 0 });
  return m;
}

const FRIENDLY = ['x', 'y', 'rotation', 'mirror', 'skew'];

/** Defaults every Figma scene node has. */
const SCENE_DEFAULTS = {
  visible: true,
  locked: false,
  opacity: 1,
  blendMode: 'PASS_THROUGH',
  isMask: false,
  effects: [],
};

/**
 * A node of `type`. `spec` holds Figma properties plus the friendly
 * position fields; `children` are nodes made with this function.
 */
function node(type, spec, children) {
  const result = Object.assign({ type, name: spec && spec.name ? spec.name : type }, SCENE_DEFAULTS, spec);
  Object.defineProperty(result, 'placement', { value: local(result), enumerable: false });
  for (const key of FRIENDLY) delete result[key];
  if (children) result.children = children;
  return result;
}

/** Gives every node an id, its parent and its absolute transform. */
function link(item, parent, parentTransform, counter) {
  if (item.id === undefined) {
    counter.next += 1;
    item.id = '1:' + counter.next;
  }
  Object.defineProperty(item, 'parent', { value: parent, enumerable: false, configurable: true });
  const transform = item.placement ? multiply(parentTransform, item.placement) : parentTransform;
  if (item.type !== 'PAGE') item.absoluteTransform = toFigma(transform);
  for (const child of item.children || []) link(child, item, transform, counter);
  return item;
}

/** A page holding `children`, with every node linked and placed. */
function page(...children) {
  const root = { id: '0:1', type: 'PAGE', name: 'Page 1', children };
  return link(root, null, { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }, { next: 0 });
}

const solid = (r, g, b, opacity) => ({
  type: 'SOLID',
  visible: true,
  opacity: opacity === undefined ? 1 : opacity,
  blendMode: 'NORMAL',
  color: { r, g, b },
});

module.exports = {
  page,
  node,
  frame: (spec, children) => node('FRAME', Object.assign({ clipsContent: true, fills: [] }, spec), children || []),
  group: (spec, children) => node('GROUP', spec, children || []),
  instance: (spec, children) => node('INSTANCE', Object.assign({ clipsContent: false, fills: [] }, spec), children || []),
  section: (spec, children) => node('SECTION', Object.assign({ fills: [] }, spec), children || []),
  rect: (spec) => node('RECTANGLE', Object.assign({ fills: [], strokes: [], cornerRadius: 0 }, spec)),
  ellipse: (spec) => node('ELLIPSE', Object.assign({ fills: [], strokes: [] }, spec)),
  star: (spec) => node('STAR', Object.assign({ fills: [], strokes: [], pointCount: 5, innerRadius: 0.382, cornerRadius: 0 }, spec)),
  polygon: (spec) => node('POLYGON', Object.assign({ fills: [], strokes: [], pointCount: 3, cornerRadius: 0 }, spec)),
  vector: (spec) => node('VECTOR', Object.assign({ fills: [], strokes: [], cornerRadius: 0 }, spec)),
  line: (spec) => node('LINE', Object.assign({ fills: [], strokes: [], height: 0 }, spec)),
  boolean: (spec, children) => node('BOOLEAN_OPERATION', Object.assign({ fills: [], strokes: [], booleanOperation: 'UNION' }, spec), children || []),
  solid,
  toFigma,
};
