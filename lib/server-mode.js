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

    if (result) {
      // Extract URLs from summary
      const urls = result.summary?.match(/https?:\/\/[^\s,)>"']+/g) ?? [];

      send({
        type: "mission_complete",
        outcome: result.outcome ?? "FAIL",
        summary: result.summary ?? "Mission complete.",
        turns: result.iterations ?? 0,
        elapsed_ms: result.elapsedMs ?? 0,
        urls,
      });
    }
  } catch (err) {
    send({ type: "error", message: err.message });
  }
}

async function handleSlashCommand(name, args, config, sessionState) {
  // Route known slash commands
  switch (name) {
    case "/help":
      send({
        type: "agent_message",
        text: "Available commands: /help, /model, /setting, /status, /plan, /review, /resume, /transcript, /artifacts, /doctor, /agents, /quit",
      });
      break;

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

    case "/quit":
      process.exit(0);
      break;

    default:
      send({
        type: "agent_message",
        text: `Command ${name} is not yet supported in server mode.`,
      });
  }
}
