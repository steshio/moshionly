'use strict';

// Checks converted layers against Moshionly's rules without Rust, so the
// Node tests can run anywhere. The fields of each layer type come from
// `core-fields.json`, which `acceptance/` writes from core itself and checks
// is current; the value rules mirror core's validator. The full check with
// core's own code is `acceptance/run.sh`.

const fields = require('./core-fields.json');

const LAYER_HOLDERS = new Set(['artboard', 'group', 'mask_group']);

const ENUMS = {
  blend_mode: [
    'normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'color_dodge', 'color_burn',
    'hard_light', 'soft_light', 'difference', 'exclusion', 'hue', 'saturation', 'color', 'luminosity',
  ],
  align: ['left', 'center', 'right'],
  vertical_align: ['top', 'middle', 'bottom'],
  auto_size: ['width_and_height', 'height', 'fixed'],
  case: ['original', 'upper', 'lower', 'title'],
  fit: ['cover', 'contain', 'stretch'],
  position: ['inside', 'center', 'outside'],
  cap: ['butt', 'round', 'square'],
  join: ['miter', 'round', 'bevel'],
  fill_rule: ['non_zero', 'even_odd'],
};

const COLOR = /^#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$/;

/**
 * The problems with a list of layers sent to Moshionly, as sentences. An
 * empty list means the layers follow the rules.
 */
function checkLayers(nodes) {
  const problems = [];
  const byId = new Map();
  for (const node of nodes) {
    if (byId.has(node.id)) problems.push(`two layers have the id ${node.id}`);
    byId.set(node.id, node);
  }
  for (const node of nodes) {
    const where = `${node.type} ${JSON.stringify(node.name || '')} (${node.id})`;
    const fail = (message) => problems.push(`${where}: ${message}`);
    if (typeof node.id !== 'string' || node.id === '') fail('has no id');
    const known = fields[node.type];
    if (!known) {
      fail('is not a layer type');
      continue;
    }
    for (const key of Object.keys(node)) {
      if (!known.includes(key)) fail(`"${key}" is not a field of ${node.type}`);
    }
    if (node.parent !== undefined) {
      const parent = byId.get(node.parent);
      if (!parent) fail(`its parent ${node.parent} is not in the design`);
      else if (!LAYER_HOLDERS.has(parent.type)) fail(`its parent is a ${parent.type}, which cannot hold layers`);
      if (node.type === 'artboard') fail('is an artboard inside another layer');
    }
    checkValues(node, fail);
  }
  return problems;
}

function finite(value) {
  return typeof value === 'number' && Number.isFinite(value);
}

