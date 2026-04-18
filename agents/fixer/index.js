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

/**
 * Propose a fix for a classified failure.
 * @param {object} classificationArtifact - from triage node
 * @param {object} context - { broken_files: [{path, content}] }
 * @param {object} config - API client config
 * @returns {Promise<object>} patch artifact
 */
export async function proposeFix(classificationArtifact, context, config) {
  const { classification, confidence, root_causes } = classificationArtifact;

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
