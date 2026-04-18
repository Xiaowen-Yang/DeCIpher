import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseFixResponse } from '../../agents/fixer/index.js';

test('parseFixResponse extracts valid patch artifact', () => {
  const raw = JSON.stringify({
    affected_files: ['Dockerfile'],
    patch: '--- a/Dockerfile\n+++ b/Dockerfile\n@@ -3 +3 @@\n-COPY src/ .\n+COPY . .',
    rationale: 'The COPY source path does not exist.',
    risk: 'low',
    blast_radius: 'Dockerfile only',
    rollback_hint: 'git checkout -- Dockerfile',
  });

  const result = parseFixResponse(raw);
  assert.deepEqual(result.affected_files, ['Dockerfile']);
  assert.ok(result.patch.includes('-COPY src/ .'));
  assert.ok(result.patch.includes('+COPY . .'));
  assert.equal(result.risk, 'low');
  assert.ok(result.rollback_hint.startsWith('git'));
});

test('parseFixResponse throws on invalid JSON', () => {
  assert.throws(
    () => parseFixResponse('not json'),
    { message: /Failed to parse fix response/ }
  );
});

test('parseFixResponse throws when affected_files is empty', () => {
  const raw = JSON.stringify({
    affected_files: [],
    patch: 'some patch',
    rationale: 'reason',
    risk: 'low',
    blast_radius: 'none',
    rollback_hint: 'git checkout -- .',
  });
  assert.throws(
    () => parseFixResponse(raw),
    { message: /No affected files/ }
  );
});
