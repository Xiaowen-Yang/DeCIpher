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

import { exec as execCb, spawn } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import pc from "picocolors";

const exec = promisify(execCb);
const EXEC_TIMEOUT = 120_000; // 2 min per command

// ── Docker resource tracking ─────────────────────────────────────────────────
// Track containers and images created during a mission so they can be cleaned
// up on failure. Prevents polluting the user's Docker environment.

/**
 * Extract Docker resource names from a command string.
 * Returns { type, name } or null.
 */
function parseDockerResource(cmd) {
  const parts = (cmd ?? "").trim();

  // docker build -t <name> ...
  const buildTag = parts.match(/docker\s+build\s+.*?-t\s+(\S+)/);
  if (buildTag) return { type: "image", name: buildTag[1] };

  // docker run --name <name> ...
  const runName = parts.match(/docker\s+run\s+.*?--name\s+(\S+)/);
  if (runName) return { type: "container", name: runName[1] };

  // docker run (without --name) — returns container ID in stdout
  if (/docker\s+run\b/.test(parts)) return { type: "container_from_run" };

  // docker compose / docker-compose up
  if (/docker[\s-]compose\s+up\b/.test(parts)) return { type: "compose" };

  return null;
}

/**
 * After an exec_command returns, extract any container IDs from the output.
 */
function extractContainerIds(output) {
  // Docker run -d prints a 64-char hex container ID
  const ids = [];
  for (const line of (output ?? "").split("\n")) {
    const trimmed = line.trim();
    if (/^[0-9a-f]{12,64}$/.test(trimmed)) {
      ids.push(trimmed);
    }
  }
  return ids;
}

/**
 * Clean up Docker resources created during a failed mission.
 * @param {Set<string>} containers - container names/IDs
 * @param {Set<string>} images - image names/tags
 * @param {string|null} composeDir - directory where docker-compose was run
 * @param {Function} log
 */
export async function cleanupDockerResources(
  containers,
  images,
  composeDir,
  log,
) {
  const cleaned = { containers: [], images: [], compose: false };

  // 1. Stop and remove containers
  for (const c of containers) {
    try {
      await exec(`docker rm -f ${c}`, { timeout: 15_000 });
      cleaned.containers.push(c);
      log(pc.dim(`  [cleanup] removed container: ${c}`));
    } catch {
      // Container may already be gone
    }
  }

  // 2. Tear down docker-compose stack
  if (composeDir) {
    try {
      await exec("docker compose down --remove-orphans -v", {
        cwd: composeDir,
        timeout: 30_000,
      });
      cleaned.compose = true;
      log(pc.dim("  [cleanup] docker compose down"));
    } catch {
      try {
        await exec("docker-compose down --remove-orphans -v", {
          cwd: composeDir,
          timeout: 30_000,
        });
        cleaned.compose = true;
        log(pc.dim("  [cleanup] docker-compose down"));
      } catch {
        // compose not available or already torn down
      }
    }
  }

  // 3. Remove images
  for (const img of images) {
    try {
      await exec(`docker rmi ${img}`, { timeout: 15_000 });
      cleaned.images.push(img);
      log(pc.dim(`  [cleanup] removed image: ${img}`));
    } catch {
      // Image may be in use or already gone
    }
  }

  return cleaned;
}

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

/**
 * Execute a shell command with real-time output streaming.
 * Calls `onOutput(chunk)` for each piece of stdout/stderr.
 * Falls back to safeExec if no onOutput callback provided.
 */
