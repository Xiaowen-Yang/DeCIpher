import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// Override config path for tests
process.env.DECIPHER_CONFIG_DIR = join(tmpdir(), `decipher-test-${Date.now()}`);

const {
  readConfig,
  writeConfig,
  CONFIG_DEFAULTS,
  maskSecret,
  maskConfig,
  normalizeConfigKey,
  canUpdateConfigKey,
  coerceConfigValue,
  validateConfigUpdates,
} = await import('../../lib/config.js');

test('readConfig returns defaults when no config file exists', async () => {
  const config = await readConfig();
  assert.equal(config.provider, 'openai');
  assert.equal(config.max_iterations, 3);
  assert.equal(config.approval_policy, 'on-request');
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
  assert.ok('approval_policy' in CONFIG_DEFAULTS);
  assert.ok('notification_command' in CONFIG_DEFAULTS);
});

test('maskSecret hides most of the api key', () => {
  assert.equal(maskSecret('sk-1234567890'), 'sk-***90');
  assert.equal(maskSecret('abcd'), '***');
  assert.equal(maskSecret(null), null);
});

test('maskConfig redacts sensitive fields without changing other values', () => {
  const masked = maskConfig({
    provider: 'openai',
    model: 'gpt-4o',
    api_key: 'sk-1234567890',
    base_url: 'https://example.test/v1',
  });

  assert.equal(masked.provider, 'openai');
  assert.equal(masked.model, 'gpt-4o');
  assert.equal(masked.base_url, 'https://example.test/v1');
  assert.equal(masked.api_key, 'sk-***90');
});

test('normalizeConfigKey maps slash-command keys to stored keys', () => {
  assert.equal(normalizeConfigKey('api-key'), 'api_key');
  assert.equal(normalizeConfigKey('base_url'), 'base_url');
  assert.equal(normalizeConfigKey('approval-policy'), 'approval_policy');
});

test('canUpdateConfigKey constrains model and setting scopes', () => {
  assert.equal(canUpdateConfigKey('model', 'model'), true);
  assert.equal(canUpdateConfigKey('base_url', 'model'), true);
  assert.equal(canUpdateConfigKey('approval_policy', 'model'), false);
  assert.equal(canUpdateConfigKey('approval_policy', 'setting'), true);
  assert.equal(canUpdateConfigKey('max_iterations', 'setting'), true);
});

test('coerceConfigValue normalizes nullable and numeric config values', () => {
  assert.equal(coerceConfigValue('base_url', 'default'), null);
  assert.equal(coerceConfigValue('api-key', 'none'), null);
  assert.equal(coerceConfigValue('notification_command', 'null'), null);
  assert.equal(coerceConfigValue('max_iterations', '5'), 5);
  assert.equal(coerceConfigValue('model', 'gpt-5.2'), 'gpt-5.2');
});

test('validateConfigUpdates rejects invalid provider, url, and approval policy', () => {
  assert.throws(
    () => validateConfigUpdates({ provider: 'bogus' }),
    { message: /provider/i },
  );
  assert.throws(
    () => validateConfigUpdates({ base_url: 'notaurl' }),
    { message: /base_url/i },
  );
  assert.throws(
    () => validateConfigUpdates({ approval_policy: 'maybe' }),
    { message: /approval_policy/i },
  );
  assert.throws(
    () => validateConfigUpdates({ max_iterations: 0 }),
    { message: /max_iterations/i },
  );
});

test('writeConfig persists approval policy and normalized keys', async () => {
  await writeConfig({
    [normalizeConfigKey('approval-policy')]: 'never',
    [normalizeConfigKey('api-key')]: 'sk-test-value',
  });

  const config = await readConfig();
  assert.equal(config.approval_policy, 'never');
  assert.equal(config.api_key, 'sk-test-value');
});

test('coerceConfigValue rejects invalid max_iterations', () => {
  assert.throws(
    () => coerceConfigValue('max_iterations', 'zero'),
    { message: /max_iterations/i },
  );
});
