import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseTriageResponse, TAXONOMY } from '../../agents/triage/index.js';

test('parseTriageResponse extracts valid classification artifact', () => {
  const raw = JSON.stringify({
    classification: 'path_or_copy_error',
    confidence: 0.91,
    root_causes: [
      { hypothesis: 'COPY src/ does not exist', evidence: 'stat src/: file does not exist', confidence: 0.91 }
    ],
    excluded: ['permission_or_executable_error'],
    needs_more_evidence: false,
  });

  const result = parseTriageResponse(raw);
  assert.equal(result.classification, 'path_or_copy_error');
  assert.equal(result.confidence, 0.91);
  assert.equal(result.needs_more_evidence, false);
  assert.ok(Array.isArray(result.root_causes));
});

test('parseTriageResponse throws on invalid JSON', () => {
  assert.throws(
    () => parseTriageResponse('not json {{{'),
    { message: /Failed to parse triage response/ }
  );
});

test('parseTriageResponse throws when classification not in taxonomy', () => {
  const raw = JSON.stringify({
    classification: 'made_up_label',
    confidence: 0.9,
    root_causes: [],
    excluded: [],
    needs_more_evidence: false,
  });
  assert.throws(
    () => parseTriageResponse(raw),
    { message: /Invalid classification/ }
  );
});

test('TAXONOMY contains all 10 required labels', () => {
  const required = [
    'dependency_version_mismatch',
    'missing_env_or_secret_contract',
    'path_or_copy_error',
    'permission_or_executable_error',
    'docker_entrypoint_runtime_error',
    'healthcheck_startup_failure',
    'test_regression',
    'ci_config_drift',
    'cache_or_lockfile_issue',
    'needs_more_evidence',
  ];
  for (const label of required) {
    assert.ok(TAXONOMY.includes(label), `Missing taxonomy label: ${label}`);
  }
});
