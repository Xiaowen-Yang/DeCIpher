import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const {
  resolveTarget,
  detectAction,
  askApproval,
  shouldConfirmWriteback,
} = await import('../../agents/executor/index.js');

test('resolveTarget detects a scenario directory from natural-language input', async () => {
  const root = await mkdtemp(join(tmpdir(), 'decipher-executor-'));
  const scenarioDir = join(root, 'scenarios', 'sample-scenario');
  await mkdir(scenarioDir, { recursive: true });
  await writeFile(join(scenarioDir, 'metadata.json'), JSON.stringify({ id: 'sample', category: 'docker' }), 'utf8');

  try {
    const target = await resolveTarget(`please repair "${scenarioDir}" build this container`);
    assert.ok(target);
    assert.equal(target.type, 'scenario');
    assert.equal(target.meta.id, 'sample');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('resolveTarget detects a Dockerfile path', async () => {
  const root = await mkdtemp(join(tmpdir(), 'decipher-dockerfile-'));
  const dockerfilePath = join(root, 'Dockerfile');
  await writeFile(dockerfilePath, 'FROM node:20-alpine\n', 'utf8');

  try {
    const target = await resolveTarget(`fix this Dockerfile ${dockerfilePath}`);
    assert.ok(target);
    assert.equal(target.type, 'dockerfile');
    assert.equal(target.path, dockerfilePath);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('detectAction defaults scenario requests to fix loop', () => {
  assert.equal(detectAction('repair this scenario', 'scenario'), 'fix');
});

test('detectAction prefers triage_only when user asks to diagnose only', () => {
  assert.equal(detectAction('triage this failure log', 'logfile'), 'triage_only');
});

test('detectAction keeps scenario build requests on the fix loop', () => {
  assert.equal(detectAction('build this container', 'scenario'), 'fix');
});

test('detectAction maps build language to docker_build for raw Dockerfile targets', () => {
  assert.equal(detectAction('build this container', 'dockerfile'), 'docker_build');
});

test('askApproval auto-approves never policy without prompting', async () => {
  let prompted = false;
  const approved = await askApproval({
    question: () => { prompted = true; },
  }, {
    approved: false,
    approvalPolicy: 'never',
  });

  assert.equal(approved, true);
  assert.equal(prompted, false);
});

test('askApproval auto-approves on-failure policy without prompting', async () => {
  let prompted = false;
  const approved = await askApproval({
    question: () => { prompted = true; },
  }, {
    approved: false,
    approvalPolicy: 'on-failure',
  });

  assert.equal(approved, true);
  assert.equal(prompted, false);
});

test('askApproval prompts once for on-request policy', async () => {
  let prompts = 0;
  const rl = {
    question: (_prompt, cb) => {
      prompts += 1;
      cb('y');
    },
  };

  const state = { approved: false, approvalPolicy: 'on-request' };
  const approved = await askApproval(rl, state);

  assert.equal(approved, true);
  assert.equal(state.approved, true);
  assert.equal(prompts, 1);
});

test('shouldConfirmWriteback requires confirmation except for never policy', () => {
  assert.equal(shouldConfirmWriteback({ approvalPolicy: 'on-request' }), true);
  assert.equal(shouldConfirmWriteback({ approvalPolicy: 'on-failure' }), true);
  assert.equal(shouldConfirmWriteback({ approvalPolicy: 'never' }), false);
});
