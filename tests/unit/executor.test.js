import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const { resolveTarget, detectAction } = await import('../../agents/executor/index.js');

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

test('detectAction maps build language to docker_build', () => {
  assert.equal(detectAction('build this container', 'scenario'), 'docker_build');
});
