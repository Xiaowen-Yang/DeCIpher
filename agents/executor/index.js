import { access, readFile } from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
import { homedir } from "node:os";
import pc from "picocolors";
import {
  loadSessionSnapshot,
  formatSessionSnapshot,
} from "../../lib/session-store.js";

// ── Target resolution ─────────────────────────────────────────────────────────

/**
 * Target types that can be auto-executed without asking the AI for clarification.
 * @typedef {'scenario'|'dockerfile'|'logfile'|'nodejs'} TargetType
 *
 * @typedef {{ path: string, type: TargetType, meta?: object }} ResolvedTarget
 */

function extractCandidatePaths(input) {
  const candidates = [];
  const punctuationBoundary = "[\\s\\\"'“”‘’`([{<，。,:;!?)]";
  const pathTerminator = "[^\\s\\\"'“”‘’`)>\\]}<，。,:;!?]+";

  // 1. Quoted paths (double or single)
  for (const m of input.matchAll(/"([^"]+)"|'([^']+)'/g)) {
    candidates.push(m[1] ?? m[2]);
  }

  // 2. Unquoted absolute or home-relative paths.
  // Accept punctuation boundaries so inputs like "修这个。/tmp/demo" still execute.
  const absolutePattern = new RegExp(
    `(?:^|${punctuationBoundary})((?:~|/)${pathTerminator})`,
    "g",
  );
  for (const m of input.matchAll(absolutePattern)) {
    candidates.push(m[1]);
  }

  // 3. Unquoted relative paths (./  ../  scenarios/  Dockerfile…)
  const relativePattern = new RegExp(
    `(?:^|${punctuationBoundary})((?:\\.\\.?/|scenarios/|Dockerfile)${pathTerminator})`,
    "g",
  );
  for (const m of input.matchAll(relativePattern)) {
    candidates.push(m[1]);
  }

  return [...new Set(candidates)];
}

function expandHome(p) {
  if (p.startsWith("~/")) return join(homedir(), p.slice(2));
  return p;
}