async function safeExecStreaming(
  cmd,
  workdir,
  onOutput,
  timeout = EXEC_TIMEOUT,
) {
  if (!onOutput) return safeExec(cmd, workdir, timeout);

  return new Promise((resolve) => {
    const child = spawn("sh", ["-c", cmd], {
      cwd: workdir,
      timeout,
      stdio: ["ignore", "pipe", "pipe"],
    });

    let output = "";

    child.stdout.on("data", (chunk) => {
      const text = chunk.toString();
      output += text;
      onOutput(text);
    });

    child.stderr.on("data", (chunk) => {
      const text = chunk.toString();
      output += text;
      onOutput(text);
    });

    child.on("error", (err) => {
      output += `\n${err.message}`;
      resolve({ exitCode: 1, output: output.trim() });
    });

    child.on("close", (code) => {
      resolve({ exitCode: code ?? 0, output: output.trim() });
    });
  });
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
      const result = await safeExecStreaming(
        cmd,
        workdir,
        context.onExecOutput ?? null,
      );

      // Track Docker resources for cleanup on failure
      const resource = parseDockerResource(cmd);
      if (resource && context.sessionState) {
        const st = context.sessionState;
        if (!st._dockerContainers) st._dockerContainers = new Set();
        if (!st._dockerImages) st._dockerImages = new Set();

        if (resource.type === "image" && resource.name) {
          st._dockerImages.add(resource.name);
        } else if (resource.type === "container" && resource.name) {
          st._dockerContainers.add(resource.name);
        } else if (resource.type === "container_from_run") {
          // Extract container ID from stdout (docker run -d prints it)
          for (const id of extractContainerIds(result.output)) {
            st._dockerContainers.add(id);
          }
        } else if (resource.type === "compose") {
          st._dockerComposeDir = workdir;
        }
      }

      return result;
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

  // ── Kubernetes tools ────────────────────────────────────────────────────────

  kubectl_get: {
    description:
      "Run `kubectl get <resource>` to inspect cluster resources. " +
      "Use output=json for machine-readable details, wide for a human overview. " +
      "Examples: resource=pods, resource=deployments, resource=services, resource=nodes. " +
      "Optionally filter by namespace or label selector.",
    argsSchema:
      '{ resource: string, namespace?: string, output?: "json"|"yaml"|"wide"|"name", selector?: string }',
    riskyByDefault: false,
    handler: async (args, context) => {
      const resource = args.resource ?? "pods";
      const ns = args.namespace ? `-n ${args.namespace}` : "";
      const out = args.output ? `-o ${args.output}` : "";
      const sel = args.selector ? `-l ${args.selector}` : "";
      const cmd = `kubectl get ${resource} ${ns} ${out} ${sel}`
        .replace(/\s+/g, " ")
        .trim();
      const result = await safeExec(cmd, context.workspace);
      return {
        success: result.exitCode === 0,
        output: result.output,
        command: cmd,
        exitCode: result.exitCode,
      };
    },
  },

  kubectl_logs: {
    description:
      "Fetch logs from a Kubernetes pod (or container within a pod). " +
      "Use previous=true to fetch logs from a crashed container. " +
      "Use tail to limit output to the last N lines.",
    argsSchema:
      "{ pod: string, namespace?: string, container?: string, previous?: boolean, tail?: number }",
    riskyByDefault: false,
    handler: async (args, context) => {
      const pod = args.pod ?? "";
      if (!pod) return { success: false, error: "pod name is required" };
      const ns = args.namespace ? `-n ${args.namespace}` : "";
      const container = args.container ? `-c ${args.container}` : "";
      const prev = args.previous ? "--previous" : "";
      const tail = args.tail ? `--tail=${args.tail}` : "--tail=200";
      const cmd = `kubectl logs ${pod} ${ns} ${container} ${prev} ${tail}`
        .replace(/\s+/g, " ")
        .trim();
      const result = await safeExec(cmd, context.workspace);
      return {
        success: result.exitCode === 0,
        output: result.output,
        command: cmd,
        exitCode: result.exitCode,
      };
    },
  },

  kubectl_describe: {
    description:
      "Run `kubectl describe <resource> <name>` to get detailed information " +
      "about a Kubernetes resource including events, conditions, and status. " +
      "Essential for diagnosing CrashLoopBackOff, OOMKilled, Pending pods, etc.",
    argsSchema: "{ resource: string, name: string, namespace?: string }",
    riskyByDefault: false,
    handler: async (args, context) => {
      const resource = args.resource ?? "pod";
      const name = args.name ?? "";
      if (!name) return { success: false, error: "resource name is required" };
      const ns = args.namespace ? `-n ${args.namespace}` : "";
      const cmd = `kubectl describe ${resource} ${name} ${ns}`
        .replace(/\s+/g, " ")
        .trim();
      const result = await safeExec(cmd, context.workspace);
      return {
        success: result.exitCode === 0,
        output: result.output,
        command: cmd,
        exitCode: result.exitCode,
      };
    },
  },

  kubectl_events: {
    description:
      "List recent Kubernetes events, sorted by timestamp. " +
      "Invaluable for diagnosing cluster issues — shows warnings, failures, " +
      "scheduling decisions, and resource lifecycle events.",
    argsSchema: "{ namespace?: string, field_selector?: string }",
    riskyByDefault: false,
    handler: async (args, context) => {
      const ns = args.namespace ? `-n ${args.namespace}` : "--all-namespaces";
      const fs = args.field_selector
        ? `--field-selector=${args.field_selector}`
        : "";
      const cmd = `kubectl get events ${ns} ${fs} --sort-by=.lastTimestamp`
        .replace(/\s+/g, " ")
        .trim();
      const result = await safeExec(cmd, context.workspace);
      return {
        success: result.exitCode === 0,
        output: result.output,
        command: cmd,
        exitCode: result.exitCode,
      };
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
