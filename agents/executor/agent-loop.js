/**
 * Agent-driven execution loop — the Codex-style runtime core.
 *
 * The LLM agent receives: mission goal + workspace + available tools + history.
 * It responds with a JSON tool call. The loop executes the tool, feeds the
 * result back, and repeats until the agent calls `done` or limits are hit.
 *
 * This replaces the fixed-mode runner (docker_build / docker_run / etc.) as
 * the primary execution engine. Docker runners and the repair subsystem remain
 * available as utilities the agent can invoke via exec_command and the shell.
 *
 * LoopResult shape is compatible with the legacy runExecutorLoop result so
 * callers (index.js, printLoopResult) work without changes.
 */

import { join } from "node:path";
import pc from "picocolors";
import { startSpinner } from "../../lib/spinner.js";
import { createEmptyWorkspace } from "./workspace.js";
import {
  persistSessionSnapshot,
  persistTranscript,
  recordPreservedWorkspace,
  compactSessionSnapshot,
} from "../../lib/session-store.js";
import { runCompletionNotification } from "../../lib/notifications.js";
import { TOOL_REGISTRY, toolsPromptSection, isToolRisky } from "./tools.js";
import { callAIWithMessages } from "../../lib/api-client.js";
import { readFile } from "node:fs/promises";

const MAX_TURNS = 20;
const AGENT_PROMPT_PATH = new URL("../../prompts/agent.md", import.meta.url)
  .pathname;

// ── System prompt builder ─────────────────────────────────────────────────────

function detectHostEnvironment() {
  const os = process.platform; // darwin, linux, win32
  const arch = process.arch; // arm64, x64
  const osLabel =
    os === "darwin"
      ? "macOS"
      : os === "win32"
        ? "Windows"
        : os === "linux"
          ? "Linux"
          : os;

  const lines = [`Host OS: ${osLabel} (${arch})`];

  if (os === "darwin") {
    lines.push(
      "Docker runs inside a Linux VM via Docker Desktop.",
      "Docker containers use linux/arm64 (Apple Silicon) or linux/amd64 (Intel Mac).",
      "Use Linux-compatible packages in Dockerfiles — not macOS packages.",
    );
  }

  return lines.join("\n");
}

async function buildSystemPrompt(mission, target, workspace, planSteps) {
  const missionGoal = mission?.goal ?? "Complete the requested task.";
  const targetPath = target?.path ?? workspace;
  const targetType = target?.type ?? "directory";

  const stepsText =
    planSteps?.length > 0
      ? planSteps.map((s, i) => `${i + 1}. ${s}`).join("\n")
      : "(determine the steps yourself based on the goal)";

  let template;
  try {
    template = await readFile(AGENT_PROMPT_PATH, "utf8");
  } catch {
    template = buildFallbackSystemPrompt();
  }

  return template
    .replace("{mission_goal}", missionGoal)
    .replace("{target_path}", targetPath)
    .replace("{target_type}", targetType)
    .replace("{workspace}", workspace)
    .replace("{plan_steps}", stepsText)
    .replace("{tools_section}", toolsPromptSection())
    .replace("{environment}", detectHostEnvironment())
    .replace("{history}", "(none yet — this is the first turn)");
}

function buildFallbackSystemPrompt() {
  return `You are DeCIpher, a mission-driven local execution agent.

## Mission
Goal: {mission_goal}
Target: {target_path} ({target_type})
Working directory: {workspace}

## Plan
{plan_steps}

## Available Tools
{tools_section}

## Output Format
Respond ONLY with a JSON object:
\`\`\`json
{ "reasoning": "...", "tool": "tool_name", "args": { ... } }
\`\`\`

Call done only when the user's goal is verified as satisfied.

## History
{history}`;
}

// ── Tool call parsing ─────────────────────────────────────────────────────────

