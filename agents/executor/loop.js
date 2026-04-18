/**
 * Unified execution loop — the heart of the executor.
 *
 * Replaces both runScenario (orchestrator) and runDockerBuildLoop (executor)
 * with a single state machine:
 *
 *   createWorkspace → runCommand → PASS? → writeBack → done
 *                          ↓ FAIL
 *                   triage → proposeFix → applyPatch → runCommand (retry)
 *
 * Supports execution_mode: "docker_build" | "docker_run" | "healthcheck"
 * Reads supplemental pre-captured logs when real command cannot be run.
 * Writes back only the repair_target_files on success.
 */

import { join } from "node:path";
import { readFile } from "node:fs/promises";
import pc from "picocolors";
import { startSpinner } from "../../lib/spinner.js";
import { createWorkspace, writeBack, readWorkspaceFiles, cleanupWorkspace } from "./workspace.js";
import { runCommand } from "./runner.js";
import { persistSessionSnapshot } from "../../lib/session-store.js";
import { runCompletionNotification } from "../../lib/notifications.js";

const MAX_ITERATIONS = 3;

/**
 * @typedef {object} LoopResult
 * @property {'PASS'|'FAIL'|'NEEDS_HUMAN_REVIEW'} outcome
 * @property {string} state            — final verification state
 * @property {string[]} writtenBack    — files written back to broken/
 * @property {string|null} workspace   — preserved workspace path on failure (for debugging)
 * @property {number} iterations       — number of attempts made
 * @property {object} classification   — from triage node
 * @property {string|null} patch       — last patch proposed
 * @property {string} transcript       — human-readable execution log
 */

/**
 * Run the unified executor loop for a scenario.
 *
 * @param {string} scenarioPath   Absolute path to scenario directory
 * @param {object} meta           Parsed metadata.json
 * @param {object} config         API config (provider, model, api_key, …)
 * @param {object} options
 * @returns {Promise<LoopResult>}
 */
