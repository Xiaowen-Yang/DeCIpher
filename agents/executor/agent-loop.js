/**
 * Agent-driven execution loop — the Codex-style runtime core.
 *
 * The LLM agent receives: mission goal + workspace + available tools + history.
 * It responds with native tool calls (OpenAI tools / Anthropic tool_use).
 * The loop executes the tools, feeds results back, and repeats until the
 * agent calls `done` or limits are hit.
 *
 * V4: Uses native function calling instead of JSON text parsing.
 * Tools are sent as structured JSON schemas alongside messages.
 */

import { join } from "node:path";
import pc from "picocolors";
import { startSpinner } from "../../lib/spinner.js";
import {
  persistSessionSnapshot,
  persistTranscript,
  recordPreservedWorkspace,
  compactSessionSnapshot,
} from "../../lib/session-store.js";
import { runCompletionNotification } from "../../lib/notifications.js";
import { TOOL_REGISTRY } from "./tools.js";
import {
  evaluatePolicy,
  createAmendments,
  recordApproval,
  Decision,
  PolicyMode,
} from "../../lib/exec-policy.js";
import {
  buildToolsForProvider,
  formatAssistantToolCallMessage,
  formatToolResultMessage,
  formatUserMessage,
  formatAssistantMessage,
} from "./tool-schemas.js";
import { callAIWithToolsStreaming } from "../../lib/api-client.js";
import { readFile, writeFile, mkdir } from "node:fs/promises";

const MAX_TURNS = 20;
const AGENT_PROMPT_PATH = new URL("../../prompts/agent.md", import.meta.url)
  .pathname;

// ── System prompt builder ─────────────────────────────────────────────────────

function detectHostEnvironment() {
  const os = process.platform;
  const arch = process.arch;
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
    .replace("{environment}", detectHostEnvironment());
}

function buildFallbackSystemPrompt() {
  return `You are DeCIpher, a mission-driven local execution agent.

## Mission
Goal: {mission_goal}
Target: {target_path} ({target_type})
Working directory: {workspace}

## Plan
{plan_steps}

## Environment
{environment}

## Instructions
Use the available tools to accomplish the mission. Call done only when the user's goal is verified as satisfied.

## History
{history}`;
}

// ── Tool result formatting ────────────────────────────────────────────────────

const OUTPUT_LIMIT = 4000;
const OUTPUT_LIMIT_FAILURE = 8000;
const FILE_LIMIT = 6000;

