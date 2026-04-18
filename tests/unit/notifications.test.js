import { test } from 'node:test';
import assert from 'node:assert/strict';

const { runCompletionNotification } = await import('../../lib/notifications.js');

test('runCompletionNotification returns skipped when no command is configured', async () => {
  const result = await runCompletionNotification(null, {
    status: 'PASS',
    targetPath: '/tmp/example',
  });

  assert.deepEqual(result, { skipped: true });
});

test('runCompletionNotification executes configured shell hook with env vars', async () => {
  const result = await runCompletionNotification(
    `printf "%s|%s" "$DECIPHER_STATUS" "$DECIPHER_TARGET_PATH"`,
    {
      status: 'PASS',
      targetPath: '/tmp/example',
      workspacePath: '/tmp/workspace',
    },
  );

  assert.equal(result.skipped, false);
  assert.equal(result.exitCode, 0);
  assert.equal(result.stdout, 'PASS|/tmp/example');
});