export async function runExecutorLoop(scenarioPath, meta, config, options = {}) {
  const { triageLog } = await import("../triage/index.js");
  const { proposeFix } = await import("../fixer/index.js");
  const { applyPatch } = await import("../verifier/index.js");

  const brokenDir     = join(scenarioPath, "broken");
  const executionMode = meta.execution_mode ?? "docker_build";
  const repairFiles   = meta.repair_target_files ?? meta.broken_files ?? ["Dockerfile"];
  const writeback     = meta.writeback_on_success !== false;
  const supplementLog = meta.log_file ? join(scenarioPath, meta.log_file) : null;

  const lines = []; // transcript accumulator
  const log = (msg) => { lines.push(msg); };
  const resumeFrom = options.resumeFrom ?? null;
  const approvalPolicy = options.sessionContext?.approvalPolicy ?? config.approval_policy ?? "on-request";
  const approved = options.sessionContext?.approved ?? false;

  log(pc.bold(`\n  [executor] scenario: ${meta.id}`));
  log(pc.dim(`  mode: ${executionMode}  max_retries: ${MAX_ITERATIONS}`));

  async function saveSnapshot(extra = {}) {
    const snapshot = {
      target_path: scenarioPath,
      target_type: "scenario",
      scenario_id: meta.id,
      execution_mode: executionMode,
      repair_target_files: repairFiles,
      approval_policy: approvalPolicy,
      approved,
      max_iterations: MAX_ITERATIONS,
      transcript: lines.join("\n"),
      ...extra,
    };
    await persistSessionSnapshot(snapshot);
    return snapshot;
  }

  // ── Workspace setup ───────────────────────────────────────
  const sp = startSpinner("Creating workspace");
  let workspace;
  let startIteration = 1;
  try {
    if (resumeFrom?.workspace_path) {
      workspace = resumeFrom.workspace_path;
      startIteration = Math.min((resumeFrom.iteration ?? 0) + 1, MAX_ITERATIONS);
      sp.stop(`Resuming workspace → ${pc.dim(workspace)}`);
      log(`  resumed workspace: ${workspace}`);
    } else {
      workspace = await createWorkspace(brokenDir, meta.id);
      sp.stop(`Workspace ready → ${pc.dim(workspace)}`);
      log(`  workspace: ${workspace}`);
    }
    await saveSnapshot({
      iteration: startIteration - 1,
      workspace_path: workspace,
      last_verification_state: null,
      resumable: true,
      outcome: "RUNNING",
    });
  } catch (err) {
    sp.stop();
    log(pc.red(`  [executor] workspace creation failed: ${err.message}`));
    await saveSnapshot({
      iteration: 0,
      workspace_path: null,
      last_verification_state: "BUILD_FAIL",
      resumable: false,
      outcome: "FAIL",
      error: err.message,
    });
    await notify("BUILD_FAIL", "workspace_creation_failed", null);
    return {
      outcome: "FAIL", state: "BUILD_FAIL", writtenBack: [],
      workspace: null, iterations: 0,
      classification: {}, patch: null,
      transcript: lines.join("\n"),
    };
  }

  let lastPatch        = null;
  let classification   = null;
  let finalState       = "FAIL";
  let preserveWorkspace = false;
  let completedIterations = startIteration - 1;
  const notify = async (status, stopReason = null, workspacePath = null) => {
    return runCompletionNotification(config.notification_command, {
      status,
      targetPath: scenarioPath,
      workspacePath,
      stopReason,
    });
  };

  try {
    for (let iteration = startIteration; iteration <= MAX_ITERATIONS; iteration++) {
      completedIterations = iteration;
      log(pc.bold(`\n  ── Iteration ${iteration}/${MAX_ITERATIONS} ──`));

      // ── Run real command ────────────────────────────────
      const runSp = startSpinner(`Running ${executionMode}`);
      const result = await runCommand(executionMode, workspace, meta.id);
      runSp.stop(`Command finished → ${stateLabel(result.state)}`);
      log(`  command result: ${result.state}`);
      await saveSnapshot({
        iteration,
        workspace_path: workspace,
        last_verification_state: result.state,
        last_output_excerpt: result.output.split("\n").slice(-20).join("\n"),
        resumable: result.state !== "PASS",
        outcome: result.state === "PASS" ? "PASS" : "RUNNING",
      });

      if (result.state === "PASS") {
        finalState = "PASS";
        break;
      }

      // ── Collect evidence ────────────────────────────────
      // Real command output is primary; pre-captured log is supplemental.
      let evidence = result.output;
      if (supplementLog) {
        try {
          const captured = await readFile(supplementLog, "utf8");
          evidence = evidence
            ? `${evidence}\n\n=== supplemental log ===\n${captured}`
            : captured;
        } catch { /* log file may not exist */ }
      }

      log(pc.dim(`\n  failure output (last 20 lines):`));
      evidence.split("\n").slice(-20).forEach(l => log(pc.dim(`    ${l}`)));

      // ── Triage ──────────────────────────────────────────
      const triageSp = startSpinner("Triaging failure");

      // Write evidence to a temp file for triageLog (it expects a file path)
      const { writeFile } = await import("node:fs/promises");
      const evidencePath = join(workspace, ".decipher-evidence.log");
      await writeFile(evidencePath, evidence, "utf8");

      classification = await triageLog(evidencePath, { category: meta.category }, config);
      triageSp.stop(
        `Triage → ${pc.yellow(classification.classification)} (confidence: ${classification.confidence})`,
      );
      log(`  classification: ${classification.classification} (${classification.confidence})`);

      if (classification.confidence < 0.6 || classification.needs_more_evidence) {
        log(pc.yellow("  confidence too low — stopping for human review"));
        preserveWorkspace = true;
        const sessionSnapshot = await saveSnapshot({
          iteration,
          workspace_path: workspace,
          last_verification_state: result.state,
          classification,
          patch: null,
          resumable: true,
          outcome: "NEEDS_HUMAN_REVIEW",
        });
        await notify(result.state, "low_confidence", workspace);
        return {
          outcome: "NEEDS_HUMAN_REVIEW", state: result.state, writtenBack: [],
          workspace: preserveWorkspace ? workspace : null,
          iterations: iteration, classification, patch: null,
          sessionSnapshot,
          transcript: lines.join("\n"),
        };
      }

      // ── Propose fix ─────────────────────────────────────
      const fixSp = startSpinner("Proposing fix");
      const brokenFiles = await readWorkspaceFiles(workspace, repairFiles);
      const patchArtifact = await proposeFix(classification, { broken_files: brokenFiles }, config);
      fixSp.stop(`Fix proposed → affects: ${patchArtifact.affected_files.join(", ")}`);
      log(`  patch affects: ${patchArtifact.affected_files.join(", ")}`);

      // Stop condition: same patch twice
      const patchFingerprint = patchArtifact.patch ?? "";
      if (patchFingerprint && patchFingerprint === lastPatch) {
        log(pc.yellow("  same patch proposed twice — stopping"));
        preserveWorkspace = true;
        const sessionSnapshot = await saveSnapshot({
          iteration,
          workspace_path: workspace,
          last_verification_state: result.state,
          classification,
          patch: lastPatch,
          resumable: true,
          outcome: "NEEDS_HUMAN_REVIEW",
        });
        await notify(result.state, "same_patch_repeated", workspace);
        return {
          outcome: "NEEDS_HUMAN_REVIEW", state: result.state, writtenBack: [],
          workspace: preserveWorkspace ? workspace : null,
          iterations: iteration, classification, patch: lastPatch,
          sessionSnapshot,
          transcript: lines.join("\n"),
        };
      }
      lastPatch = patchFingerprint;

      // Stop condition: patch scope too large
      if (patchArtifact.affected_files.length > 3) {
        log(pc.yellow("  patch touches >3 files — stopping for human review"));
        preserveWorkspace = true;
        const sessionSnapshot = await saveSnapshot({
          iteration,
          workspace_path: workspace,
          last_verification_state: result.state,
          classification,
          patch: lastPatch,
          resumable: true,
          outcome: "NEEDS_HUMAN_REVIEW",
        });
        await notify(result.state, "patch_scope_too_large", workspace);
        return {
          outcome: "NEEDS_HUMAN_REVIEW", state: result.state, writtenBack: [],
          workspace: preserveWorkspace ? workspace : null,
          iterations: iteration, classification, patch: lastPatch,
          sessionSnapshot,
          transcript: lines.join("\n"),
        };
      }

      // ── Apply patch ─────────────────────────────────────
      if (patchArtifact.patch) {
        for (const rel of repairFiles) {
          try {
            await applyPatch(patchArtifact.patch, join(workspace, rel));
            log(pc.dim(`  patch applied to: ${rel}`));
          } catch (err) {
            log(pc.dim(`  patch apply note for ${rel}: ${err.message}`));
          }
        }
      } else {
        log(pc.yellow("  no patch produced — stopping"));
        break;
      }
    }

    // ── Write back on success ───────────────────────────────
    let writtenBack = [];
    if (finalState === "PASS" && writeback) {
      const writebackAllowed = options.sessionContext?.confirmWriteback
        ? await options.sessionContext.confirmWriteback(repairFiles)
        : true;

      if (!writebackAllowed) {
        preserveWorkspace = true;
        const sessionSnapshot = await saveSnapshot({
          iteration: completedIterations,
          workspace_path: workspace,
          last_verification_state: finalState,
          classification,
          patch: lastPatch,
          written_back: [],
          resumable: true,
          outcome: "NEEDS_HUMAN_REVIEW",
          stop_reason: "writeback_declined",
        });
        await notify(finalState, "writeback_declined", workspace);
        return {
          outcome: "NEEDS_HUMAN_REVIEW",
          state: finalState,
          writtenBack: [],
          workspace,
          iterations: completedIterations,
          classification: classification ?? {},
          patch: lastPatch,
          sessionSnapshot,
          transcript: lines.join("\n"),
        };
      }

      const wbSp = startSpinner("Writing back repaired files");
      writtenBack = await writeBack(workspace, brokenDir, repairFiles);
      wbSp.stop(`Write-back complete: ${writtenBack.join(", ")}`);
      log(`  written back: ${writtenBack.join(", ")}`);
    }

    const sessionSnapshot = await saveSnapshot({
      iteration: completedIterations,
      workspace_path: finalState === "PASS" ? null : workspace,
      last_verification_state: finalState,
      classification,
      patch: lastPatch,
      written_back: writtenBack,
      resumable: finalState !== "PASS",
      outcome: finalState === "PASS" ? "PASS" : "FAIL",
    });
    await notify(finalState, finalState === "PASS" ? null : "loop_complete", finalState === "PASS" ? null : workspace);

    return {
      outcome: finalState === "PASS" ? "PASS" : "FAIL",
      state: finalState,
      writtenBack,
      workspace: finalState !== "PASS" ? workspace : null,
      iterations: completedIterations,
      classification: classification ?? {},
      patch: lastPatch,
      sessionSnapshot,
      transcript: lines.join("\n"),
    };

  } finally {
    // On failure preserve workspace; on success clean it up
    if (finalState === "PASS" && !preserveWorkspace) {
      await cleanupWorkspace(workspace, false);
    } else {
      // Leave workspace intact for debugging; already recorded in result
    }
  }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

function stateLabel(state) {
  switch (state) {
    case "PASS":             return pc.green("PASS");
    case "BUILD_FAIL":       return pc.red("BUILD_FAIL");
    case "RUN_FAIL":         return pc.red("RUN_FAIL");
    case "HEALTHCHECK_FAIL": return pc.red("HEALTHCHECK_FAIL");
    default:                 return pc.red(state);
  }
}

/**
 * Print a human-readable summary of a LoopResult to stdout.
 * Output reads like an execution transcript, not a chat reply.
 * @param {LoopResult} result
 */
export function printLoopResult(result) {
  const divider = pc.dim("─".repeat(60));
  console.log(`\n${divider}`);

  console.log(pc.bold("\n  [RESULT]"));
  const outcomeLabel = result.outcome === "PASS"
    ? pc.bold(pc.green("PASS"))
    : result.outcome === "NEEDS_HUMAN_REVIEW"
      ? pc.bold(pc.yellow("NEEDS HUMAN REVIEW"))
      : pc.bold(pc.red("FAIL"));
  console.log(`  Outcome:       ${outcomeLabel}`);
  console.log(`  Final state:   ${result.state}`);
  console.log(`  Iterations:    ${result.iterations}`);

  if (result.classification?.classification) {
    console.log(`  Classification: ${pc.yellow(result.classification.classification)} (${result.classification.confidence})`);
  }

  if (result.patch) {
    const patchLines = result.patch.split("\n").slice(0, 8).join("\n");
    console.log(pc.bold("\n  [PATCH APPLIED]"));
    patchLines.split("\n").forEach(l => {
      if (l.startsWith("+") && !l.startsWith("+++")) process.stdout.write(`  ${pc.green(l)}\n`);
      else if (l.startsWith("-") && !l.startsWith("---")) process.stdout.write(`  ${pc.red(l)}\n`);
      else process.stdout.write(`  ${pc.dim(l)}\n`);
    });
  }

  if (result.writtenBack?.length > 0) {
    console.log(pc.bold("\n  [WRITTEN BACK]"));
    result.writtenBack.forEach(f => console.log(`  ${pc.green("✓")} ${f}`));
  }

  if (result.workspace) {
    console.log(pc.bold("\n  [DEBUG ARTIFACTS]"));
    console.log(`  Workspace preserved for inspection:`);
    console.log(`  ${pc.cyan(result.workspace)}`);
  }

  console.log(`\n${divider}\n`);
}