async function exists(p) {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

async function isDir(p) {
  const { stat } = await import("node:fs/promises");
  try {
    return (await stat(p)).isDirectory();
  } catch {
    return false;
  }
}

async function classifyPath(rawPath) {
  const p = resolve(expandHome(rawPath));
  if (!(await exists(p))) {
    // Path doesn't exist yet — treat as a new directory target so the agent
    // can create files there.  Return null only when the path looks like a
    // stray token (no slashes or only one segment).
    if (p.includes("/") && p.split("/").filter(Boolean).length >= 2) {
      return { path: p, type: "new_directory" };
    }
    return null;
  }

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
    // Existing directory without a known marker — still a valid target
    return { path: p, type: "directory" };
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
 * Extract a git repo URL from user input.
 * Supports github.com, gitlab.com, bitbucket.org — HTTPS and SSH formats.
 */
function extractGitUrl(input) {
  // HTTPS: https://github.com/user/repo or https://github.com/user/repo.git
  const httpsMatch = input.match(
    /https?:\/\/(github\.com|gitlab\.com|bitbucket\.org)\/[^\s"'<>]+/i,
  );
  if (httpsMatch) {
    const url = httpsMatch[0].replace(/\/+$/, "").replace(/\.git$/, "");
    const parts = url.split("/");
    const repoName = parts[parts.length - 1];
    return { url: httpsMatch[0], repoName, host: httpsMatch[1] };
  }

  // SSH: git@github.com:user/repo.git
  const sshMatch = input.match(
    /git@(github\.com|gitlab\.com|bitbucket\.org):([^\s"'<>]+)/i,
  );
  if (sshMatch) {
    const repoPath = sshMatch[2].replace(/\.git$/, "");
    const repoName = repoPath.split("/").pop();
    return { url: sshMatch[0], repoName, host: sshMatch[1] };
  }

  return null;
}

/**
 * Try to resolve an executable target from the user's input.
 * @param {string} input
 * @returns {Promise<ResolvedTarget|null>}
 */
export async function resolveTarget(input) {
  // Git repo URL takes priority over file paths
  const gitInfo = extractGitUrl(input);
  if (gitInfo) {
    return {
      path: gitInfo.url,
      type: "git_repo",
      meta: {
        url: gitInfo.url,
        repoName: gitInfo.repoName,
        host: gitInfo.host,
        start_state: "empty",
        mission_type: "clone_and_run",
      },
    };
  }

  for (const raw of extractCandidatePaths(input)) {
    const target = await classifyPath(raw);
    if (target) return target;
  }
  return null;
}

export { extractGitUrl };

// ── Action routing ────────────────────────────────────────────────────────────

/**
 * @typedef {'demo'|'triage_only'|'docker_build'|'fix'} Action
 */

export function resolveScenarioRuntime(meta = {}) {
  // Greenfield scenarios always go through the agent loop — no structural path
  if (meta.start_state === "empty" || meta.mission_type === "greenfield") {
    return {
      kind: "runtime",
      meta: {
        ...meta,
        execution_mode: "agent",
      },
    };
  }

  if (meta.execution_mode) {
    return {
      kind: "runtime",
      meta,
    };
  }

  if (meta.category === "docker") {
    return {
      kind: "runtime",
      meta: {
        ...meta,
        execution_mode: "docker_build",
        repair_target_files: meta.repair_target_files ??
          meta.broken_files ?? ["Dockerfile"],
      },
    };
  }

  return {
    kind: "structural",
    meta: {
      ...meta,
      repair_target_files: meta.repair_target_files ?? meta.broken_files ?? [],
    },
  };
}

// ── Approval model ────────────────────────────────────────────────────────────

const CAPABILITIES = [
  "read  — read scenario files, logs, and Dockerfiles",
  "fix   — propose and apply patches directly to your files",
  "run   — execute commands (docker build, grep, node, etc.)",
  "retry — attempt up to 3 fix iterations automatically",
  "write — create or modify files in the working directory",
];

/**
 * One-time session approval prompt.
 * Shows what the agent is about to do and asks for permission.
 *
 * @param {object} rl      readline.Interface open in the interactive session
 * @param {object} state   mutable session state { approved: boolean }
 * @param {object} [action] optional context: { tool, args, reasoning }
 * @returns {Promise<boolean>}
 */
export async function askApproval(rl, state, action = null) {
  if (
    state.approvalPolicy === "never" ||
    state.approvalPolicy === "on-failure"
  ) {
    state.approved = true;
    return true;
  }
  if (state.approved) return true;

  process.stdout.write("\n");
  console.log(pc.dim("  ┌─ APPROVAL ─────────────────────────────────────"));
  console.log(pc.dim("  │"));

  if (action) {
    console.log(
      `  ${pc.dim("│")} ${pc.bold("Action:")}  ${pc.cyan(action.tool)} ${action.args?.cmd ? pc.dim(String(action.args.cmd).slice(0, 60)) : action.args?.path ? pc.dim(action.args.path) : ""}`,
    );
    if (action.reasoning) {
      console.log(
        `  ${pc.dim("│")} ${pc.bold("Reason:")}  ${pc.dim(action.reasoning.slice(0, 80))}`,
      );
    }
    // Preview content for write_file
    if (action.tool === "write_file" && action.args?.content) {
      const preview = action.args.content
        .split("\n")
        .slice(0, 6)
        .map((l) => `  ${pc.dim("│")}   ${pc.dim(l)}`)
        .join("\n");
      console.log(`  ${pc.dim("│")} ${pc.bold("Preview:")}`);
      console.log(preview);
      if (action.args.content.split("\n").length > 6) {
        console.log(`  ${pc.dim("│")}   ${pc.dim("...")}`);
      }
    }
  } else {
    console.log(
      `  ${pc.dim("│")} DeCIpher needs permission to modify files and run commands.`,
    );
    CAPABILITIES.forEach((c) =>
      console.log(`  ${pc.dim("│")}   ${pc.yellow("›")} ${c}`),
    );
  }

  console.log(`  ${pc.dim("│")}`);
  console.log(
    pc.dim("  │  Scope: this session only. Nothing pushed or deployed."),
  );
  console.log(pc.dim("  └──────────────────────────────────────────────────"));

  let answer;
  try {
    answer = await new Promise((res, rej) => {
      if (rl.closed) return rej(new Error("readline closed"));
      rl.question(`  ${pc.bold("Allow?")} [Y/n] `, res);
    });
  } catch {
    // readline was closed (e.g. Ctrl-C during prompt) — treat as approved
    // so the mission can proceed in non-interactive / piped contexts.
    state.approved = true;
    return true;
  }

  const approved = answer.trim() === "" || /^y/i.test(answer.trim());
  state.approved = approved;

  if (approved) {
    console.log(`\n  ${pc.green("✓")} Approved.\n`);
  } else {
    console.log(`\n  ${pc.yellow("✗")} Denied.\n`);
  }

  return approved;
}

// ── Main executor dispatch ────────────────────────────────────────────────────

/**
 * Build a synthetic mission object when no session mission exists.
 * Maps the legacy action name to a natural-language goal so the agent
 * loop has meaningful context to work from.
 */
function buildSyntheticMission(action, target) {
  const path = target?.path ?? "";
  const typeLabel = target?.type ?? "target";
  const meta = target?.meta ?? {};

  // Greenfield scenarios carry a user prompt — use it as the mission goal
  if (
    meta.prompt &&
    (meta.start_state === "empty" || meta.mission_type === "greenfield")
  ) {
    return {
      goal: meta.prompt,
      type: meta.mission_type ?? "greenfield",
      id: meta.id ?? "greenfield-adhoc",
      stop_boundary: meta.mission_stop_boundary ?? "user_satisfied",
    };
  }

  // Git repo URL — clone into the current workspace and run in Docker
  if (target?.type === "git_repo") {
    const repoName = meta.repoName ?? "repo";
    return {
      goal: `Clone the repository ${path} into the current working directory, read the README to understand the project, then build and run it in Docker. The container must be running when you are done. Do NOT clone into /tmp — work in the current directory.`,
      type: "clone_and_run",
      id: `clone-${repoName}`,
      stop_boundary: "container_running",
    };
  }

  const goalMap = {
    fix: `Fix the failing ${typeLabel} at ${path}`,
    demo: `Demonstrate and fix the ${typeLabel} at ${path}`,
    docker_build: `Build the Docker image at ${path} successfully`,
    build_start: `Build and start the container at ${path}. The container must still be running when you are done.`,
    benchmark_run: `Build and run the benchmark at ${path} to completion in Docker. Create all needed files (Dockerfile, configs, scripts) if they do not exist.`,
    generate: `Generate the required files at ${path}. Create the directory if it does not exist.`,
    triage_only: `Triage and explain the failure at ${path}`,
  };
  return {
    goal: goalMap[action] ?? `Complete the ${action} task on ${path}`,
    type: action,
    id: `${action}-adhoc`,
  };
}

/**
 * Execute an identified target + action.
 *
 * Primary path: agent loop (LLM decides what tools to call to achieve the goal).
 * The agent understands the action as a goal, not a hardcoded execution mode.
 *
 * triage_only is the sole exception — it is a read-only analysis, not a loop.
 *
 * @param {ResolvedTarget} target
 * @param {Action} action
 * @param {object} config
 * @param {object} sessionState
 * @param {object} options
 */
export async function executeTarget(
  target,
  action,
  config,
  sessionState = {},
  options = {},
) {
  sessionState.currentTarget = target;

  // ── triage_only — standalone read-only analysis, not a loop ──────────────
  if (action === "triage_only") {
    const logPath =
      target.type === "logfile"
        ? target.path
        : target.meta?.log_file
          ? join(target.path, target.meta.log_file)
          : null;

    if (!logPath) {
      console.log(
        pc.yellow(
          "  Cannot determine log file for triage. Provide a .log path.\n",
        ),
      );
      return null;
    }

    const { triageLog } = await import("../triage/index.js");
    const { formatSection } = await import("../../lib/reporter.js");
    const { startSpinner } = await import("../../lib/spinner.js");

    console.log(pc.bold(`\n  Triaging: ${pc.cyan(logPath)}\n`));
    const sp = startSpinner("Triaging failure");
    const result = await triageLog(logPath, {}, config);
    sp.stop(
      `${pc.yellow(result.classification)} (confidence: ${result.confidence})`,
    );
    console.log(
      formatSection(
        "CLASSIFICATION",
        `  label:      ${pc.yellow(result.classification)}\n  confidence: ${result.confidence}`,
      ),
    );
    if (result.root_causes?.length) {
      console.log(
        formatSection(
          "EVIDENCE",
          result.root_causes
            .map((rc) => `  ${rc.hypothesis}: ${rc.evidence}`)
            .join("\n"),
        ),
      );
    }
    console.log("");
    sessionState.lastRunResult = result;
    sessionState.lastVerificationResult = null;
    return result;
  }

  // ── All other actions → agent loop ────────────────────────────────────────
  const mission =
    sessionState.currentMission ?? buildSyntheticMission(action, target);

  console.log(
    pc.bold(
      `\n  Agent starting on: ${pc.cyan(target.path ?? "(no target)")}\n`,
    ),
  );
  console.log(pc.dim(`  Goal: ${mission.goal}`));

  const { runAgentLoop, printAgentLoopResult } =
    await import("./agent-loop.js");

  const result = await runAgentLoop(mission, target, config, {
    ...options,
    sessionContext: sessionState,
    rl: options.rl ?? sessionState.rl ?? null,
  });

  sessionState.lastRunResult = result;
  sessionState.lastVerificationResult = result.state;
  printAgentLoopResult(result);

  // ── Acceptance checks for greenfield scenarios ────────────────────────────
  const meta = target?.meta ?? {};
  if (meta.acceptance && result.workspace) {
    try {
      const {
        loadAcceptanceChecks,
        runAcceptanceChecks,
        printAcceptanceSummary,
      } = await import("../verifier/acceptance.js");

      console.log(pc.bold("\n  [ACCEPTANCE CHECKS]\n"));
      const checks = await loadAcceptanceChecks(target.path);
      const report = await runAcceptanceChecks(checks, result.workspace);
      printAcceptanceSummary(report);

      // Override the result outcome based on acceptance
      if (report.passed && result.outcome === "PASS") {
        result.acceptancePassed = true;
      } else if (!report.passed) {
        result.outcome = "FAIL";
        result.state = "ACCEPTANCE_FAIL";
        result.acceptancePassed = false;
      }
    } catch (err) {
      console.log(pc.yellow(`  Acceptance check error: ${err.message}\n`));
    }
  }

  return result;
}

export async function executeScenarioPath(
  scenarioPath,
  config,
  sessionState = {},
  options = {},
) {
  const target = await resolvePathTarget(scenarioPath);
  if (!target || target.type !== "scenario") {
    throw new Error(`Scenario not found or invalid: ${scenarioPath}`);
  }

  const meta = target.meta ?? {};

  // Greenfield scenarios use the scenario's prompt as the action
  if (meta.start_state === "empty" || meta.mission_type === "greenfield") {
    return executeTarget(target, "generate", config, sessionState, options);
  }

  return executeTarget(target, "fix", config, sessionState, options);
}

export function decideResumeAction(snapshot = {}) {
  if (!snapshot?.resumable) {
    return { mode: "not_resumable" };
  }

  if (
    snapshot.stop_reason === "needs_clarification" ||
    snapshot.plan?.requires_clarification
  ) {
    return {
      mode: "clarify",
      question:
        snapshot.plan?.clarification_question ??
        "What do you want DeCIpher to do exactly?",
    };
  }

  if (snapshot.target_path) {
    return {
      mode: "execute_target",
      targetPath: snapshot.target_path,
    };
  }

  return { mode: "not_resumable" };
}

export async function resumeLastTarget(config, sessionState = {}) {
  const snapshot = await loadSessionSnapshot();
  if (!snapshot) {
    console.log(pc.yellow("  No saved executor session to resume.\n"));
    return null;
  }

  const resumeAction = decideResumeAction(snapshot);
  if (resumeAction.mode === "not_resumable") {
    console.log(pc.yellow("  The last saved session is not resumable.\n"));
    console.log(`  ${formatSessionSnapshot(snapshot, { public: true })}\n`);
    return null;
  }

  sessionState.currentMission =
    snapshot.mission ?? sessionState.currentMission ?? null;
  sessionState.currentPlan = snapshot.plan ?? sessionState.currentPlan ?? null;
  sessionState.lastVerificationResult =
    snapshot.last_verification_state ??
    sessionState.lastVerificationResult ??
    null;
  sessionState.approved = snapshot.approved ?? sessionState.approved ?? false;

  if (resumeAction.mode === "clarify") {
    console.log(pc.bold("\n  Resuming mission clarification\n"));
    if (snapshot.mission_summary) {
      console.log(pc.dim(`  mission: ${snapshot.mission_summary}`));
    }
    console.log(
      pc.dim(
        `  previous state: ${snapshot.last_verification_state ?? "needs clarification"}`,
      ),
    );
    console.log(pc.yellow(`  question: ${resumeAction.question}\n`));
    return {
      needs_clarification: resumeAction.question,
      resumed: true,
      mode: "clarify",
      snapshot,
    };
  }

  const target = await resolvePathTarget(snapshot.target_path);
  if (!target) {
    throw new Error(
      `Saved target is no longer available: ${snapshot.target_path}`,
    );
  }

  sessionState.currentTarget = target;

  console.log(pc.bold("\n  Resuming executor session\n"));
  console.log(pc.dim(`  target: ${snapshot.target_path}`));
  console.log(
    pc.dim(
      `  previous state: ${snapshot.last_verification_state ?? "unknown"}`,
    ),
  );
  console.log(
    pc.dim(
      `  iteration: ${snapshot.iteration ?? 0}/${snapshot.max_iterations ?? 3}\n`,
    ),
  );

  return executeTarget(target, "fix", config, sessionState, {
    resumeFrom: snapshot,
  });
}
