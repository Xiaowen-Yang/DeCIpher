import { access, readFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { homedir } from "node:os";
import pc from "picocolors";
import { loadSessionSnapshot, formatSessionSnapshot } from "../../lib/session-store.js";

// ── Target resolution ─────────────────────────────────────────────────────────

/**
 * Target types that can be auto-executed without asking the AI for clarification.
 * @typedef {'scenario'|'dockerfile'|'logfile'|'nodejs'} TargetType
 *
 * @typedef {{ path: string, type: TargetType, meta?: object }} ResolvedTarget
 */

function extractCandidatePaths(input) {
  const candidates = [];

  // 1. Quoted paths (double or single)
  for (const m of input.matchAll(/"([^"]+)"|'([^']+)'/g)) {
    candidates.push(m[1] ?? m[2]);
  }

  // 2. Unquoted absolute or home-relative paths
  for (const m of input.matchAll(/(?:^|\s)((?:~|\/)[^\s]+)/g)) {
    candidates.push(m[1]);
  }

  // 3. Unquoted relative paths (./  ../  scenarios/  Dockerfile…)
  for (const m of input.matchAll(/(?:^|\s)((?:\.\.?\/|scenarios\/|Dockerfile)[^\s]*)/g)) {
    candidates.push(m[1]);
  }

  return [...new Set(candidates)];
}

function expandHome(p) {
  if (p.startsWith("~/")) return join(homedir(), p.slice(2));
  return p;
}

async function exists(p) {
  try { await access(p); return true; } catch { return false; }
}

async function isDir(p) {
  const { stat } = await import("node:fs/promises");
  try { return (await stat(p)).isDirectory(); } catch { return false; }
}

async function classifyPath(rawPath) {
  const p = resolve(expandHome(rawPath));
  if (!(await exists(p))) return null;

  if (await isDir(p)) {
    if (await exists(join(p, "metadata.json"))) {
      const meta = JSON.parse(await readFile(join(p, "metadata.json"), "utf8"));
      return { path: p, type: "scenario", meta };
    }
    if (await exists(join(p, "Dockerfile"))) {
      return { path: p, type: "dockerfile" };
    }
    if (await exists(join(p, "package.json"))) {
      return { path: p, type: "nodejs" };
    }
    return null;
  }

  const name = p.split("/").pop();
  if (name === "Dockerfile" || name.startsWith("Dockerfile.")) {
    return { path: p, type: "dockerfile" };
  }
  if (name.endsWith(".log") || name.endsWith(".txt")) {
    return { path: p, type: "logfile" };
  }

  return null;
}

export async function resolvePathTarget(rawPath) {
  return classifyPath(rawPath);
}

/**
 * Try to resolve an executable target from the user's input.
 * @param {string} input
 * @returns {Promise<ResolvedTarget|null>}
 */
export async function resolveTarget(input) {
  for (const raw of extractCandidatePaths(input)) {
    const target = await classifyPath(raw);
    if (target) return target;
  }
  return null;
}

// ── Action detection ──────────────────────────────────────────────────────────

/**
 * @typedef {'demo'|'triage_only'|'docker_build'|'fix'} Action
 */

const ACTION_KEYWORDS = {
  triage_only: /\b(triage|classify|analyze|what.s wrong|diagnose)\b/i,
  docker_build: /\b(build|docker build|rebuild|image)\b/i,
  demo:         /\b(demo|show|run scenario|walk.?through)\b/i,
};

/**
 * Detect the intended action from natural language input.
 * Falls back to 'fix' (full executor loop) when ambiguous.
 * @param {string} input
 * @param {TargetType} targetType
 * @returns {Action}
 */
export function detectAction(input, targetType) {
  if (ACTION_KEYWORDS.triage_only.test(input)) return "triage_only";
  if (ACTION_KEYWORDS.demo.test(input))        return "demo";
  if (targetType === "dockerfile") {
    return "docker_build";
  }
  if (ACTION_KEYWORDS.docker_build.test(input) && targetType !== "scenario") {
    return "docker_build";
  }
  return "fix"; // default: unified executor loop (triage → fix → verify → writeback)
}

// ── Approval model ────────────────────────────────────────────────────────────

