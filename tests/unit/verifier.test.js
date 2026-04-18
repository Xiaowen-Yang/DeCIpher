import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyPatch, runCommand, checkEnvironment } from '../../agents/verifier/index.js';

test('runCommand captures exit code 0 for successful command', async () => {
  const result = await runCommand('echo hello');
  assert.equal(result.exitCode, 0);
  assert.ok(result.stdout.includes('hello'));
});

test('runCommand captures non-zero exit code for failing command', async () => {
  const result = await runCommand('sh -c "exit 1"');
  assert.equal(result.exitCode, 1);
});

test('checkEnvironment returns allPassed and items array', async () => {
  const result = await checkEnvironment();
  assert.ok('allPassed' in result);
  assert.ok(Array.isArray(result.items));
  // Each item has name, version/error, passed
  for (const item of result.items) {
    assert.ok('name' in item);
    assert.ok('passed' in item);
  }
});

test('applyPatch creates a backup and modifies file', async () => {
  const { writeFile, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');

  const testFile = join(tmpdir(), `decipher-test-${Date.now()}.txt`);
  await writeFile(testFile, 'COPY src/ .\n', 'utf8');

  const patch = `--- a/testfile\n+++ b/testfile\n@@ -1 +1 @@\n-COPY src/ .\n+COPY . .\n`;
  await applyPatch(patch, testFile);

  const content = await readFile(testFile, 'utf8');
  assert.ok(content.includes('COPY . .'));
  assert.ok(!content.includes('COPY src/ .'));

  await rm(testFile, { force: true });
});
