import { test } from 'node:test';
import assert from 'node:assert/strict';
import { interpolate } from '../../lib/template.js';

test('interpolate replaces single variable', () => {
  const result = interpolate('Hello {name}!', { name: 'DeCIpher' });
  assert.equal(result, 'Hello DeCIpher!');
});

test('interpolate replaces multiple variables', () => {
  const result = interpolate('{a} and {b}', { a: 'foo', b: 'bar' });
  assert.equal(result, 'foo and bar');
});

test('interpolate replaces same variable used multiple times', () => {
  const result = interpolate('{x} + {x}', { x: '5' });
  assert.equal(result, '5 + 5');
});

test('interpolate leaves unknown variables untouched', () => {
  const result = interpolate('{known} and {unknown}', { known: 'hello' });
  assert.equal(result, 'hello and {unknown}');
});

test('interpolate handles multiline template values', () => {
  const result = interpolate('Log:\n{log}', { log: 'line1\nline2' });
  assert.equal(result, 'Log:\nline1\nline2');
});
