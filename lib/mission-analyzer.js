/**
 * LLM-based mission analyzer.
 *
 * Instead of rigid regex pattern matching, this module sends the user's
 * input to the AI and asks it to reason about intent and produce a plan.
 * This is the Codex Plan Mode equivalent for DeCIpher.
 *
 * Falls back to target-type inference if the API call fails.
 */

import { readFile, readdir } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { callAI } from "./api-client.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROMPTS_DIR = join(__dirname, "../prompts");
const SCENARIOS_DIR = join(__dirname, "../scenarios");

/**
 * @typedef {object} MissionAnalysis
 * @property {string}   understood_as          — one sentence, what we think the user wants
 * @property {string}   action                 — fix|generate|docker_build|triage_only|build_start|benchmark_run
 * @property {string[]} steps                  — ordered plan steps
 * @property {boolean}  requires_clarification
 * @property {string|null} clarification_question
 * @property {boolean}  inferred               — true when derived from target type without LLM
 */

function targetInfo(target) {
  if (!target) return "No target resolved from user input.";
  return `Path: ${target.path}\nType: ${target.type}${target.meta?.id ? `\nScenario: ${target.meta.id}` : ""}`;
}

/**
 * Build a brief list of available scenarios so the LLM can suggest matches.
 */
async function buildScenarioList() {
  try {
    const entries = await readdir(SCENARIOS_DIR, { withFileTypes: true });
    const scenarios = [];
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      try {
        const metaRaw = await readFile(
          join(SCENARIOS_DIR, entry.name, "metadata.json"),
          "utf8",
        );
        const meta = JSON.parse(metaRaw);
        scenarios.push(
          `- ${meta.id}: ${meta.description ?? meta.category} (${meta.mission_type ?? meta.category})`,
        );
      } catch {
        // skip scenarios without valid metadata
      }
    }
    return scenarios.length > 0
      ? scenarios.join("\n")
      : "(no scenarios available)";
  } catch {
    return "(scenarios directory not found)";
  }
}

/**
 * Build prior context string for multi-turn awareness.
 */
function buildPriorContext(priorAnalysis) {
  if (!priorAnalysis) return "(first turn — no prior context)";
  const parts = [];
  if (priorAnalysis.understood_as) {
    parts.push(`Previous understanding: ${priorAnalysis.understood_as}`);
  }
  if (priorAnalysis.action) {
    parts.push(`Previous action: ${priorAnalysis.action}`);
  }
  if (priorAnalysis.steps?.length) {
    parts.push(`Previous plan: ${priorAnalysis.steps.join(" → ")}`);
  }
  return parts.length > 0 ? parts.join("\n") : "(no prior context)";
}

function parseAnalysisResponse(raw) {
  try {
    const cleaned = raw
      .replace(/^```json?\n?/m, "")
      .replace(/\n?```$/m, "")
      .trim();
    const parsed = JSON.parse(cleaned);
    return {
      understood_as: parsed.understood_as ?? "Execute the requested task",
      action: parsed.action ?? "fix",
      steps: Array.isArray(parsed.steps) ? parsed.steps : [],
      requires_clarification: parsed.requires_clarification === true,
      clarification_question: parsed.clarification_question ?? null,
      inferred: false,
    };
  } catch {
    // Attempt partial JSON recovery — the model may have returned truncated
    // output due to token limits or timeout.
    return tryRecoverPartialJSON(raw);
  }
}

/**
 * Try to recover a usable analysis from truncated or malformed JSON.
 * Extracts key fields using regex when full JSON parsing fails.
 */
function tryRecoverPartialJSON(raw) {
  if (!raw || typeof raw !== "string") return null;

  const understood = raw.match(/"understood_as"\s*:\s*"([^"]+)"/)?.[1];
  const action = raw.match(/"action"\s*:\s*"([^"]+)"/)?.[1];
  const clarify = raw.match(/"requires_clarification"\s*:\s*(true|false)/)?.[1];

  if (understood && action) {
    return {
      understood_as: understood,
      action,
      steps: [],
      requires_clarification: clarify === "true",
      clarification_question: null,
      inferred: false,
    };
  }

  return null;
}

/**
 * Infer a safe default analysis when the LLM call fails.
 * No API call — purely deterministic based on target type and input keywords.
 */
function fallbackAnalysis(input, target) {
  const lower = (input ?? "").toLowerCase();

  // Detect action from target type or input keywords
  let action;
  if (target?.type === "dockerfile") {
    action = "docker_build";
  } else if (target?.type === "logfile") {
    action = "triage_only";
  } else if (
    target?.type === "new_directory" ||
    /\b(create|generate|set up|setup|write|scaffold|init)\b/i.test(input)
  ) {
    action = "generate";
  } else if (/\b(benchmark|run.*benchmark|hpl|linpack)\b/i.test(input)) {
    action = "benchmark_run";
  } else if (/\b(build\s+and\s+(start|run)|start.*container)\b/i.test(input)) {
    action = "build_start";
  } else if (/\b(build|docker\s+build)\b/i.test(input)) {
    action = "docker_build";
  } else {
    action = "fix";
  }

  const understood = target
    ? `Run DeCIpher on ${target.path} (${target.type})`
    : input.trim() || "Execute the requested task";

  // If the user input has a clear verb + subject, don't require clarification
  // even without a resolved target.
  const hasClearIntent =
    /\b(fix|build|run|create|generate|set up|deploy|start|benchmark|triage|analyze|debug|write|scaffold)\b/i.test(
      input,
    ) && input.trim().length > 15;

  return {
    understood_as: understood,
    action,
    steps: ["Inspect target", "Execute action", "Verify result"],
    requires_clarification: !target && !hasClearIntent,
    clarification_question:
      !target && !hasClearIntent
        ? "Which directory, Dockerfile, or log file should DeCIpher work on?"
        : null,
    inferred: true,
  };
}

/**
 * Use the LLM to understand what the user wants and create a plan.
 *
 * @param {string}      input          Raw user input
 * @param {object|null} target         Resolved target from resolveTarget()
 * @param {object}      config         API config
 * @param {object}      [options]      Optional settings
 * @param {object|null} [options.priorAnalysis] Previous analysis for multi-turn context
 * @param {number}      [options.timeout]       API timeout in ms (default 15000)
 * @returns {Promise<MissionAnalysis>}
 */
export async function analyzeMission(input, target, config, options = {}) {
  if (!config?.api_key) {
    return fallbackAnalysis(input, target);
  }

  let promptTemplate;
  try {
    promptTemplate = await readFile(join(PROMPTS_DIR, "analyze.md"), "utf8");
  } catch {
    return fallbackAnalysis(input, target);
  }

  const scenarioList = await buildScenarioList();
  const priorContext = buildPriorContext(options.priorAnalysis ?? null);

  const prompt = promptTemplate
    .replace("{user_input}", input.trim())
    .replace("{target_info}", targetInfo(target))
    .replace("{scenario_list}", scenarioList)
    .replace("{prior_context}", priorContext);

  const timeoutMs = options.timeout ?? 15000;

  try {
    const raw = await Promise.race([
      callAI(prompt, config),
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error("Mission analysis timed out")),
          timeoutMs,
        ),
      ),
    ]);
    const parsed = parseAnalysisResponse(raw);
    if (parsed) return parsed;
  } catch {
    // API error or timeout — degrade gracefully
  }

  return fallbackAnalysis(input, target);
}

export {
  parseAnalysisResponse,
  fallbackAnalysis,
  buildScenarioList,
  buildPriorContext,
  tryRecoverPartialJSON,
};
