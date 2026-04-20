/**
 * JSON server mode for Rust TUI communication.
 *
 * When started with `--server`, DeCIpher reads newline-delimited JSON from
 * stdin and writes newline-delimited JSON to stdout. This allows the Rust TUI
 * (or any other frontend) to drive the agent.
 *
 * Protocol:
 *   Client → Server: { type: "user_input"|"slash_command"|"approval_response", ... }
 *   Server → Client: { type: "banner"|"mission"|"tool_start"|"tool_result"|..., ... }
 */

import { createInterface } from "node:readline";
import { homedir } from "node:os";
import { readConfig } from "./config.js";
import { createRequire } from "node:module";

const _require = createRequire(import.meta.url);
const { version } = _require("../package.json");

// ── CRITICAL: Redirect console to stderr in server mode ─────────────────
// In server mode, stdout is the JSON protocol channel to the Rust TUI.
// Any non-JSON text on stdout corrupts the protocol stream and appears as
// raw text in the TUI. All console output MUST go to stderr.
const _origLog = console.log;
const _origWarn = console.warn;
const _origError = console.error;
console.log = (...args) => process.stderr.write(args.join(" ") + "\n");
console.warn = (...args) => process.stderr.write(args.join(" ") + "\n");
console.error = (...args) => process.stderr.write(args.join(" ") + "\n");

/**
 * Send a JSON message to the TUI (stdout).
 */
function send(msg) {
  process.stdout.write(JSON.stringify(msg) + "\n");
}

/**
 * Run the JSON server mode.
 * Reads from stdin, processes messages, writes to stdout.
 */
export async function runServerMode() {
  const config = await readConfig();

  // Send banner
  const dir = process.cwd();
  const shortDir = dir.startsWith(homedir())
    ? "~" + dir.slice(homedir().length)
    : dir;

  send({
    type: "banner",
    version,
    provider: config.provider ?? "openai",
    model: config.model ?? "gpt-4o",
    directory: shortDir,
    api_key_set: !!config.api_key,
  });

  // Send available commands
  const { SLASH_COMMANDS } = await import("./cli-surface.js");
  send({
    type: "command_list",
    commands: SLASH_COMMANDS.map((c) => ({
      name: c.name,
      description: c.description,
    })),
  });

  // Session state (mirrors interactive mode)
  const sessionState = {
    approved: false,
    approvalPolicy: config.approval_policy ?? "on-request",
    currentTarget: null,
    currentMission: null,
    currentPlan: null,
    lastVerificationResult: null,
    lastRunResult: null,
    interrupted: false,
  };

  // Approval resolver — set when agent requests approval
  let approvalResolver = null;

  // Policy-aware TUI approval callback.
  // Sends approval_request to TUI and waits for approval_response.
  // Used by the exec-policy engine when decision is ASK.
  const tuiAskApproval = async (action) => {
    // "always approve" shortcut (user pressed 'a' earlier)
    if (sessionState.approved) return true;

    send({
      type: "approval_request",
      capabilities: [
        `[${action.toolClass ?? "exec"}] ${action.reason ?? action.tool}`,
      ],
      action: {
        tool: action.tool,
        reasoning: action.reason ?? null,
      },
    });

    const approved = await new Promise((resolve) => {
      approvalResolver = resolve;
    });

    return approved;
  };

  // Read JSON messages from stdin
  const rl = createInterface({ input: process.stdin });

  rl.on("line", async (line) => {
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      send({ type: "error", message: "Invalid JSON input" });
      return;
    }

    switch (msg.type) {
      case "user_input": {
        await handleUserInput(msg.text, config, sessionState, tuiAskApproval);
        break;
      }

      case "slash_command": {
        await handleSlashCommand(msg.name, msg.args, config, sessionState);
        break;
      }

      case "approval_response": {
        if (approvalResolver) {
          approvalResolver(msg.approved);
          approvalResolver = null;
        }
        break;
      }

      case "interrupt": {
        sessionState.interrupted = true;
        // If waiting for approval, auto-deny
        if (approvalResolver) {
          approvalResolver(false);
          approvalResolver = null;
        }
        send({
          type: "agent_message",
          text: "Interrupted. Ready for new input.",
        });
        break;
      }

      default:
        send({ type: "error", message: `Unknown message type: ${msg.type}` });
    }
  });

  rl.on("close", () => {
    process.exit(0);
  });
}