const CAPABILITIES = [
  "read  — read scenario files, logs, and Dockerfiles",
  "fix   — propose and apply minimal patches to a temp workspace",
  "run   — execute verification commands (docker build, grep, node)",
  "retry — attempt up to 3 fix iterations automatically",
  "write — write repaired files back to broken/ on success",
];

export function shouldConfirmWriteback(state = {}) {
  return (state.approvalPolicy ?? "on-request") !== "never";
}

/**
 * One-time session approval prompt (Codex-style).
 * @param {object} rl    readline.Interface open in the interactive session
 * @param {object} state mutable session state { approved: boolean }
 * @returns {Promise<boolean>}
 */
export async function askApproval(rl, state) {
  if (state.approvalPolicy === "never" || state.approvalPolicy === "on-failure") {
    state.approved = true;
    return true;
  }
  if (state.approved) return true;

  process.stdout.write("\n");
  console.log(pc.bold(pc.cyan("  DeCIpher executor — one-time session approval")));
  console.log(pc.dim("  " + "─".repeat(50)));
  console.log(pc.dim("  Granting approval authorises this session to:"));
  CAPABILITIES.forEach(c => console.log(`    ${pc.yellow("›")} ${c}`));
  console.log(pc.dim("\n  Scope: current session only. No files pushed or deployed."));
  console.log(pc.dim("  Destructive changes (editing original files) still ask per-action.\n"));

  const answer = await new Promise(res =>
    rl.question(`  ${pc.bold("Allow?")} [Y/n] `, res),
  );

  const approved = answer.trim() === "" || /^y/i.test(answer.trim());
  state.approved = approved;

  if (approved) {
    console.log(`\n  ${pc.green("✓")} Approved for this session.\n`);
  } else {
    console.log(`\n  ${pc.yellow("✗")} Approval denied — conversational mode only.\n`);
  }

  return approved;
}

export async function confirmWriteback(rl, state, files = []) {
  if (!shouldConfirmWriteback(state)) {
    return true;
  }

  process.stdout.write("\n");
  console.log(pc.bold(pc.yellow("  Confirm write-back")));
  console.log(pc.dim("  The repaired temp workspace is ready to write back to broken/."));
  if (files.length > 0) {
    console.log(pc.dim(`  Files: ${files.join(", ")}`));
  }

  const answer = await new Promise((res) =>
    rl.question(`  ${pc.bold("Write repaired files back?")} [y/N] `, res),
  );
  const confirmed = /^y/i.test(answer.trim());

  if (!confirmed) {
    console.log(`\n  ${pc.yellow("✗")} Write-back declined. Preserving temp workspace only.\n`);
  }

  return confirmed;
}

// ── Main executor dispatch ────────────────────────────────────────────────────

/**
 * Execute an identified target + action using the unified executor loop.
 *
 * For scenarios with execution_mode metadata (docker_build / docker_run /
 * healthcheck), the loop runs the real command, captures failure, triages,
 * patches the temp workspace, and writes repaired files back on success.
 *
 * Falls back to the orchestrator's runScenario for legacy scenarios without
 * execution_mode.
 *
 * @param {ResolvedTarget} target
 * @param {Action} action
 * @param {object} config
 * @param {object} sessionState
 * @param {object} options
 */
