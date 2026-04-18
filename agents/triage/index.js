import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { callAI } from '../../lib/api-client.js';
import { loadPrompt } from '../../lib/template.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROMPTS_DIR = join(__dirname, '../../prompts');
const SKILLS_DIR = join(__dirname, '../../skills');

export const TAXONOMY = [
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

/**
 * Parse and validate triage AI response into a classification artifact.
 * @param {string} raw - Raw text from AI API
 * @returns {object} classification artifact
 */
export function parseTriageResponse(raw) {
  let parsed;
  try {
    // Strip markdown fences if the model wrapped the JSON
    const cleaned = raw.replace(/^```json?\n?/m, '').replace(/\n?```$/m, '').trim();
    parsed = JSON.parse(cleaned);
  } catch (err) {
    throw new Error(`Failed to parse triage response: ${err.message}\nRaw: ${raw.slice(0, 200)}`);
  }

  if (!TAXONOMY.includes(parsed.classification)) {
    throw new Error(`Invalid classification label: "${parsed.classification}". Must be one of: ${TAXONOMY.join(', ')}`);
  }

  return {
    classification: parsed.classification,
    confidence: parsed.confidence ?? 0,
    root_causes: parsed.root_causes ?? [],
    excluded: parsed.excluded ?? [],
    needs_more_evidence: parsed.needs_more_evidence ?? false,
  };
}

function getSkillFile(category) {
  const map = {
    docker: 'docker-debug',
    ci: 'ci-triage',
    env: 'env-bootstrap',
  };
  return map[category] ?? 'ci-triage';
}

/**
 * Triage a failure log and return a classification artifact.
 * @param {string} logFile - Path to the failure log file
 * @param {object} context - { category, broken_files_content }
 * @param {object} config - API client config
 * @returns {Promise<object>} classification artifact
 */
export async function triageLog(logFile, context = {}, config) {
  const failureLog = await readFile(logFile, 'utf8');

  const skillName = getSkillFile(context.category ?? 'ci');
  const skillContent = await readFile(
    join(SKILLS_DIR, skillName, 'SKILL.md'),
    'utf8'
  );

  const prompt = await loadPrompt(join(PROMPTS_DIR, 'triage.md'), {
    taxonomy: TAXONOMY.join('\n'),
    skill_content: skillContent,
    failure_log: failureLog,
    context_summary: context.summary ?? 'No additional context provided.',
  });

  const rawResponse = await callAI(prompt, config);
  return parseTriageResponse(rawResponse);
}