async function handleUserInput(input, config, sessionState, tuiAskApproval) {
  const { resolveTarget } = await import("../agents/executor/index.js");
  const { analyzeMission } = await import("./mission-analyzer.js");

  // Resolve target
  let target = null;
  try {
    target = await resolveTarget(input);
  } catch {
    /* ignore */
  }

  if (
    !target &&
    /\b(this folder|this dir|here|current|当前|这个|这里)\b/i.test(input)
  ) {
    const { resolvePathTarget } = await import("../agents/executor/index.js");
    target = (await resolvePathTarget(process.cwd()).catch(() => null)) ?? {
      path: process.cwd(),
      type: "directory",
    };
  }

  sessionState.currentTarget = target ?? sessionState.currentTarget;

  // Analyze mission
  send({ type: "spinner", label: "Understanding mission", done: false });

  const meta = target?.meta ?? {};
  const analysis = await analyzeMission(input, target, config);

  send({ type: "spinner", label: "", done: true });

  // Build mission goal
  const isGreenfield =
    meta.start_state === "empty" ||
    meta.mission_type === "greenfield" ||
    target?.type === "new_directory";

  let missionGoal;
  if (analysis.inferred) {
    const pathPattern = target?.path
      ? new RegExp(target.path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g")
      : null;
    const naturalText = pathPattern
      ? input.replace(pathPattern, "").replace(/["']/g, "").trim()
      : input;
    missionGoal =
      naturalText.length > 10
        ? naturalText
        : (meta.prompt ?? analysis.understood_as);
  } else {
    missionGoal = analysis.understood_as;
  }

  sessionState.currentMission = {
    type: isGreenfield ? "greenfield" : analysis.action,
    goal: missionGoal,
    domain: "deployment",
    stop_boundary: meta.mission_stop_boundary ?? "mission_complete",
    requires_clarification: analysis.requires_clarification,
    clarification_question: analysis.clarification_question,
  };

  // Send mission to TUI — no hard-coded plan steps.
  // The agent loop drives execution dynamically; tool_start/tool_result
  // events show the real work as it happens.
  send({
    type: "mission",
    understood: missionGoal,
    target: target?.path ?? null,
    target_type: target?.type ?? null,
    steps: [],
  });

  // Clarification gate
  if (analysis.requires_clarification) {
    send({
      type: "clarification",
      question:
        analysis.clarification_question ??
        "What do you want DeCIpher to do exactly?",
    });
    return;
  }

  // Default target
  if (!target && !analysis.requires_clarification) {
    target = { path: process.cwd(), type: "directory" };
    sessionState.currentTarget = target;
  }

  if (!target) {
    send({
      type: "error",
      message:
        "No target resolved. Please provide a path or describe the task.",
    });
    return;
  }

  // Execute
  try {
    const { executeTarget } = await import("../agents/executor/index.js");
    const { getContextWindow } = await import("./compact.js");
    const contextWindow = getContextWindow(config.model);
    const result = await executeTarget(
      target,
      analysis.action,
      config,
      sessionState,
      {
        askApproval: tuiAskApproval,
        onStatus: (status) =>
          send({
            type: "agent_status",
            phase: status.phase,
            turn: status.turn,
            max_turns: status.max_turns,
            elapsed_ms: status.elapsed_ms,
            tool_name: status.tool_name ?? undefined,
          }),
        onToolStart: (tool, reasoning, args, callId) =>
          send({
            type: "tool_start",
            tool,
            reasoning,
            args: args ?? undefined,
            call_id: callId ?? undefined,
          }),
        onToolResult: (tool, success, summary, elapsedMs, extra = {}) =>
          send({
            type: "tool_result",
            tool,
            success,
            summary,
            elapsed_ms: elapsedMs,
            exit_code: extra.exit_code ?? undefined,
            output_preview: extra.output_preview ?? undefined,
            output_lines_total: extra.output_lines_total ?? undefined,
            call_id: extra.call_id ?? undefined,
          }),
        onDelta: (delta) => {
          // Native tool calling: text content IS the agent's reasoning.
          // Forward directly — no JSON extraction needed.
          if (delta) {
            send({ type: "agent_message_delta", delta });
          }
        },
        onReasoning: (delta) => {
          // Anthropic extended thinking: forward as reasoning text.
          // P0.3 will add a dedicated reasoning cell; for now show inline.
          if (delta) {
            send({ type: "agent_message_delta", delta });
          }
        },
        onUsage: (turnUsage, accumulated) => {
          // Emit token_usage per turn for continuous TUI budget display.
          send({
            type: "token_usage",
            prompt_tokens: accumulated.prompt_tokens,
            completion_tokens: accumulated.completion_tokens,
            total_tokens: accumulated.total_tokens,
            context_window: contextWindow,
          });
        },
        onExecOutput: (chunk) =>
          send({ type: "exec_output_delta", delta: chunk }),
      },
    );

    // Final token usage (may duplicate last per-turn emit — TUI ignores if same)
    const usage = result?.totalUsage;
    if (usage && usage.total_tokens > 0) {
      send({
        type: "token_usage",
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        context_window: contextWindow,
      });
    }

    // ALWAYS send mission_complete — never leave the TUI spinner idle.
    if (result) {
      const urls = result.summary?.match(/https?:\/\/[^\s,)>"']+/g) ?? [];
      const done = result.doneResult ?? {};
      send({
        type: "mission_complete",
        outcome: result.outcome ?? "FAIL",
        summary: result.summary ?? "Mission complete.",
        turns: result.iterations ?? 0,
        elapsed_ms: result.elapsedMs ?? 0,
        urls,
        files_modified: done.files_modified ?? [],
        errors_encountered: done.errors_encountered ?? [],
        next_steps: done.next_steps ?? [],
      });
    } else {
      send({
        type: "mission_complete",
        outcome: "FAIL",
        summary: "Agent returned no result.",
        turns: 0,
        elapsed_ms: 0,
        urls: [],
        files_modified: [],
        errors_encountered: [],
        next_steps: [],
      });
    }
  } catch (err) {
    // Unhandled exception in agent loop — still send mission_complete
    // so the TUI spinner stops and the user sees the error.
    send({
      type: "mission_complete",
      outcome: "FAIL",
      summary: `Agent error: ${err.message}`,
      turns: 0,
      elapsed_ms: 0,
      urls: [],
      files_modified: [],
      errors_encountered: [],
      next_steps: [],
    });
  }
}

async function handleSlashCommand(name, args, config, sessionState) {
  const {
    buildCliModelView,
    buildCliSettingsView,
    buildCliStatusSnapshot,
    buildCliPlanView,
    buildCliReviewSnapshot,
    buildCliTranscriptView,
    buildCliArtifactsView,
    SLASH_COMMANDS,
  } = await import("./cli-surface.js");

  /** Format a plain object as key: value lines for TUI display. */
  function formatView(obj, indent = "") {
    return Object.entries(obj)
      .map(([k, v]) => {
        if (v === null || v === undefined) return `${indent}${k}: —`;
        if (typeof v === "object" && !Array.isArray(v))
          return `${indent}${k}:\n${formatView(v, indent + "  ")}`;
        if (Array.isArray(v))
          return v.length
            ? `${indent}${k}:\n${v.map((i) => `${indent}  • ${typeof i === "object" ? JSON.stringify(i) : i}`).join("\n")}`
            : `${indent}${k}: (none)`;
        return `${indent}${k}: ${v}`;
      })
      .join("\n");
  }

  switch (name) {
    case "/help": {
      const lines = SLASH_COMMANDS.map(
        (c) => `  ${c.name.padEnd(14)} ${c.description}`,
      );
      send({ type: "agent_message", text: lines.join("\n") });
      break;
    }

    case "/model": {
      if (args) {
        // Switch model: /model <model-name>
        const { updateConfig } = await import("./config.js");
        updateConfig({ model: args.trim() });
        config.model = args.trim();
        send({
          type: "agent_message",
          text: `Model set to: ${config.model}`,
        });
      } else {
        const view = buildCliModelView(config);
        send({ type: "agent_message", text: formatView(view) });
      }
      break;
    }

    case "/setting": {
      if (args) {
        const parts = args.trim().split(/\s+/);
        if (parts.length >= 2) {
          const [key, ...rest] = parts;
          const value = rest.join(" ");
          const { updateConfig } = await import("./config.js");
          updateConfig({ [key]: value });
          config[key] = value;
          send({
            type: "agent_message",
            text: `Setting ${key} = ${value}`,
          });
        } else {
          send({
            type: "agent_message",
            text: `Usage: /setting <key> <value>`,
          });
        }
      } else {
        const view = buildCliSettingsView(config, sessionState);
        send({ type: "agent_message", text: formatView(view) });
      }
      break;
    }

    case "/status": {
      const { loadSessionSnapshot } = await import("./session-store.js");
      const persisted = await loadSessionSnapshot().catch(() => null);
      const view = buildCliStatusSnapshot(config, sessionState, persisted);
      send({ type: "agent_message", text: formatView(view) });
      break;
    }

    case "/plan": {
      const { loadSessionSnapshot } = await import("./session-store.js");
      const persisted = await loadSessionSnapshot().catch(() => null);
      const text = buildCliPlanView(sessionState, persisted);
      send({
        type: "agent_message",
        text: text || "No active plan. Start a mission first.",
      });
      break;
    }

    case "/review": {
      const { loadSessionSnapshot } = await import("./session-store.js");
      const persisted = await loadSessionSnapshot().catch(() => null);
      const view = buildCliReviewSnapshot(sessionState, persisted);
      send({ type: "agent_message", text: formatView(view) });
      break;
    }

    case "/transcript":
    case "/log": {
      const { loadSessionSnapshot } = await import("./session-store.js");
      const persisted = await loadSessionSnapshot().catch(() => null);
      const view = buildCliTranscriptView(sessionState, persisted);
      if (view.transcript) {
        // Show last 50 lines to avoid flooding
        const lines = view.transcript.split("\n");
        const tail =
          lines.length > 50
            ? `… (${lines.length - 50} earlier lines)\n` +
              lines.slice(-50).join("\n")
            : view.transcript;
        send({ type: "agent_message", text: tail });
      } else {
        send({
          type: "agent_message",
          text: "No transcript available. Run a mission first.",
        });
      }
      break;
    }

    case "/artifacts": {
      const { loadSessionSnapshot } = await import("./session-store.js");
      const persisted = await loadSessionSnapshot().catch(() => null);
      const view = buildCliArtifactsView(sessionState, persisted);
      send({ type: "agent_message", text: formatView(view) });
      break;
    }

    case "/agents": {
      const { readdirSync } = await import("node:fs");
      const { join } = await import("node:path");
      const { fileURLToPath } = await import("node:url");
      const { dirname } = await import("node:path");
      const agentsDir = join(
        dirname(fileURLToPath(import.meta.url)),
        "../agents",
      );
      try {
        const dirs = readdirSync(agentsDir, { withFileTypes: true })
          .filter((d) => d.isDirectory())
          .map((d) => d.name);
        const { TOOL_REGISTRY } = await import("../agents/executor/tools.js");
        const tools = Object.entries(TOOL_REGISTRY).map(
          ([name, t]) =>
            `  ${name.padEnd(16)} ${(t.description ?? "").slice(0, 60)}`,
        );
        const text = [
          "Agents:",
          ...dirs.map((d) => `  • ${d}`),
          "",
          "Tools:",
          ...tools,
        ].join("\n");
        send({ type: "agent_message", text });
      } catch {
        send({
          type: "agent_message",
          text: "Could not read agents directory.",
        });
      }
      break;
    }

    case "/tools": {
      const { TOOL_REGISTRY } = await import("../agents/executor/tools.js");
      const lines = Object.entries(TOOL_REGISTRY).map(
        ([name, t]) =>
          `  ${name.padEnd(16)} ${(t.description ?? "").slice(0, 80)}`,
      );
      send({ type: "agent_message", text: lines.join("\n") });
      break;
    }

    case "/context": {
      const { getLastUsage } = await import("./api-client.js");
      const usage = getLastUsage();
      const totalTokens = sessionState._totalTokens ?? 0;
      const text = [
        `  Context tokens: ${usage?.prompt_tokens?.toLocaleString() ?? "—"}`,
        `  Last response:  ${usage?.completion_tokens?.toLocaleString() ?? "—"} tokens`,
        `  Session total:  ${totalTokens > 0 ? totalTokens.toLocaleString() : "—"} tokens`,
      ].join("\n");
      send({ type: "agent_message", text });
      break;
    }

    case "/resume": {
      const { loadSessionSnapshot } = await import("./session-store.js");
      const persisted = await loadSessionSnapshot().catch(() => null);
      if (persisted?.resumable) {
        send({
          type: "agent_message",
          text: `Resumable session found:\n  Goal: ${persisted.mission_summary ?? "unknown"}\n  State: ${persisted.last_verification_state ?? "unknown"}\n  Workspace: ${persisted.workspace_path ?? "—"}\n\nType your next instruction to continue.`,
        });
        sessionState.currentTarget = persisted.target_path
          ? { path: persisted.target_path, type: persisted.target_type }
          : null;
        sessionState.currentMission = persisted.mission ?? null;
      } else {
        send({
          type: "agent_message",
          text: "No resumable session found.",
        });
      }
      break;
    }

    case "/demo": {
      if (!args) {
        send({
          type: "agent_message",
          text: "Usage: /demo <scenario-path>\n\nExample: /demo scenarios/docker-copy-path-bug",
        });
      } else {
        send({
          type: "agent_message",
          text: `Demo mode not yet wired in server mode. Run directly:\n  ./bin/decipher demo ${args.trim()}`,
        });
      }
      break;
    }

    case "/doctor": {
      const { checkEnvironment } = await import("../agents/verifier/index.js");
      const result = await checkEnvironment();
      const text = result.items
        .map(
          (item) =>
            `${item.passed ? "✓" : "✗"} ${item.label.padEnd(12)} ${item.version ?? "not found"}`,
        )
        .join("\n");
      send({ type: "agent_message", text });
      break;
    }

    case "/policy": {
      const { PolicyMode } = await import("./exec-policy.js");
      const validModes = Object.values(PolicyMode);
      const currentMode = config.approval_policy_mode ?? PolicyMode.AUTO;

      if (args && args.trim()) {
        const newMode = args.trim().toLowerCase();
        if (validModes.includes(newMode)) {
          config.approval_policy_mode = newMode;
          // Reset amendments when changing policy
          if (sessionState._amendments) {
            const { createAmendments } = await import("./exec-policy.js");
            sessionState._amendments = createAmendments();
          }
          send({
            type: "agent_message",
            text: `Approval policy changed to **${newMode}**.\n\nModes: ${validModes.join(", ")}`,
          });
        } else {
          send({
            type: "agent_message",
            text: `Unknown policy mode: "${newMode}".\nValid modes: ${validModes.join(", ")}`,
          });
        }
      } else {
        const amendments = sessionState._amendments;
        const approvedClasses = amendments?.approvedClasses
          ? [...amendments.approvedClasses].join(", ") || "none"
          : "none";
        send({
          type: "agent_message",
          text:
            `**Approval Policy:** ${currentMode}\n\n` +
            `Modes:\n` +
            `  auto — read=auto, write/exec=ask-once, destructive=always-ask\n` +
            `  read-only — read=auto, all else denied\n` +
            `  granular — read=auto, all else always-ask\n` +
            `  full-access — everything auto-approved\n\n` +
            `Approved classes this session: ${approvedClasses}\n\n` +
            `Usage: /policy <mode>`,
        });
      }
      break;
    }

    case "/compact": {
      send({ type: "spinner", label: "Compacting context", done: false });
      try {
        const { getLastUsage } = await import("./api-client.js");
        const usage = getLastUsage();
        const beforeTokens =
          usage?.total_tokens ?? sessionState._totalTokens ?? 0;

        sessionState.currentTarget = null;
        sessionState.currentMission = null;
        sessionState.currentPlan = null;
        sessionState.lastVerificationResult = null;
        sessionState.lastRunResult = null;
        sessionState._totalTokens = 0;

        send({ type: "spinner", label: "", done: true });
        send({
          type: "agent_message",
          text: `Context compacted. Session state reset.\nPrevious token usage: ${beforeTokens > 0 ? beforeTokens.toLocaleString() : "unknown"} tokens cleared.`,
        });
      } catch (err) {
        send({ type: "spinner", label: "", done: true });
        send({ type: "error", message: `Compaction failed: ${err.message}` });
      }
      break;
    }

    case "/quit":
      process.exit(0);
      break;

    default:
      send({
        type: "agent_message",
        text: `Unknown command: ${name}\nType /help for available commands.`,
      });
  }
}
