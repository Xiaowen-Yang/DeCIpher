import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatReport, formatSection } from '../../lib/reporter.js';

test('formatSection produces bracketed header line', () => {
  const result = formatSection('SUMMARY', 'Build failed due to missing file');
  assert.ok(result.includes('[SUMMARY]'));
  assert.ok(result.includes('Build failed due to missing file'));
});

test('formatReport includes all 7 required sections', () => {
  const report = {
    summary: 'Docker build failed',
    classification: { label: 'path_or_copy_error', confidence: 0.91 },
    evidence: ['stat src/: file does not exist'],
    patch: '--- a/Dockerfile\n+++ b/Dockerfile\n-COPY src/ .\n+COPY . .',
    verification: { command: 'docker build .', exit_code: 0, result: 'PASS', excerpt: 'Successfully built' },
    risk: { blast_radius: 'Dockerfile only', rollback_hint: 'git checkout -- Dockerfile' },
    next: 'Commit and push',
  };

  const output = formatReport(report);
  assert.ok(output.includes('[SUMMARY]'));
  assert.ok(output.includes('[CLASSIFICATION]'));
  assert.ok(output.includes('[EVIDENCE]'));
  assert.ok(output.includes('[PATCH]'));
  assert.ok(output.includes('[VERIFICATION]'));
  assert.ok(output.includes('[RISK]'));
  assert.ok(output.includes('[NEXT]'));
});

test('formatReport includes classification label and confidence', () => {
  const report = {
    summary: 'test',
    classification: { label: 'path_or_copy_error', confidence: 0.91 },
    evidence: [],
    patch: '',
    verification: { command: '', exit_code: 0, result: 'PASS', excerpt: '' },
    risk: { blast_radius: '', rollback_hint: '' },
    next: '',
  };
  const output = formatReport(report);
  assert.ok(output.includes('path_or_copy_error'));
  assert.ok(output.includes('0.91'));
});