export async function executeTarget(target, action, config, sessionState = {}, options = {}) {
  sessionState.currentTarget = target;
  switch (action) {
    case "demo":
    case "fix": {
      if (target.type !== "scenario") {
        console.log(pc.yellow(
          `  Target type '${target.type}' does not support the '${action}' action.\n`,
        ));
        return;
      }

      console.log(pc.bold(`\n  Executor loop starting on: ${pc.cyan(target.path)}\n`));

      const meta = target.meta;

      // Scenarios with execution_mode use the unified loop (real commands + writeback)
      if (meta?.execution_mode) {
        const { runExecutorLoop, printLoopResult } = await import("./loop.js");
        const result = await runExecutorLoop(target.path, meta, config, {
          resumeFrom: options.resumeFrom,
          sessionContext: sessionState,
        });
        sessionState.lastRunResult = result;
        sessionState.lastVerificationResult = result.state;
        printLoopResult(result);
        return result;
      } else {
        // Legacy scenarios: use orchestrator (pre-captured logs, no writeback)
        const { runScenario } = await import("../orchestrator/index.js");
        const { formatReport } = await import("../../lib/reporter.js");
        const report = await runScenario(target.path, config);
        console.log(formatReport(report));
        sessionState.lastRunResult = report;
        sessionState.lastVerificationResult = report?.verification?.result ?? null;
        return report;
      }
    }

    case "triage_only": {
      const logPath = target.type === "logfile"
        ? target.path
        : target.meta?.log_file
          ? join(target.path, target.meta.log_file)
          : null;

      if (!logPath) {
        console.log(pc.yellow("  Cannot determine log file for triage. Provide a .log path.\n"));
        return null;
      }

      const { triageLog } = await import("../triage/index.js");
      const { formatSection } = await import("../../lib/reporter.js");
      const { startSpinner } = await import("../../lib/spinner.js");

      console.log(pc.bold(`\n  Triaging: ${pc.cyan(logPath)}\n`));
      const sp = startSpinner("Triaging failure");
      const result = await triageLog(logPath, {}, config);
      sp.stop(`${pc.yellow(result.classification)} (confidence: ${result.confidence})`);

      console.log(formatSection("CLASSIFICATION",
        `  label:      ${pc.yellow(result.classification)}\n  confidence: ${result.confidence}`,
      ));
      if (result.root_causes?.length) {
        console.log(formatSection("EVIDENCE",
          result.root_causes.map(rc => `  ${rc.hypothesis}: ${rc.evidence}`).join("\n"),
        ));
      }
      console.log("");
      sessionState.lastRunResult = result;
      sessionState.lastVerificationResult = null;
      return result;
    }

    case "docker_build": {
      // Dockerfile dir without a scenario wrapper — create synthetic meta
      const dir = await isDir(target.path)
        ? target.path
        : target.path.replace(/\/Dockerfile.*$/, "");

      const syntheticMeta = {
        id:                  "docker-adhoc",
        category:            "docker",
        execution_mode:      "docker_build",
        repair_target_files: ["Dockerfile"],
        writeback_on_success: false, // ad-hoc: don't overwrite originals
        log_file:            null,
      };

      console.log(pc.bold(`\n  Executor loop starting on: ${pc.cyan(dir)}\n`));
      const { runExecutorLoop, printLoopResult } = await import("./loop.js");
      const result = await runExecutorLoop(dir, syntheticMeta, config, {
        resumeFrom: options.resumeFrom,
        sessionContext: sessionState,
      });
      // For ad-hoc docker dir the "brokenDir" is the workspace's parent — override
      result.writtenBack = []; // no write-back for ad-hoc targets
      sessionState.lastRunResult = result;
      sessionState.lastVerificationResult = result.state;
      printLoopResult(result);
      return result;
    }

    default:
      console.log(pc.yellow(`  Unknown action '${action}'.\n`));
      return null;
  }
}

export async function executeScenarioPath(scenarioPath, config, sessionState = {}, options = {}) {
  const target = await resolvePathTarget(scenarioPath);
  if (!target || target.type !== "scenario") {
    throw new Error(`Scenario not found or invalid: ${scenarioPath}`);
  }
  return executeTarget(target, "fix", config, sessionState, options);
}

export async function resumeLastTarget(config, sessionState = {}) {
  const snapshot = await loadSessionSnapshot();
  if (!snapshot) {
    console.log(pc.yellow("  No saved executor session to resume.\n"));
    return null;
  }

  if (!snapshot.resumable || !snapshot.target_path) {
    console.log(pc.yellow("  The last saved session is not resumable.\n"));
    console.log(`  ${formatSessionSnapshot(snapshot, { public: true })}\n`);
    return null;
  }

  const target = await resolvePathTarget(snapshot.target_path);
  if (!target) {
    throw new Error(`Saved target is no longer available: ${snapshot.target_path}`);
  }

  console.log(pc.bold("\n  Resuming executor session\n"));
  console.log(pc.dim(`  target: ${snapshot.target_path}`));
  console.log(pc.dim(`  previous state: ${snapshot.last_verification_state ?? "unknown"}`));
  console.log(pc.dim(`  iteration: ${snapshot.iteration ?? 0}/${snapshot.max_iterations ?? 3}\n`));

  return executeTarget(target, "fix", config, sessionState, { resumeFrom: snapshot });
}