function parseToolCall(raw) {
  try {
    const cleaned = raw
      .replace(/^```(?:json)?\s*/m, "")
      .replace(/\s*```\s*$/m, "")
      .trim();
    const parsed = JSON.parse(cleaned);
    const tool = parsed.tool;
    if (!tool || !TOOL_REGISTRY[tool]) return null;
    return {
      tool,
      args: parsed.args ?? {},
      reasoning: String(parsed.reasoning ?? "").slice(0, 200),
    };
  } catch {
    return null;
  }
}

// ── Tool result formatting ────────────────────────────────────────────────────

const OUTPUT_LIMIT = 4000;
const FILE_LIMIT = 6000;

function formatToolResult(toolName, args, result) {
  const header = `[Tool result: ${toolName}]`;
  switch (toolName) {
    case "exec_command": {
      const fullOutput = result.output ?? "";
      const truncated = fullOutput.length > OUTPUT_LIMIT;
      const preview = truncated
        ? fullOutput.slice(0, OUTPUT_LIMIT) +
          `\n... (output truncated: ${fullOutput.length} chars total — run read_file or exec_command to see full output)`
        : fullOutput;
      return `${header}\nCommand: ${args.cmd}\nExit code: ${result.exitCode}${result.exitCode !== 0 ? " (FAILED)" : ""}\nOutput:\n${preview || "(no output)"}`;
    }
    case "read_file": {
      if (!result.success)
        return `${header}\nError reading ${args.path}: ${result.error}`;
      const content = result.content ?? "";
      const truncated = content.length > FILE_LIMIT;
      const preview = truncated
        ? content.slice(0, FILE_LIMIT) +
          `\n... (file truncated: ${content.length} chars total)`
        : content;
      return `${header}\nPath: ${result.path}\nContent:\n${preview}`;
    }
    case "write_file":
      return result.success
        ? `${header}\nWrote: ${result.path} (${result.previous_existed ? "overwritten" : "created new"})`
        : `${header}\nError writing ${args.path}: ${result.error}`;
    case "apply_patch":
      return result.success
        ? `${header}\nPatch applied to: ${result.applied_to}`
        : `${header}\nPatch failed: ${result.error}`;
    case "update_plan":
      return `${header}\nPlan updated (${(result.steps ?? []).length} steps).`;
    default:
      return `${header}\n${JSON.stringify(result).slice(0, 500)}`;
  }
}

// ── Plan update renderer ──────────────────────────────────────────────────────

function renderPlanUpdate(steps) {
  console.log(pc.bold("\n  [PLAN]"));
  for (const s of steps) {
    const icon =
      s.status === "completed"
        ? pc.green("✓")
        : s.status === "failed"
          ? pc.red("✗")
          : s.status === "in_progress"
            ? pc.yellow("→")
            : pc.dim("○");
    console.log(`    ${icon} ${s.step}`);
  }
}

// ── Main agent loop ───────────────────────────────────────────────────────────

/**
 * Run the agent-driven execution loop.
 *
 * @param {object|null} mission   — { goal, type, id, steps, … }
 * @param {object|null} target    — { path, type, meta }
 * @param {object}      config    — API config
 * @param {object}      options
 * @returns {Promise<LoopResult>}
 */
