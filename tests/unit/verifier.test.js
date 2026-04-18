import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyPatch, runCommand, runVerification, checkEnvironment } from '../../agents/verifier/index.js';

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
  for (const item of result.items) {
    assert.ok('name' in item);
    assert.ok('passed' in item);
  }
});

// ── applyPatch tests ────────────────────────────────────────

test('applyPatch: replacement hunk (- then +)', async () => {
  const { writeFile, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');

  const testFile = join(tmpdir(), `decipher-test-replace-${Date.now()}.txt`);
  await writeFile(testFile, 'COPY src/ .\n', 'utf8');

  const patch = `--- a/testfile\n+++ b/testfile\n@@ -1 +1 @@\n-COPY src/ .\n+COPY . .\n`;
  await applyPatch(patch, testFile);

  const content = await readFile(testFile, 'utf8');
  assert.ok(content.includes('COPY . .'), 'replacement line should be present');
  assert.ok(!content.includes('COPY src/ .'), 'original line should be removed');

  await rm(testFile, { force: true });
  await rm(`${testFile}.bak`, { force: true });
});

test('applyPatch: pure insertion hunk (no paired removal)', async () => {
  // Regression: before fix, insertion-only hunks were silently dropped
  const { writeFile, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');

  const testFile = join(tmpdir(), `decipher-test-insert-${Date.now()}.txt`);
  const original = 'COPY entrypoint.sh /entrypoint.sh\nENTRYPOINT ["/entrypoint.sh"]\n';
  await writeFile(testFile, original, 'utf8');

  // Insert RUN chmod +x between the two lines
  const patch = [
    '--- a/Dockerfile',
    '+++ b/Dockerfile',
    '@@ -1,2 +1,3 @@',
    ' COPY entrypoint.sh /entrypoint.sh',
    '+RUN chmod +x /entrypoint.sh',
    ' ENTRYPOINT ["/entrypoint.sh"]',
  ].join('\n');

  await applyPatch(patch, testFile);

  const content = await readFile(testFile, 'utf8');
  assert.ok(content.includes('RUN chmod +x /entrypoint.sh'), 'inserted line must appear');
  assert.ok(content.includes('COPY entrypoint.sh /entrypoint.sh'), 'first line must be preserved');
  assert.ok(content.includes('ENTRYPOINT ["/entrypoint.sh"]'), 'last line must be preserved');

  // Insertion must appear between COPY and ENTRYPOINT
  const lines = content.split('\n').filter(Boolean);
  const copyIdx = lines.findIndex(l => l.includes('COPY entrypoint.sh'));
  const chmodIdx = lines.findIndex(l => l.includes('RUN chmod'));
  const entryIdx = lines.findIndex(l => l.includes('ENTRYPOINT'));
  assert.ok(copyIdx < chmodIdx, 'chmod must come after COPY');
  assert.ok(chmodIdx < entryIdx, 'chmod must come before ENTRYPOINT');

  await rm(testFile, { force: true });
  await rm(`${testFile}.bak`, { force: true });
});

// ── runVerification PASS/FAIL detection ────────────────────

test('runVerification: reports FAIL when stdout ends with FAIL (exit 0)', async () => {
  // Regression: `false && echo PASS || echo FAIL` exits 0 (echo FAIL succeeds)
  // but must be reported as FAIL
  const result = await runVerification('false && echo PASS || echo FAIL');
  assert.equal(result.result, 'FAIL', 'FAIL marker in stdout must override exit 0');
});

test('runVerification: reports PASS when stdout ends with PASS', async () => {
  const result = await runVerification('true && echo PASS || echo FAIL');
  assert.equal(result.result, 'PASS');
  assert.equal(result.exit_code, 0);
});

test('runVerification: falls back to exit code when no PASS/FAIL marker', async () => {
  const pass = await runVerification('echo "all good"');
  assert.equal(pass.result, 'PASS');

  const fail = await runVerification('sh -c "exit 2"');
  assert.equal(fail.result, 'FAIL');
});