function checkValues(node, fail) {
  const number = (key, low, high) => {
    if (node[key] === undefined) return;
    if (!finite(node[key])) return fail(`"${key}" is ${node[key]}, not a number`);
    if (low !== undefined && node[key] < low) fail(`"${key}" is ${node[key]}, below ${low}`);
    if (high !== undefined && node[key] > high) fail(`"${key}" is ${node[key]}, above ${high}`);
  };
  const oneOf = (key, value, list) => {
    if (value !== undefined && !list.includes(value)) fail(`"${key}" is ${JSON.stringify(value)}`);
  };
  number('x');
  number('y');
  number('width', node.type === 'artboard' ? 1 : 0);
  number('height', node.type === 'artboard' ? 1 : 0);
  number('rotation');
  number('opacity', 0, 100);
  number('blur', 0);
  oneOf('blend_mode', node.blend_mode, ENUMS.blend_mode);
  for (const key of ['hidden', 'locked', 'clip']) {
    if (node[key] !== undefined && typeof node[key] !== 'boolean') fail(`"${key}" is not true or false`);
  }
  if (node.type === 'artboard') {
    for (const key of ['rotation', 'opacity', 'blur', 'blend_mode', 'shadow']) {
      if (node[key] !== undefined) fail(`an artboard cannot have "${key}"`);
    }
  }
  if (node.shadow !== undefined) {
    const shadow = node.shadow;
    if (!COLOR.test(shadow.color)) fail('the shadow colour is not a colour');
    if (![shadow.x, shadow.y].every(finite) || !(finite(shadow.blur) && shadow.blur >= 0)) fail('the shadow has bad numbers');
  }
  if (node.fill !== undefined) checkPaint(node.fill, 'fill', fail);
  if (node.stroke !== undefined) {
    const stroke = node.stroke;
    checkPaint(stroke.paint === undefined ? '#000000' : stroke.paint, 'stroke.paint', fail);
    if (stroke.width !== undefined && !(finite(stroke.width) && stroke.width >= 0)) fail('the stroke width is bad');
    oneOf('stroke.position', stroke.position, ENUMS.position);
    oneOf('stroke.cap', stroke.cap, ENUMS.cap);
    oneOf('stroke.join', stroke.join, ENUMS.join);
    for (const key of Object.keys(stroke)) {
      if (!['paint', 'width', 'position', 'cap', 'join'].includes(key)) fail(`"stroke.${key}" is not a stroke field`);
    }
  }
  if (node.corner_radius !== undefined) {
    const radii = Array.isArray(node.corner_radius) ? node.corner_radius : [node.corner_radius];
    if (!(radii.length === 1 || radii.length === 4) || !radii.every((radius) => finite(radius) && radius >= 0)) {
      fail('"corner_radius" is not one number or four');
    }
  }
  if (node.type === 'ellipse') {
    number('start_angle');
    number('sweep', -360, 360);
    number('inner_radius', 0, 1);
  }
  if (node.type === 'star') {
    number('points', 3, 64);
    if (node.points !== undefined && !Number.isInteger(node.points)) fail('"points" is not a whole number');
    number('inner_radius', 0, 1);
  }
  if (node.type === 'path' && node.path !== undefined) {
    oneOf('path.fill_rule', node.path.fill_rule, ENUMS.fill_rule);
    for (const subpath of node.path.subpaths || []) {
      for (const anchor of subpath.anchors) {
        for (const point of [anchor.point, anchor.in, anchor.out]) {
          if (point !== undefined && !(Array.isArray(point) && point.length === 2 && point.every(finite))) {
            fail('a path point is not two numbers');
          }
        }
      }
    }
  }
  if (node.type === 'text') {
    if (typeof node.content !== 'string') fail('"content" is not text');
    if (node.font) {
      if (!node.font.family || !node.font.family.trim()) fail('the font has no family');
      if (!(Number.isInteger(node.font.weight) && node.font.weight >= 1 && node.font.weight <= 1000)) fail('the font weight is bad');
    }
    if (node.font_size !== undefined && !(finite(node.font_size) && node.font_size > 0)) fail('"font_size" is not above 0');
    if (node.line_height !== undefined && !(finite(node.line_height) && node.line_height > 0)) fail('"line_height" is not above 0');
    number('letter_spacing');
    oneOf('align', node.align, ENUMS.align);
    oneOf('vertical_align', node.vertical_align, ENUMS.vertical_align);
    oneOf('auto_size', node.auto_size, ENUMS.auto_size);
    oneOf('case', node.case, ENUMS.case);
  }
  if (node.type === 'image') {
    // Moshionly's import takes embedded pictures only.
    if (typeof node.source !== 'string' || !/^data:image\/[a-z]+;base64,/.test(node.source)) {
      fail('"source" is not an embedded picture');
    }
    oneOf('fit', node.fit, ENUMS.fit);
  }
}

function checkPaint(paint, field, fail) {
  if (typeof paint === 'string') {
    if (!COLOR.test(paint)) fail(`"${field}" is ${paint}, not a colour`);
    return;
  }
  const points = paint.type === 'linear_gradient' ? ['start', 'end'] : ['center', 'radius'];
  if (paint.type !== 'linear_gradient' && paint.type !== 'radial_gradient') return fail(`"${field}" is not a paint`);
  for (const key of points) {
    const point = paint[key];
    if (point !== undefined && !(Array.isArray(point) && point.length === 2 && point.every(finite))) {
      fail(`"${field}.${key}" is not two numbers`);
    }
  }
  if (!Array.isArray(paint.stops) || paint.stops.length < 2) return fail(`"${field}" has fewer than two stops`);
  let previous = 0;
  for (const stop of paint.stops) {
    if (!COLOR.test(stop.color)) fail(`"${field}" has a stop that is not a colour`);
    if (!(finite(stop.offset) && stop.offset >= previous && stop.offset <= 1)) fail(`"${field}" has stops out of order`);
    previous = stop.offset;
  }
}

module.exports = { checkLayers };