export async function runAgentLoop(mission, target, config, options = {}) {
  const isGreenfield = target?.meta?.start_state === "empty";
  const missionId = mission?.id ?? target?.meta?.id ?? "adhoc";
  const planSteps = options.planSteps ?? mission?.steps ?? [];

  const lines = [];
  const log = (msg) => lines.push(msg);

  const notify = async (status, stopReason = null, workspacePath = null) => {
    try {
      await runCompletionNotification(config.notification_command, {
        status,
        targetPath: target?.path,
        workspacePath,
        stopReason,
      });
    } catch {
      /* ignore notification errors */
    }
  };

  // ── Workspace setup ────────────────────────────────────────
  // Work directly in the user's directory whenever possible.
  // Only create a temp workspace for git_repo (need a clean clone dir)
  // or when no real target path exists.
  const sp = startSpinner("Preparing workspace");
  let workspace;
  const isGitRepo = target?.type === "git_repo";
  try {
    if (options.workspace) {
      workspace = options.workspace;
      sp.stop(`Resuming → ${pc.dim(workspace)}`);
    } else if (isGitRepo) {
      // Git repos clone into a fresh temp dir
      workspace = await createEmptyWorkspace(missionId);
      sp.stop(`Workspace → ${pc.dim(workspace)}`);
    } else if (isGreenfield && target?.path) {
      // Greenfield with a real path — work directly there
      workspace = target.path;
      sp.stop(`Working in → ${pc.dim(workspace)}`);
    } else if (isGreenfield) {
      // Greenfield with no path — temp workspace
      workspace = await createEmptyWorkspace(missionId);
      sp.stop(`Workspace → ${pc.dim(workspace)}`);
    } else if (target?.type === "scenario") {
      workspace = join(target.path, "broken");
      sp.stop(`Working in → ${pc.dim(workspace)}`);
    } else {
      workspace = target?.path ?? process.cwd();
      sp.stop(`Working in → ${pc.dim(workspace)}`);
    }
  } catch (err) {
    sp.stop();
    log(pc.red(`  workspace error: ${err.message}`));
    return makeFailResult("BUILD_FAIL", 0, lines, err.message);
  }

  const sessionState = options.sessionContext ?? {};

  const context = {
    workspace,
    sessionState,
    config,
    log,
    onPlanUpdate: renderPlanUpdate,
  };

  // ── Build system prompt (async, template-based) ────────────
  const systemPrompt = await buildSystemPrompt(
    mission,
    target,
    workspace,
    planSteps,
  );

  // ── Conversation history ────────────────────────────────────
  const messages = [
    {
      role: "user",
      content: buildInitialUserMessage(mission, target, workspace),
    },
  ];

  let outcome = "FAIL";
  let finalSummary = "";
  let lastPatch = null;
  let completedTurns = 0;
  let preserveWorkspace = false;
  let consecutiveFailures = 0;
  const failureHistory = new Map(); // "tool:argsHash" → count
  const missionStartTime = Date.now();

  log(pc.bold(`\n  [agent] mission: ${mission?.goal ?? "(no goal)"}`));
  log(pc.dim(`  workspace: ${workspace}`));
  log(pc.dim(`  max_turns: ${MAX_TURNS}`));

  // ── Main turn loop ─────────────────────────────────────────
  try {
    for (let turn = 1; turn <= MAX_TURNS; turn++) {
      completedTurns = turn;
      log(pc.bold(`\n  ── Turn ${turn}/${MAX_TURNS} ──`));

      // Call LLM (withRetry handles transient errors automatically)
      const turnSp = startSpinner(`Agent turn ${turn}`);
      let raw;
      try {
        raw = await callAIWithMessages(messages, config, systemPrompt);
      } catch (err) {
        // Only reaches here after all retries are exhausted (terminal error)
        turnSp.stop(pc.red("API error (retries exhausted)"));
        log(pc.red(`  turn ${turn} API error: ${err.message}`));
        finalSummary = `API error after retries: ${err.message}`;
        preserveWorkspace = true;
        break;
      }

      // Parse tool call
      const toolCall = parseToolCall(raw);
      if (!toolCall) {
        turnSp.stop(pc.yellow("no valid tool call — retrying"));
        log(pc.yellow(`  turn ${turn}: no valid JSON tool call in response`));
        messages.push({ role: "assistant", content: raw });
        messages.push({
          role: "user",
          content:
            "Your response was not a valid JSON tool call. " +
            'Respond only with a JSON object: { "reasoning": "...", "tool": "...", "args": { ... } }',
        });
        continue;
      }

      turnSp.stop(
        `${pc.cyan(toolCall.tool)} — ${toolCall.reasoning.slice(0, 60)}`,
      );
      log(`  tool: ${toolCall.tool} | ${toolCall.reasoning}`);

      messages.push({ role: "assistant", content: raw });

      // ── done ────────────────────────────────────────────────
      if (toolCall.tool === "done") {
        outcome = toolCall.args.outcome === "FAIL" ? "FAIL" : "PASS";
        finalSummary = toolCall.args.summary ?? "Mission complete.";
        log(pc.bold(`  [done] ${outcome} — ${finalSummary}`));
        if (outcome !== "PASS") preserveWorkspace = true;
        break;
      }

      // ── Approval gate for risky operations ──────────────────
      if (isToolRisky(toolCall.tool, toolCall.args)) {
        const alreadyApproved = sessionState.approved ?? false;
        if (!alreadyApproved) {
          const rl = options.rl ?? sessionState.rl ?? null;
          if (rl) {
            const { askApproval } = await import("./index.js");
            const approved = await askApproval(rl, sessionState, toolCall);
            if (!approved) {
              log(pc.yellow("  approval denied — stopping"));
              outcome = "FAIL";
              finalSummary = "Stopped: approval denied for risky operation.";
              preserveWorkspace = true;
              break;
            }
          }
          // No rl available → auto-approve (non-interactive mode)
        }
      }

      // ── Execute tool ─────────────────────────────────────────
      const execSp = startSpinner(`${toolCall.tool}`);
      let toolResult;
      try {
        toolResult = await TOOL_REGISTRY[toolCall.tool].handler(
          toolCall.args,
          context,
        );
      } catch (err) {
        toolResult = { success: false, error: err.message };
      }

      const resultText = formatToolResult(
        toolCall.tool,
        toolCall.args,
        toolResult,
      );
      // Stop spinner — show elapsed time only, no redundant "ok" text
      execSp.stop();
      const resultOk = toolResult.success !== false;
      log(
        pc.dim(
          `  result: ${resultText.split("\n").slice(0, 2).join(" | ").slice(0, 120)}`,
        ),
      );

      // ── Repair loop detection ─────────────────────────────────
      // Track consecutive failures and same-operation repeats
      if (!resultOk) {
        consecutiveFailures++;
        const argsKey = `${toolCall.tool}:${simpleHash(JSON.stringify(toolCall.args))}`;
        failureHistory.set(argsKey, (failureHistory.get(argsKey) ?? 0) + 1);

        if (failureHistory.get(argsKey) >= 2) {
          // Same exact operation failed twice — inject escalation prompt
          log(pc.yellow("  same operation failed twice — escalating strategy"));
          messages.push({ role: "assistant", content: raw });
          messages.push({
            role: "user",
            content:
              resultText +
              "\n\n[STRATEGY ESCALATION] The same approach has failed twice. " +
              "STOP and think about the ROOT CAUSE. What is fundamentally wrong? " +
              "Do NOT retry the same approach. Try a completely different strategy. " +
              "Read the error carefully. Form a hypothesis. Test it with a minimal action.",
          });
          continue;
        }

        if (consecutiveFailures >= 3) {
          log(pc.yellow("  3 consecutive failures — forcing strategy shift"));
          messages.push({ role: "assistant", content: raw });
          messages.push({
            role: "user",
            content:
              resultText +
              "\n\n[3 CONSECUTIVE FAILURES] Your recent approaches are not working. " +
              "Step back and reconsider the problem from scratch. " +
              "What assumptions are you making that might be wrong? " +
              "Try a fundamentally different approach.",
          });
          consecutiveFailures = 0; // reset after escalation
          continue;
        }
      } else {
        consecutiveFailures = 0; // reset on success
      }

      // Detect patch loop (same patch applied twice)
      if (toolCall.tool === "apply_patch") {
        const fp = toolCall.args.patch ?? "";
        if (fp && fp === lastPatch) {
          log(pc.yellow("  same patch twice — stopping"));
          outcome = "FAIL";
          finalSummary =
            "Loop detected: same patch applied twice without progress.";
          preserveWorkspace = true;
          break;
        }
        lastPatch = fp;

        // Track patch targets for corruption detection
        const patchTarget =
          toolCall.args.target_file ??
          fp.match(/^\+\+\+ b\/(.+)$/m)?.[1] ??
          "unknown";
        sessionState._lastPatchTarget = patchTarget;
      }

      // Detect patch corruption pattern: write_file immediately after apply_patch
      // on the same file = the patch corrupted the file and had to be rewritten.
      if (
        toolCall.tool === "write_file" &&
        sessionState._lastPatchTarget &&
        toolCall.args.path &&
        toolCall.args.path.includes(sessionState._lastPatchTarget)
      ) {
        sessionState._patchCorruptionCount =
          (sessionState._patchCorruptionCount ?? 0) + 1;
        if (sessionState._patchCorruptionCount >= 2) {
          log(
            pc.yellow(
              "  patch corruption detected twice — injecting write_file preference",
            ),
          );
          messages.push({ role: "assistant", content: raw });
          messages.push({
            role: "user",
            content:
              resultText +
              "\n\n[TOOL GUIDANCE] apply_patch has corrupted this file multiple times. " +
              "From now on, use ONLY write_file to modify this file. " +
              "Do NOT use apply_patch on Dockerfiles, scripts, or config files.",
          });
          sessionState._lastPatchTarget = null;
          continue;
        }
      }

      messages.push({ role: "user", content: resultText });

      // ── Auto-compaction ─────────────────────────────────────
      // Rough estimate: ~4 chars per token. Compact when messages get large
      // to prevent context limit errors (which killed the HPL run at turn 14).
      const totalChars = messages.reduce(
        (sum, m) => sum + (m.content?.length ?? 0),
        0,
      );
      if (totalChars > 60_000 && messages.length > 6) {
        // Keep: first user message, last 4 message pairs, all error messages
        const keepFirst = messages.slice(0, 1);
        const keepRecent = messages.slice(-8);
        const middle = messages.slice(1, -8);
        const summary = middle
          .filter((m) => m.role === "user" && m.content.includes("Exit code:"))
          .map((m) => m.content.split("\n").slice(0, 2).join(" "))
          .join("\n");
        const compacted = [
          ...keepFirst,
          {
            role: "user",
            content: `[Earlier turns compacted — ${middle.length} messages summarized]\n${summary || "(no errors in compacted range)"}`,
          },
          ...keepRecent,
        ];
        messages.length = 0;
        messages.push(...compacted);
        log(
          pc.dim(
            `  [compacted] ${middle.length} older messages → summary (${totalChars} → ${messages.reduce((s, m) => s + (m.content?.length ?? 0), 0)} chars)`,
          ),
        );
      }

      // Periodic snapshot
      if (turn % 5 === 0) {
        await persistSessionSnapshot({
          target_path: target?.path,
          target_type: target?.type,
          mission,
          mission_summary: mission?.goal,
          execution_mode: "agent",
          iteration: turn,
          max_iterations: MAX_TURNS,
          workspace_path: workspace,
          last_verification_state: "RUNNING",
          resumable: true,
          outcome: "RUNNING",
          transcript: lines.join("\n"),
        }).catch(() => null);
      }
    }
  } finally {
    // Only clean up TEMP workspaces on failure (git_repo clones, no-path greenfield).
    // Never delete the user's own directory.
    const isTempWorkspace = isGitRepo || (isGreenfield && !target?.path);
    if (isTempWorkspace && outcome !== "PASS") {
      preserveWorkspace = false;
      try {
        const { rm } = await import("node:fs/promises");
        await rm(workspace, { recursive: true, force: true });
        log(pc.dim(`  [cleanup] removed failed temp workspace: ${workspace}`));
      } catch {
        // ignore cleanup errors
      }
    }
  }

  const finalState = outcome === "PASS" ? "PASS" : "AGENT_FAIL";
  const transcriptText = lines.join("\n");

  // Persist transcript as a standalone file for /transcript inspection
  const transcriptPath = await persistTranscript(
    transcriptText,
    missionId,
  ).catch(() => null);

  // Record preserved workspace for /artifacts discovery
  if (preserveWorkspace && workspace) {
    await recordPreservedWorkspace(workspace, {
      mission_id: missionId,
      reason: outcome !== "PASS" ? "failure" : "review",
    }).catch(() => null);
  }

  const rawSnapshot = {
    target_path: target?.path,
    target_type: target?.type,
    mission,
    mission_summary: mission?.goal,
    execution_mode: "agent",
    iteration: completedTurns,
    max_iterations: MAX_TURNS,
    workspace_path: preserveWorkspace ? workspace : null,
    last_verification_state: finalState,
    resumable: outcome !== "PASS",
    outcome,
    transcript: transcriptText,
    transcript_path: transcriptPath,
  };

  // Compact long sessions before persisting to session.json
  const snapshot =
    completedTurns > 10 ? compactSessionSnapshot(rawSnapshot) : rawSnapshot;

  await persistSessionSnapshot(snapshot).catch(() => null);

  await notify(
    finalState,
    outcome !== "PASS" ? "agent_loop_complete" : null,
    preserveWorkspace ? workspace : null,
  );

  return {
    outcome,
    state: finalState,
    writtenBack: [],
    workspace: preserveWorkspace ? workspace : null,
    iterations: completedTurns,
    classification: {},
    patch: lastPatch,
    executionMode: "agent",
    containerStarted: false,
    cleanupPerformed: false,
    preservedArtifacts: null,
    summary: finalSummary,
    transcript: transcriptText,
    transcriptPath,
    elapsedMs: Date.now() - missionStartTime,
  };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function simpleHash(str) {
  let h = 0;
  for (let i = 0; i < str.length; i++) {
    h = ((h << 5) - h + str.charCodeAt(i)) | 0;
  }
  return h.toString(36);
}

function buildInitialUserMessage(mission, target, workspace) {
  const goal = mission?.goal ?? "Complete the task.";
  const parts = [`Mission: ${goal}`, `Workspace: ${workspace}`];
  if (target?.path) parts.push(`Target path: ${target.path}`);
  if (target?.type) parts.push(`Target type: ${target.type}`);
  parts.push("\nBegin. What is your first action?");
  return parts.join("\n");
}

function makeFailResult(state, iterations, lines, reason = "") {
  return {
    outcome: "FAIL",
    state,
    writtenBack: [],
    workspace: null,
    iterations,
    classification: {},
    patch: null,
    executionMode: "agent",
    containerStarted: false,
    cleanupPerformed: false,
    preservedArtifacts: null,
    summary: reason || `Failed with state: ${state}`,
    transcript: lines.join("\n"),
  };
}

// ── Result printer ────────────────────────────────────────────────────────────

/**
 * Print a human-readable summary of a runAgentLoop result.
 * Signature matches printLoopResult so callers can use either interchangeably.
 */
export function printAgentLoopResult(result) {
  const divider = pc.dim("─".repeat(60));
  console.log(`\n${divider}`);
  console.log(pc.bold("\n  [RESULT]"));

  const outcomeLabel =
    result.outcome === "PASS"
      ? pc.bold(pc.green("PASS"))
      : pc.bold(pc.red("FAIL"));

  const elapsed = result.elapsedMs
    ? pc.dim(`(${(result.elapsedMs / 1000).toFixed(1)}s)`)
    : "";

  console.log(`  Outcome:     ${outcomeLabel} ${elapsed}`);
  console.log(`  Turns:       ${result.iterations}`);

  if (result.summary) {
    console.log(`  Summary:     ${result.summary}`);
  }

  if (result.workspace) {
    console.log(pc.bold("\n  [WORKSPACE]"));
    console.log(`  ${pc.cyan(result.workspace)}`);
  }

  console.log(`\n${divider}\n`);
}
