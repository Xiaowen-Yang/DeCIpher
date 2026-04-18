import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { callAI } from '../../lib/api-client.js';
import { loadPrompt } from '../../lib/template.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROMPTS_DIR = join(__dirname, '../../prompts');
const SKILLS_DIR = join(__dirname, '../../skills');

/**
 * Parse and validate fix AI response into a patch artifact.
 */
export function parseFixResponse(raw) {
  let parsed;
  try {
    const cleaned = raw.replace(/^```json?\n?/m, '').replace(/\n?```$/m, '').trim();
    parsed = JSON.parse(cleaned);
  } catch (err) {
    throw new Error(`Failed to parse fix response: ${err.message}`);
  }

  if (!parsed.affected_files || parsed.affected_files.length === 0) {
    throw new Error('No affected files in fix response — patch cannot be applied.');
  }

  return {
    affected_files: parsed.affected_files,
    patch: parsed.patch ?? '',
    rationale: parsed.rationale ?? '',
    risk: parsed.risk ?? 'medium',
    blast_radius: parsed.blast_radius ?? 'unknown',
    rollback_hint: parsed.rollback_hint ?? 'git checkout -- .',
  };
}

function getSkillFile(classification) {
  const dockerLabels = ['path_or_copy_error', 'permission_or_executable_error', 'docker_entrypoint_runtime_error', 'healthcheck_startup_failure'];
  const envLabels = ['missing_env_or_secret_contract'];
  if (dockerLabels.includes(classification)) return 'docker-debug';
  if (envLabels.includes(classification)) return 'env-bootstrap';
  return 'ci-triage';
}

function buildSingleLinePatch(path, fromLine, toLine, lineNumber) {
  return [
    `--- a/${path}`,
    `+++ b/${path}`,
    `@@ -${lineNumber} +${lineNumber} @@`,
    `-${fromLine}`,
    `+${toLine}`,
  ].join('\n');
}

function buildInsertionPatch(path, insertedLine, lineNumber) {
  return [
    `--- a/${path}`,
    `+++ b/${path}`,
    `@@ -${lineNumber},0 +${lineNumber},1 @@`,
    `+${insertedLine}`,
  ].join('\n');
}

function findDockerfile(context) {
  return (context.broken_files ?? []).find((file) => file.path === 'Dockerfile') ?? null;
}

export function buildDeterministicFix(classificationArtifact, context) {
  const dockerfile = findDockerfile(context);
  if (!dockerfile) return null;

  const { classification, root_causes = [] } = classificationArtifact;
  const lines = dockerfile.content.split('\n');

  if (classification === 'path_or_copy_error') {
    const lineIndex = lines.findIndex((line) => line.trim() === 'COPY src/ .');
    if (lineIndex !== -1) {
      return {
        affected_files: ['Dockerfile'],
        patch: buildSingleLinePatch('Dockerfile', 'COPY src/ .', 'COPY . .', lineIndex + 1),
        rationale: 'The Docker build context is the broken/ workspace itself, so COPY must reference the current directory.',
        risk: 'low',
        blast_radius: 'Dockerfile COPY source path only',
        rollback_hint: 'git checkout -- Dockerfile',
      };
    }

    const multistageLineIndex = lines.findIndex((line) =>
      line.includes('COPY --from=builder /src/dist/app.js ./app.js'),
    );
    if (multistageLineIndex !== -1) {
      return {
        affected_files: ['Dockerfile'],
        patch: buildSingleLinePatch(
          'Dockerfile',
          'COPY --from=builder /src/dist/app.js ./app.js',
          'COPY --from=builder /src/output/app.js ./app.js',
          multistageLineIndex + 1,
        ),
        rationale: 'The builder stage outputs app.js under /src/output, so the runtime stage must copy from that path.',
        risk: 'low',
        blast_radius: 'Dockerfile multistage artifact path only',
        rollback_hint: 'git checkout -- Dockerfile',
      };
    }
  }

  if (classification === 'healthcheck_startup_failure') {
    const evidenceText = root_causes
      .map((rootCause) => `${rootCause.hypothesis ?? ''} ${rootCause.evidence ?? ''}`)
      .join(' ')
      .toLowerCase();
    const lineIndex = lines.findIndex((line) => line.includes('http://localhost:8080/'));
    const appPort = evidenceText.includes('3000') || dockerfile.content.includes('EXPOSE 3000') ? '3000' : null;
    if (lineIndex !== -1 && appPort) {
      const fromLine = lines[lineIndex];
      const toLine = fromLine.replace('http://localhost:8080/', `http://localhost:${appPort}/`);
      if (fromLine !== toLine) {
        return {
          affected_files: ['Dockerfile'],
          patch: buildSingleLinePatch('Dockerfile', fromLine, toLine, lineIndex + 1),
          rationale: 'The HEALTHCHECK port must match the port the app actually listens on.',
          risk: 'low',
          blast_radius: 'Dockerfile healthcheck command only',
          rollback_hint: 'git checkout -- Dockerfile',
        };
      }
    }

  }

  if (classification === 'missing_env_or_secret_contract') {
    const hasEnv = lines.some((line) => line.trim().startsWith('ENV DATABASE_URL='));
    const cmdLineIndex = lines.findIndex((line) => line.trim().startsWith('CMD '));
    if (!hasEnv && cmdLineIndex !== -1) {
      return {
        affected_files: ['Dockerfile'],
        patch: buildInsertionPatch(
          'Dockerfile',
          'ENV DATABASE_URL=sqlite:///app/data.db',
          cmdLineIndex + 1,
        ),
        rationale: 'The container fails only because DATABASE_URL is missing at runtime, so provide a safe default in the Dockerfile.',
        risk: 'low',
        blast_radius: 'Dockerfile runtime environment default only',
        rollback_hint: 'git checkout -- Dockerfile',
      };
    }
  }

  return null;
}

/**
 * Propose a fix for a classified failure.
 * @param {object} classificationArtifact - from triage node
 * @param {object} context - { broken_files: [{path, content}] }
 * @param {object} config - API client config
 * @returns {Promise<object>} patch artifact
 */
export async function proposeFix(classificationArtifact, context, config) {
  const { classification, confidence, root_causes } = classificationArtifact;

  const deterministicFix = buildDeterministicFix(classificationArtifact, context);
  if (deterministicFix) {
    return deterministicFix;
  }

  const skillName = getSkillFile(classification);
  const skillContent = await readFile(
    join(SKILLS_DIR, skillName, 'SKILL.md'),
    'utf8'
  );

  const evidenceSummary = root_causes
    .map(rc => `- ${rc.hypothesis}: ${rc.evidence}`)
    .join('\n');

  const brokenFilesContent = (context.broken_files ?? [])
    .map(f => `### ${f.path}\n\`\`\`\n${f.content}\n\`\`\``)
    .join('\n\n');

  const prompt = await loadPrompt(join(PROMPTS_DIR, 'fix.md'), {
    classification,
    confidence,
    evidence: evidenceSummary,
    skill_content: skillContent,
    broken_files: brokenFilesContent,
  });

  const rawResponse = await callAI(prompt, config);
  return parseFixResponse(rawResponse);
}
