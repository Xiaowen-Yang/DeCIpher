/**
 * Tool registry for the agent-driven execution loop.
 *
 * Tools are the primitives the LLM agent calls at each turn.
 * Each tool has a name, description (for the prompt), args schema,
 * a risky flag, and an async handler(args, context) → result.
 *
 * context shape:
 *   { workspace: string, sessionState: object, config: object,
 *     log: function, onPlanUpdate: function }
 */

import { exec as execCb } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const exec = promisify(execCb);
const EXEC_TIMEOUT = 120_000; // 2 min per command

// ── Risky command detection ────────────────────────────────────────────────────

const RISKY_PREFIXES = [
  ["rm", "-rf"],
  ["rm", "-r"],
  ["sudo", "rm"],
  ["git", "push", "--force"],
  ["git", "reset", "--hard"],
  ["kubectl", "delete"],
  ["terraform", "destroy"],
  ["docker", "system", "prune"],
  ["mkfs"],
  ["dd"],
];

export function isRiskyCommand(cmd) {
  const parts = (cmd ?? "").trim().toLowerCase().split(/\s+/);
  return RISKY_PREFIXES.some((prefix) =>
    prefix.every((p, i) => parts[i] === p),
  );
}

// ── Shell helper ───────────────────────────────────────────────────────────────

async function safeExec(cmd, workdir, timeout = EXEC_TIMEOUT) {
  try {
    const { stdout, stderr } = await exec(cmd, { cwd: workdir, timeout });
    return { exitCode: 0, output: (stdout + stderr).trim() };
  } catch (err) {
    return {
      exitCode: err.code ?? 1,
      output: (
        (err.stdout ?? "") +
        (err.stderr ?? "") +
        "\n" +
        err.message
      ).trim(),
    };
  }
}

// ── Tool registry ──────────────────────────────────────────────────────────────

export const TOOL_REGISTRY = {
  exec_command: {
    description:
      "Run any shell command. Returns stdout, stderr, and exit code. " +
      "Use this to build images, run tests, install deps, inspect files — anything shell-based.",
    argsSchema: "{ cmd: string, workdir?: string }",
    riskyByDefault: false, // evaluated per-command via isRiskyCommand
    handler: async (args, context) => {
      const cmd = args.cmd ?? "";
      const workdir = args.workdir ? resolve(args.workdir) : context.workspace;
      return safeExec(cmd, workdir);
    },
  },

  read_file: {
    description:
      "Read the full content of a file. " +
      "Path may be absolute or relative to the workspace.",
    argsSchema: "{ path: string }",
    riskyByDefault: false,
    handler: async (args, context) => {
      const p = resolve(context.workspace, args.path ?? "");
      try {
        const content = await readFile(p, "utf8");
        return { success: true, content, path: p };
      } catch (err) {
        return { success: false, error: err.message, path: p };
      }
    },
  },

  write_file: {
    description:
      "Write content to a file, creating it (and any parent directories) if needed. " +
      "This replaces the entire file. Requires session approval.",
    argsSchema: "{ path: string, content: string }",
    riskyByDefault: true,
    handler: async (args, context) => {
      const p = resolve(context.workspace, args.path ?? "");
      try {
        // Capture original content in result so the session transcript has
        // rollback info. The agent or user can restore from this if needed.
        let previous = null;
        try {
          previous = await readFile(p, "utf8");
        } catch {
          // File doesn't exist yet — that's fine
        }
        await mkdir(dirname(p), { recursive: true });
        await writeFile(p, args.content ?? "", "utf8");
        return {
          success: true,
          path: p,
          previous_existed: previous !== null,
          previous_length: previous?.length ?? 0,
        };
      } catch (err) {
        return { success: false, error: err.message, path: p };
      }
    },
  },

  apply_patch: {
    description:
      "Apply a unified diff to one or more files in the workspace. " +
      "The patch must be a valid unified diff (--- a/file, +++ b/file headers). " +
      "Requires session approval.",
    argsSchema: "{ patch: string, target_file?: string }",
    riskyByDefault: true,
    handler: async (args, context) => {
      const patch = args.patch ?? "";
      if (!patch.trim()) {
        return { success: false, error: "Empty patch" };
      }

      // Determine target file from explicit arg or patch header
      let targetPath = args.target_file
        ? resolve(context.workspace, args.target_file)
        : null;

      if (!targetPath) {
        const match = patch.match(/^\+\+\+ b\/(.+)$/m);
        if (!match) {
          return {
            success: false,
            error: "Cannot determine target file from patch header",
          };
        }
        targetPath = resolve(context.workspace, match[1].trim());
      }

      try {
        const { applyPatch } = await import("../verifier/index.js");
        await applyPatch(patch, targetPath);
        return { success: true, applied_to: targetPath };
      } catch (err) {
        return { success: false, error: err.message, target: targetPath };
      }
    },
  },

  update_plan: {
    description:
      "Update the displayed plan with current step statuses. " +
      "Use this to communicate progress to the user.",
    argsSchema:
      '{ steps: [{ step: string, status: "pending"|"in_progress"|"completed"|"failed" }] }',
    riskyByDefault: false,
    handler: async (args, context) => {
      const steps = args.steps ?? [];
      if (context.onPlanUpdate) {
        context.onPlanUpdate(steps);
      }
      return { success: true, steps };
    },
  },

  done: {
    description:
      "Declare the mission complete. " +
      "Only call this after you have verified the user's stated goal is satisfied. " +
      "Set outcome to PASS if the goal was achieved, FAIL if it could not be.",
    argsSchema: '{ summary: string, outcome: "PASS"|"FAIL" }',
    riskyByDefault: false,
    handler: async (args) => {
      return {
        summary: args.summary ?? "Mission complete.",
        outcome: args.outcome === "FAIL" ? "FAIL" : "PASS",
      };
    },
  },
};

// ── Prompt helpers ─────────────────────────────────────────────────────────────

/**
 * Returns the tools section text for inclusion in the agent system prompt.
 */
export function toolsPromptSection() {
  return Object.entries(TOOL_REGISTRY)
    .map(
      ([name, t]) => `### ${name}\n${t.description}\nArgs: \`${t.argsSchema}\``,
    )
    .join("\n\n");
}

/**
 * Returns true if this tool+args combination requires elevated approval.
 */
export function isToolRisky(toolName, args = {}) {
  const tool = TOOL_REGISTRY[toolName];
  if (!tool) return false;
  if (tool.riskyByDefault) return true;
  if (toolName === "exec_command") return isRiskyCommand(args.cmd ?? "");
  return false;
}