function formatToolResult(toolName, args, result) {
  const header = `[Tool result: ${toolName}]`;
  switch (toolName) {
    case "exec_command": {
      const fullOutput = result.output ?? "";
      const limit = result.exitCode !== 0 ? OUTPUT_LIMIT_FAILURE : OUTPUT_LIMIT;
      const truncated = fullOutput.length > limit;
      const preview = truncated
        ? fullOutput.slice(0, limit) +
          `\n... (output truncated: ${fullOutput.length} chars total)`
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
        ? pc.green("\u2713")
        : s.status === "failed"
          ? pc.red("\u2717")
          : s.status === "in_progress"
            ? pc.yellow("\u2192")
            : pc.dim("\u25CB");
    console.log(`    ${icon} ${s.step}`);
  }
}

// ── Main agent loop ───────────────────────────────────────────────────────────

/**
 * Run the agent-driven execution loop.
 *
 * @param {object|null} mission   — { goal, type, id, steps, ... }
 * @param {object|null} target    — { path, type, meta }
 * @param {object}      config    — API config
 * @param {object}      options
 * @returns {Promise<LoopResult>}
 */
export async function runAgentLoop(mission, target, config, options = {}) {
  const isGreenfield =
    target?.meta?.start_state === "empty" ||
    target?.type === "new_directory" ||
    mission?.type === "greenfield";
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

  // ── Workspace setup ────────────────────────────────────
  const sp = startSpinner("Preparing workspace");
  let workspace;
  try {
    if (options.workspace) {
      workspace = options.workspace;
      sp.stop(`Resuming \u2192 ${pc.dim(workspace)}`);
    } else if (target?.type === "scenario") {
      workspace = join(target.path, "broken");
      sp.stop(`Working in \u2192 ${pc.dim(workspace)}`);
    } else if (target?.path && target.type !== "git_repo") {
      await mkdir(target.path, { recursive: true });
      workspace = target.path;
      sp.stop(`Working in \u2192 ${pc.dim(workspace)}`);
    } else {
      workspace = process.cwd();
      sp.stop(`Working in \u2192 ${pc.dim(workspace)}`);
    }
  } catch (err) {
    sp.stop();
    log(pc.red(`  workspace error: ${err.message}`));
    return makeFailResult("BUILD_FAIL", 0, lines, err.message);
  }

  const sessionState = options.sessionContext ?? {};

  // ── Policy engine setup ─────────────────────────────────
  const policyMode =
    options.policyMode ?? config.approval_policy_mode ?? PolicyMode.AUTO;
  const amendments = sessionState._amendments ?? createAmendments();
  sessionState._amendments = amendments;

  const context = {
    workspace,
    sessionState,
    config,
    log,
    onPlanUpdate: renderPlanUpdate,
    onExecOutput: options.onExecOutput ?? null,
  };

  // ── Build system prompt (async, template-based) ────────────
  const systemPrompt = await buildSystemPrompt(
    mission,
    target,
    workspace,
    planSteps,
  );

  // ── Native tool calling setup ──────────────────────────────
  const provider = config.base_url ? "openai" : (config.provider ?? "openai");
  const tools = buildToolsForProvider(provider);

  // ── Conversation history ────────────────────────────────────
  const messages = [
    formatUserMessage(buildInitialUserMessage(mission, target, workspace)),
  ];

  let outcome = "FAIL";
  let finalSummary = "";
  let doneResult = null;
  let lastPatch = null;
  let completedTurns = 0;
  let preserveWorkspace = false;
  let consecutiveNoToolCalls = 0;
  const MAX_NO_TOOL_CALLS = 3;
  const totalUsage = {
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
  };
  let consecutiveFailures = 0;
  const failureHistory = new Map();
  const missionStartTime = Date.now();

  log(pc.bold(`\n  [agent] mission: ${mission?.goal ?? "(no goal)"}`));
  log(pc.dim(`  workspace: ${workspace}`));
  log(pc.dim(`  max_turns: ${MAX_TURNS}`));
  log(pc.dim(`  provider: ${provider} (native tool calling)`));

  // ── Main turn loop ─────────────────────────────────────────
  try {
    for (let turn = 1; turn <= MAX_TURNS; turn++) {
      completedTurns = turn;
      log(pc.bold(`\n  \u2500\u2500 Turn ${turn}/${MAX_TURNS} \u2500\u2500`));

      // Send status update to TUI
      if (options.onStatus) {
        options.onStatus({
          phase: "thinking",
          turn,
          max_turns: MAX_TURNS,
          elapsed_ms: Date.now() - missionStartTime,
        });
      }

      // ── Call LLM with native tool calling + streaming ─────────
      const turnSp = startSpinner(`Agent turn ${turn}`);
      let response;
      try {
        response = await callAIWithToolsStreaming(
          messages,
          tools,
          config,
          systemPrompt,
          options.onDelta ?? null,
          options.onReasoning ?? null,
        );
      } catch (err) {
        turnSp.stop(pc.red("API error (retries exhausted)"));
        log(pc.red(`  turn ${turn} API error: ${err.message}`));
        finalSummary = `API error after retries: ${err.message}`;
        preserveWorkspace = true;
        break;
      }

      // Accumulate per-turn usage
      const { getLastUsage } = await import("../../lib/api-client.js");
      const turnUsage = response.usage ?? getLastUsage();
      if (turnUsage) {
        totalUsage.prompt_tokens += turnUsage.prompt_tokens;
        totalUsage.completion_tokens += turnUsage.completion_tokens;
        totalUsage.total_tokens += turnUsage.total_tokens;
        if (options.onUsage) options.onUsage(turnUsage, totalUsage);
      }

      // ── Handle text-only response (no tool calls) ──────────
      if (response.type === "text" || response.toolCalls.length === 0) {
        consecutiveNoToolCalls++;
        turnSp.stop(pc.yellow("no tool call \u2014 retrying"));
        log(
          pc.yellow(
            `  turn ${turn}: LLM returned text without tool calls (${consecutiveNoToolCalls}/${MAX_NO_TOOL_CALLS})`,
          ),
        );

        if (consecutiveNoToolCalls >= MAX_NO_TOOL_CALLS) {
          log(
            pc.red(
              `  ${MAX_NO_TOOL_CALLS} turns without tool calls \u2014 aborting`,
            ),
          );
          finalSummary = `Agent failed to produce tool calls after ${MAX_NO_TOOL_CALLS} attempts.`;
          preserveWorkspace = true;
          break;
        }

        if (response.content) {
          messages.push(formatAssistantMessage(response.content));
        }
        messages.push(
          formatUserMessage(
            "You must use one of the available tools to make progress. " +
              "Call a tool now to continue working on the mission.",
          ),
        );
        continue;
      }
      consecutiveNoToolCalls = 0;

      // ── Push assistant's tool-calling message to history ──
      messages.push(
        formatAssistantToolCallMessage(
          provider,
          response.toolCalls,
          response.content,
        ),
      );

      // ── Process each tool call sequentially ────────────────
      let doneInThisTurn = false;

      for (const tc of response.toolCalls) {
        const toolName = tc.name;
        const toolArgs = tc.input ?? {};
        const toolCallId = tc.id;

        // Validate tool exists
        if (!TOOL_REGISTRY[toolName]) {
          log(pc.yellow(`  unknown tool: ${toolName} \u2014 skipping`));
          messages.push(
            formatToolResultMessage(
              provider,
              toolCallId,
              `Error: Unknown tool "${toolName}". Available tools: ${Object.keys(TOOL_REGISTRY).join(", ")}`,
              true,
            ),
          );
          continue;
        }

        turnSp.stop(`${pc.cyan(toolName)}`);
        log(`  tool: ${toolName}`);

        // Notify TUI of tool start
        if (options.onToolStart) {
          options.onToolStart(toolName, response.content ?? "", toolArgs);
        }

        // ── done ────────────────────────────────────────────────
        if (toolName === "done") {
          outcome =
            toolArgs.outcome === "FAIL"
              ? "FAIL"
              : toolArgs.outcome === "PARTIAL"
                ? "PARTIAL"
                : "PASS";
          finalSummary = toolArgs.summary ?? "Mission complete.";
          doneResult = {
            files_modified: toolArgs.files_modified ?? [],
            errors_encountered: toolArgs.errors_encountered ?? [],
            next_steps: toolArgs.next_steps ?? [],
          };
          log(pc.bold(`  [done] ${outcome} \u2014 ${finalSummary}`));
          if (outcome !== "PASS") preserveWorkspace = true;
          doneInThisTurn = true;
          break;
        }

        // ── Policy-driven approval gate ─────────────────────────
        const policyResult = evaluatePolicy(
          policyMode,
          toolName,
          toolArgs,
          amendments,
          workspace,
        );

        if (policyResult.decision === Decision.DENY) {
          log(
            pc.red(
              `  denied [${policyResult.toolClass}]: ${policyResult.reason}`,
            ),
          );
          messages.push(
            formatToolResultMessage(
              provider,
              toolCallId,
              `Error: Action denied by policy (${policyResult.reason}). ` +
                `Try a different approach that does not require ${policyResult.toolClass} access.`,
              true,
            ),
          );
          continue;
        }

        if (policyResult.decision === Decision.ASK) {
          // Support both TUI callback and readline approval mechanisms
          const approvalFn = options.askApproval ?? null;
          const rl = options.rl ?? sessionState.rl ?? null;
          let approved = true; // default if no approval mechanism

          if (approvalFn) {
            // TUI mode: callback-based approval
            approved = await approvalFn({
              tool: toolName,
              args: toolArgs,
              toolClass: policyResult.toolClass,
              reason: policyResult.reason,
            });
          } else if (rl) {
            // Readline mode: interactive prompt
            const { askApproval: askApprovalRL } = await import("./index.js");
            approved = await askApprovalRL(rl, sessionState, {
              tool: toolName,
              args: toolArgs,
            });
          }

          if (!approved) {
            log(pc.yellow("  approval denied \u2014 stopping"));
            outcome = "FAIL";
            finalSummary = "Stopped: approval denied for risky operation.";
            preserveWorkspace = true;
            doneInThisTurn = true;
            break;
          }
          // Record approval for this tool class (ask-once-per-class in auto mode)
          recordApproval(amendments, policyResult.toolClass, toolName);
        }

        // ── Execute tool ─────────────────────────────────────────
        const execSp = startSpinner(`${toolName}`);
        const execStartMs = Date.now();
        let toolResult;
        try {
          toolResult = await TOOL_REGISTRY[toolName].handler(toolArgs, context);
        } catch (err) {
          toolResult = { success: false, error: err.message };
        }

        const resultText = formatToolResult(toolName, toolArgs, toolResult);
        const execElapsedMs = Date.now() - execStartMs;
        execSp.stop();
        const resultOk = toolResult.success !== false;

        // Notify TUI of tool completion
        if (options.onToolResult) {
          const summary = resultOk
            ? (
                toolResult.summary ??
                resultText.split("\n").slice(0, 1).join("")
              ).slice(0, 120)
            : (toolResult.error ?? "failed").slice(0, 120);
          const outputText = toolResult.output ?? "";
          const outputLines = outputText.split("\n");
          const outputPreview = resultOk
            ? null
            : outputLines.slice(-5).join("\n").slice(0, 500);
          options.onToolResult(toolName, resultOk, summary, execElapsedMs, {
            exit_code: toolResult.exitCode ?? null,
            output_preview: outputPreview,
            output_lines_total: outputLines.length,
          });
        }
        log(
          pc.dim(
            `  result: ${resultText.split("\n").slice(0, 2).join(" | ").slice(0, 120)}`,
          ),
        );

        // Push tool result to conversation history
        messages.push(
          formatToolResultMessage(provider, toolCallId, resultText, !resultOk),
        );

        // ── Repair loop detection ─────────────────────────────────
        if (!resultOk) {
          consecutiveFailures++;
          const argsKey = `${toolName}:${simpleHash(JSON.stringify(toolArgs))}`;
          failureHistory.set(argsKey, (failureHistory.get(argsKey) ?? 0) + 1);

          if (failureHistory.get(argsKey) >= 2) {
            log(
              pc.yellow(
                "  same operation failed twice \u2014 escalating strategy",
              ),
            );
            messages.push(
              formatUserMessage(
                "[STRATEGY ESCALATION] The same approach has failed twice. " +
                  "STOP and think about the ROOT CAUSE. " +
                  "Do NOT retry the same approach. Try a completely different strategy.",
              ),
            );
          } else if (consecutiveFailures >= 3) {
            log(
              pc.yellow(
                "  3 consecutive failures \u2014 forcing strategy shift",
              ),
            );
            messages.push(
              formatUserMessage(
                "[3 CONSECUTIVE FAILURES] Your recent approaches are not working. " +
                  "Step back and reconsider the problem from scratch. " +
                  "Try a fundamentally different approach.",
              ),
            );
            consecutiveFailures = 0;
          }
        } else {
          consecutiveFailures = 0;
        }

        // Detect patch loop
        if (toolName === "apply_patch") {
          const fp = toolArgs.patch ?? "";
          if (fp && fp === lastPatch) {
            log(pc.yellow("  same patch twice \u2014 stopping"));
            outcome = "FAIL";
            finalSummary =
              "Loop detected: same patch applied twice without progress.";
            preserveWorkspace = true;
            doneInThisTurn = true;
            break;
          }
          lastPatch = fp;

          const patchTarget =
            toolArgs.target_file ??
            fp.match(/^\+\+\+ b\/(.+)$/m)?.[1] ??
            "unknown";
          sessionState._lastPatchTarget = patchTarget;
        }

        // Detect patch corruption pattern
        if (
          toolName === "write_file" &&
          sessionState._lastPatchTarget &&
          toolArgs.path &&
          toolArgs.path.includes(sessionState._lastPatchTarget)
        ) {
          sessionState._patchCorruptionCount =
            (sessionState._patchCorruptionCount ?? 0) + 1;
          if (sessionState._patchCorruptionCount >= 2) {
            log(
              pc.yellow(
                "  patch corruption detected twice \u2014 injecting write_file preference",
              ),
            );
            messages.push(
              formatUserMessage(
                "[TOOL GUIDANCE] apply_patch has corrupted this file multiple times. " +
                  "From now on, use ONLY write_file to modify this file.",
              ),
            );
            sessionState._lastPatchTarget = null;
          }
        }
      } // end for-each tool call

      // If done was called inside the inner loop, break outer loop
      if (doneInThisTurn) break;

      // ── Token-based auto-compaction ─────────────────────────
      const { shouldCompact, compactMessages } =
        await import("../../lib/compact.js");
      const lastPromptTokens = totalUsage.prompt_tokens;
      if (
        shouldCompact(lastPromptTokens, config.model) &&
        messages.length > 8
      ) {
        try {
          const workspaceReminder = [
            `Workspace: ${workspace}`,
            `Mission: ${mission?.goal ?? "(unknown)"}`,
            `Turn: ${turn}/${MAX_TURNS}`,
            `The system prompt and tool definitions are always available.`,
            `Continue working toward the mission goal.`,
          ].join("\n");
          const result = await compactMessages(messages, config, {
            keepRecent: 6,
            workspaceReminder,
          });
          messages.length = 0;
          messages.push(...result.messages);
          log(
            pc.dim(
              `  [compacted] token-based (~${Math.round(result.beforeTokens)} \u2192 ~${Math.round(result.afterTokens)} est. tokens)`,
            ),
          );
        } catch {
          const keepFirst = messages.slice(0, 1);
          const keepRecent = messages.slice(-8);
          const middle = messages.slice(1, -8);
          const compacted = [
            ...keepFirst,
            formatUserMessage(
              `[Earlier turns compacted \u2014 ${middle.length} messages summarized]`,
            ),
            ...keepRecent,
          ];
          messages.length = 0;
          messages.push(...compacted);
          log(
            pc.dim(
              `  [compacted] fallback: ${middle.length} older messages truncated`,
            ),
          );
        }
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
    if (outcome !== "PASS") {
      const containers = sessionState._dockerContainers ?? new Set();
      const images = sessionState._dockerImages ?? new Set();
      const composeDir = sessionState._dockerComposeDir ?? null;

      if (containers.size > 0 || images.size > 0 || composeDir) {
        log(
          pc.bold(
            "\n  [CLEANUP] Removing Docker resources from failed mission\u2026",
          ),
        );
        try {
          const { cleanupDockerResources } = await import("./tools.js");
          const cleaned = await cleanupDockerResources(
            containers,
            images,
            composeDir,
            log,
          );
          const total =
            cleaned.containers.length +
            cleaned.images.length +
            (cleaned.compose ? 1 : 0);
          if (total > 0) {
            log(
              pc.green(
                `  [CLEANUP] Removed ${cleaned.containers.length} container(s), ${cleaned.images.length} image(s)${cleaned.compose ? ", compose stack" : ""}`,
              ),
            );
          }
        } catch (err) {
          log(pc.yellow(`  [CLEANUP] Error during cleanup: ${err.message}`));
        }
      }
    }
  }

  const finalState = outcome === "PASS" ? "PASS" : "AGENT_FAIL";
  const transcriptText = lines.join("\n");

  const transcriptPath = await persistTranscript(
    transcriptText,
    missionId,
  ).catch(() => null);

  // Save log under docs/logs/ for development analysis
  try {
    const { fileURLToPath } = await import("node:url");
    const { dirname: dn } = await import("node:path");
    const projectRoot = join(dn(fileURLToPath(import.meta.url)), "../..");
    const logsDir = join(projectRoot, "docs/logs");
    await mkdir(logsDir, { recursive: true });
    const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    const logName = `${ts}_${missionId}.log`;
    const header = [
      `# Agent Log: ${missionId}`,
      `Date: ${new Date().toISOString()}`,
      `Goal: ${mission?.goal ?? "(unknown)"}`,
      `Target: ${target?.path ?? "(none)"} (${target?.type ?? "unknown"})`,
      `Workspace: ${workspace}`,
      `Outcome: ${outcome}`,
      `Turns: ${completedTurns}`,
      `Elapsed: ${((Date.now() - missionStartTime) / 1000).toFixed(1)}s`,
      "",
      "---",
      "",
    ].join("\n");
    await writeFile(join(logsDir, logName), header + transcriptText, "utf8");
  } catch {
    // Non-critical
  }

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
    totalUsage,
    doneResult,
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

function highlightUrls(text) {
  return text.replace(
    /https?:\/\/[^\s,)>"']+/g,
    (url) => `\x1b]8;;${url}\x07${pc.underline(pc.cyan(url))}\x1b]8;;\x07`,
  );
}

export function printAgentLoopResult(result) {
  const divider = pc.dim("\u2500".repeat(60));
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
    console.log(`  Summary:     ${highlightUrls(result.summary)}`);
  }

  if (result.workspace) {
    console.log(pc.bold("\n  [WORKSPACE]"));
    console.log(`  ${pc.cyan(result.workspace)}`);
  }

  console.log(`\n${divider}\n`);
}
