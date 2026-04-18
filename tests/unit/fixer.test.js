import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseFixResponse, buildDeterministicFix } from '../../agents/fixer/index.js';

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

test('buildDeterministicFix returns COPY path repair for docker path_or_copy_error', () => {
  const artifact = buildDeterministicFix(
    {
      classification: 'path_or_copy_error',
      confidence: 1,
      root_causes: [],
    },
    {
      broken_files: [
        {
          path: 'Dockerfile',
          content: 'FROM node:18-alpine\nWORKDIR /app\nCOPY src/ .\nRUN npm install\n',
        },
      ],
    },
  );

  assert.ok(artifact, 'expected deterministic artifact');
  assert.deepEqual(artifact.affected_files, ['Dockerfile']);
  assert.match(artifact.patch, /-COPY src\/ \./);
  assert.match(artifact.patch, /\+COPY \. \./);
  assert.equal(artifact.risk, 'low');
});

test('buildDeterministicFix returns healthcheck port repair when Dockerfile shows 8080 vs 3000 mismatch', () => {
  const artifact = buildDeterministicFix(
    {
      classification: 'healthcheck_startup_failure',
      confidence: 1,
      root_causes: [
        {
          hypothesis: 'port mismatch',
          evidence: 'HEALTHCHECK probes localhost:8080 but the app listens on port 3000',
        },
      ],
    },
    {
      broken_files: [
        {
          path: 'Dockerfile',
          content: [
            'FROM node:18-alpine',
            'EXPOSE 3000',
            'HEALTHCHECK --interval=5s --timeout=3s --retries=3 \\',
            '  CMD wget -qO- http://localhost:8080/ || exit 1',
            'CMD ["node", "server.js"]',
          ].join('\n'),
        },
      ],
    },
  );

  assert.ok(artifact, 'expected deterministic artifact');
  assert.deepEqual(artifact.affected_files, ['Dockerfile']);
  assert.match(artifact.patch, /localhost:8080/);
  assert.match(artifact.patch, /localhost:3000/);
});

test('buildDeterministicFix returns multistage artifact path repair for docker path_or_copy_error', () => {
  const artifact = buildDeterministicFix(
    {
      classification: 'path_or_copy_error',
      confidence: 1,
      root_causes: [],
    },
    {
      broken_files: [
        {
          path: 'Dockerfile',
          content: [
            'FROM node:18-alpine AS builder',
            'WORKDIR /src',
            'RUN mkdir -p output && cp index.js output/app.js',
            'FROM node:18-alpine AS runner',
            'WORKDIR /app',
            'COPY --from=builder /src/dist/app.js ./app.js',
            'CMD ["node", "app.js"]',
          ].join('\n'),
        },
      ],
    },
  );

  assert.ok(artifact, 'expected deterministic artifact');
  assert.deepEqual(artifact.affected_files, ['Dockerfile']);
  assert.match(artifact.patch, /\/src\/dist\/app\.js/);
  assert.match(artifact.patch, /\/src\/output\/app\.js/);
});

test('buildDeterministicFix inserts a default DATABASE_URL for missing env runtime failures', () => {
  const artifact = buildDeterministicFix(
    {
      classification: 'missing_env_or_secret_contract',
      confidence: 1,
      root_causes: [],
    },
    {
      broken_files: [
        {
          path: 'Dockerfile',
          content: [
            'FROM node:18-alpine',
            'WORKDIR /app',
            'COPY package.json server.js ./',
            'EXPOSE 3000',
            'CMD ["node", "server.js"]',
          ].join('\n'),
        },
      ],
    },
  );

  assert.ok(artifact, 'expected deterministic artifact');
  assert.deepEqual(artifact.affected_files, ['Dockerfile']);
  assert.match(artifact.patch, /ENV DATABASE_URL=sqlite:\/\/\/app\/data\.db/);
});

test('buildDeterministicFix returns null when no deterministic Docker repair matches', () => {
  const artifact = buildDeterministicFix(
    {
      classification: 'docker_entrypoint_runtime_error',
      confidence: 1,
      root_causes: [],
    },
    {
      broken_files: [
        {
          path: 'Dockerfile',
          content: 'FROM node:18-alpine\nCMD ["node", "server.js"]\n',
        },
      ],
    },
  );

  assert.equal(artifact, null);
});
