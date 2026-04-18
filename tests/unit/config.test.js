import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { rm } from 'node:fs/promises';

// Override config path for tests
process.env.DECIPHER_CONFIG_DIR = join(tmpdir(), `decipher-test-${Date.now()}`);

const { readConfig, writeConfig, CONFIG_DEFAULTS } = await import('../../lib/config.js');

test('readConfig returns defaults when no config file exists', async () => {
  const config = await readConfig();
  assert.equal(config.provider, 'openai');
  assert.equal(config.max_iterations, 3);
  assert.equal(config.auto_approve, false);
});

test('writeConfig persists and readConfig retrieves values', async () => {
  await writeConfig({ provider: 'anthropic', api_key: 'test-key-123' });
  const config = await readConfig();
  assert.equal(config.provider, 'anthropic');
  assert.equal(config.api_key, 'test-key-123');
  assert.equal(config.max_iterations, 3); // default preserved
});

test('CONFIG_DEFAULTS has required keys', () => {
  assert.ok('provider' in CONFIG_DEFAULTS);
  assert.ok('model' in CONFIG_DEFAULTS);
  assert.ok('max_iterations' in CONFIG_DEFAULTS);
  assert.ok('auto_approve' in CONFIG_DEFAULTS);
});
