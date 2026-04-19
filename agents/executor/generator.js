/**
 * Generation subsystem — creates missing files for a mission target.
 *
 * generateFiles(targetDir, missionGoal, config)
 *   → { generated_files, rationale, needs_clarification, writtenPaths }
 */

import { readFile, readdir, writeFile, mkdir } from "node:fs/promises";
import { join, dirname } from "node:path";
import { callAI } from "../../lib/api-client.js";
import { loadPrompt } from "../../lib/template.js";
import { fileURLToPath } from "node:url";

const PROMPT_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  "../../prompts/generate.md",
);

const INSPECTABLE_NAMES = new Set([
  "Dockerfile",
  "docker-compose.yml",
  "docker-compose.yaml",
  "package.json",
  ".github",
  "Makefile",
  "requirements.txt",
  "setup.py",
  "pyproject.toml",
  "go.mod",
  ".nvmrc",
  ".node-version",
]);

/**
 * Build a short summary of a directory for the prompt context.
 * @param {string} dir  Absolute path to inspect
 * @returns {Promise<string>}
 */
export async function buildContextSummary(dir) {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return `Directory: ${dir} (unreadable or missing)`;
  }

  const names = entries.map((e) => (e.isDirectory() ? `${e.name}/` : e.name));
  const lines = [`Directory: ${dir}`, `Files: ${names.join(", ") || "(empty)"}`];

  // Read key files to surface their content in the context
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    if (!INSPECTABLE_NAMES.has(entry.name)) continue;
    try {
      const content = await readFile(join(dir, entry.name), "utf8");
      lines.push(`\n--- ${entry.name} ---\n${content.slice(0, 800)}`);
    } catch {
      // skip unreadable files
    }
  }

  return lines.join("\n");
}

/**
 * Parse the AI's JSON response for the generation prompt.
 * Returns a safe object with defaults on parse failure.
 *
 * @param {string} raw  Raw AI response text
 * @returns {{ generated_files: Array<{path:string,content:string}>, rationale: string, needs_clarification: string|null }}
 */
export function parseGenerateResponse(raw) {
  const text = raw.replace(/^```(?:json)?\s*/i, "").replace(/\s*```$/, "").trim();
  try {
    const parsed = JSON.parse(text);
    return {
      generated_files: Array.isArray(parsed.generated_files) ? parsed.generated_files : [],
      rationale: typeof parsed.rationale === "string" ? parsed.rationale : "",
      needs_clarification: parsed.needs_clarification ?? null,
    };
  } catch {
    return {
      generated_files: [],
      rationale: "",
      needs_clarification: `Could not parse generation response: ${text.slice(0, 200)}`,
    };
  }
}

/**
 * List existing file paths under targetDir (top-level only) as a string.
 * @param {string} dir
 * @returns {Promise<string>}
 */
async function listExistingFiles(dir) {
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    if (entries.length === 0) return "(none)";
    return entries.map((e) => (e.isDirectory() ? `${e.name}/` : e.name)).join("\n");
  } catch {
    return "(directory not accessible)";
  }
}

/**
 * Main generation entry point.
 *
 * @param {string} targetDir   Absolute path to the directory to generate into
 * @param {string} missionGoal Human-readable mission goal
 * @param {object} config      API config (provider, model, api_key, …)
 * @returns {Promise<{
 *   generated_files: Array<{path:string,content:string}>,
 *   rationale: string,
 *   needs_clarification: string|null,
 *   writtenPaths: string[],
 * }>}
 */
export async function generateFiles(targetDir, missionGoal, config) {
  const contextSummary = await buildContextSummary(targetDir);
  const existingFiles = await listExistingFiles(targetDir);

  const prompt = await loadPrompt(PROMPT_PATH, {
    mission_goal: missionGoal ?? "generate required files",
    context_summary: contextSummary,
    existing_files: existingFiles,
  });

  const raw = await callAI(prompt, config);
  const parsed = parseGenerateResponse(raw);

  if (parsed.needs_clarification) {
    return {
      generated_files: parsed.generated_files,
      rationale: parsed.rationale,
      needs_clarification: parsed.needs_clarification,
      writtenPaths: [],
    };
  }

  const writtenPaths = [];
  for (const file of parsed.generated_files) {
    if (!file.path || typeof file.content !== "string") continue;
    const absPath = join(targetDir, file.path);
    await mkdir(dirname(absPath), { recursive: true });
    await writeFile(absPath, file.content, "utf8");
    writtenPaths.push(absPath);
  }

  return {
    generated_files: parsed.generated_files,
    rationale: parsed.rationale,
    needs_clarification: null,
    writtenPaths,
  };
}
