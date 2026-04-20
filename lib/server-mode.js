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

  // Override the approval function to send JSON instead of readline prompt
  const originalAskApproval = async (_rl, state, action = null) => {
    if (
      state.approvalPolicy === "never" ||
      state.approvalPolicy === "on-failure"
    ) {
      state.approved = true;
      return true;
    }
    if (state.approved) return true;

    // Send approval request to TUI
    send({
      type: "approval_request",
      capabilities: [
        "read — read files, logs, and Dockerfiles",
        "fix — propose and apply patches",
        "run — execute commands (docker build, grep, etc.)",
        "retry — attempt up to 3 fix iterations",
        "write — create or modify files",
      ],
      action: action
        ? {
            tool: action.tool,
            reasoning: action.reasoning ?? null,
          }
        : null,
    });

    // Wait for approval response from TUI
    const approved = await new Promise((resolve) => {
      approvalResolver = resolve;
    });

    state.approved = approved;
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
        await handleUserInput(msg.text, config, sessionState);
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

async function handleUserInput(input, config, sessionState) {
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

  // Get plan
  const { createMissionPlan } = await import("../agents/planner/index.js");
  const plan = createMissionPlan(sessionState.currentMission);
  sessionState.currentPlan = plan;

  const displaySteps =
    plan.steps?.length > 0 ? plan.steps.map((s) => s.label) : analysis.steps;

  // Send mission to TUI
  send({
    type: "mission",
    understood: missionGoal,
    target: target?.path ?? null,
    target_type: target?.type ?? null,
    steps: displaySteps,
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
    const result = await executeTarget(
      target,
      analysis.action,
      config,
      sessionState,
      {
        onToolStart: (tool, reasoning) =>
          send({ type: "tool_start", tool, reasoning }),
        onToolResult: (tool, success, summary, elapsedMs) =>
          send({
            type: "tool_result",
            tool,
            success,
            summary,
            elapsed_ms: elapsedMs,
          }),
        onDelta: (() => {
          // Stream reasoning text to TUI as it arrives.
          // The agent responds with JSON: {"reasoning": "...", "tool": "...", ...}
          // We extract and forward only the reasoning field for live display.
          let buf = "";
          let inReasoning = false;
          let reasoningDone = false;
          return (delta) => {
            if (reasoningDone) return;
            buf += delta;
            if (!inReasoning) {
              const idx = buf.indexOf('"reasoning"');
              if (idx === -1) return;
              // Find the opening quote of the value
              const valStart = buf.indexOf('"', idx + 11);
              if (valStart === -1) return;
              inReasoning = true;
              buf = buf.slice(valStart + 1);
            }
            // Stream characters until we hit the closing unescaped quote
            let out = "";
            let i = 0;
            while (i < buf.length) {
              if (buf[i] === "\\" && i + 1 < buf.length) {
                out += buf[i + 1] === "n" ? "\n" : buf[i + 1];
                i += 2;
                continue;
              }
              if (buf[i] === '"') {
                reasoningDone = true;
                break;
              }
              out += buf[i];
              i++;
            }
            buf = buf.slice(i + 1);
            if (out) {
              send({ type: "agent_message_delta", delta: out });
            }
          };
        })(),
        onExecOutput: (chunk) =>
          send({ type: "exec_output_delta", delta: chunk }),
      },
    );

    // Emit token usage if available
    const { getLastUsage } = await import("./api-client.js");
    const usage = getLastUsage();
    if (usage) {
      send({
        type: "token_usage",
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
      });
    }

    // ALWAYS send mission_complete — never leave the TUI spinner idle.
    if (result) {
      const urls = result.summary?.match(/https?:\/\/[^\s,)>"']+/g) ?? [];
      send({
        type: "mission_complete",
        outcome: result.outcome ?? "FAIL",
        summary: result.summary ?? "Mission complete.",
        turns: result.iterations ?? 0,
        elapsed_ms: result.elapsedMs ?? 0,
        urls,
      });
    } else {
      send({
        type: "mission_complete",
        outcome: "FAIL",
        summary: "Agent returned no result.",
        turns: 0,
        elapsed_ms: 0,
        urls: [],
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
