/**
 * Execution policy engine.
 *
 * Evaluates every tool action through a typed policy decision instead of
 * ad hoc prompt rules. Separates policy decisions from execution so a
 * future sandbox transform layer can operate independently.
 *
 * Policy modes:
 *   auto        — read=auto, write/exec=ask-once-per-class, destructive=always-ask
 *   read-only   — read=auto, everything else denied
 *   granular    — read=auto, all other classes always-ask
 *   full-access — everything auto-approved (--trust)
 */

import { resolve } from "node:path";

// ── Tool classes ──────────────────────────────────────────────────────────────

export const ToolClass = Object.freeze({
  READ: "read",
  WRITE: "write",
  EXEC: "exec",
  DESTRUCTIVE: "destructive",
});

// ── Policy modes ──────────────────────────────────────────────────────────────

export const PolicyMode = Object.freeze({
  AUTO: "auto",
  READ_ONLY: "read-only",
  GRANULAR: "granular",
  FULL_ACCESS: "full-access",
});

// ── Policy decisions ──────────────────────────────────────────────────────────

export const Decision = Object.freeze({
  ALLOW: "allow",
  DENY: "deny",
  ASK: "ask",
});

// ── Protected paths ───────────────────────────────────────────────────────────

const PROTECTED_PATH_PATTERNS = [
  /\/\.git(?:\/|$)/,
  /\/\.decipher(?:\/|$)/,
  /^\/etc\//,
  /^\/usr\//,
  /^\/var\//,
  /^\/sys\//,
  /^\/proc\//,
  /\/\.ssh(?:\/|$)/,
  /\/\.aws(?:\/|$)/,
  /\/\.kube(?:\/|$)/,
  /\/\.gnupg(?:\/|$)/,
  /\/\.env$/,
  /\/\.env\./,
];

// ── Destructive command patterns ──────────────────────────────────────────────

const DESTRUCTIVE_CMD_PATTERNS = [
  /\brm\s+(-[^\s]*)?-r/,
  /\brm\s+(-[^\s]*)?-f/,
  /\bgit\s+push\s+--force/,
  /\bgit\s+push\s+-f\b/,
  /\bgit\s+reset\s+--hard/,
  /\bgit\s+clean\s+-[^\s]*f/,
  /\bdocker\s+rm\b/,
  /\bdocker\s+rmi\b/,
  /\bdocker\s+system\s+prune/,
  /\bkubectl\s+delete\b/,
  /\bdrop\s+table\b/i,
  /\bdrop\s+database\b/i,
  /\btruncate\b/i,
  /\bchmod\s+777/,
  /\bchown\s+-R/,
  /\bsudo\b/,
  /\bcurl\s.*\|\s*(?:sh|bash)\b/,
  /\bwget\s.*\|\s*(?:sh|bash)\b/,
  /\beval\b/,
];

// ── Read-only command patterns ────────────────────────────────────────────────

const READ_CMD_PATTERNS = [
  /^\s*(?:cat|head|tail|less|more)\s/,
  /^\s*(?:ls|ll|dir)\s/,
  /^\s*(?:find|fd)\s/,
  /^\s*(?:grep|rg|ag)\s/,
  /^\s*(?:which|where|type)\s/,
  /^\s*(?:echo|printf)\s/,
  /^\s*(?:wc|du|df)\s/,
  /^\s*(?:file|stat)\s/,
  /^\s*(?:docker\s+ps|docker\s+images|docker\s+inspect)\b/,
  /^\s*(?:git\s+status|git\s+log|git\s+diff|git\s+show|git\s+branch)\b/,
  /^\s*(?:node|python|ruby)\s+--version/,
  /^\s*(?:uname|hostname|whoami|id|env|printenv)\b/,
  /^\s*apt-cache\s/,
];

// ── Tool classification ───────────────────────────────────────────────────────

/**
 * Classify a tool invocation into a tool class.
 *
 * @param {string} toolName
 * @param {object} args
 * @returns {{ toolClass: string, paths: string[], reason: string }}
 */
export function classifyToolAction(toolName, args = {}) {
  switch (toolName) {
    case "read_file":
      return {
        toolClass: ToolClass.READ,
        paths: [args.path].filter(Boolean),
        reason: `read ${args.path ?? "file"}`,
      };

    case "write_file":
      return {
        toolClass: ToolClass.WRITE,
        paths: [args.path].filter(Boolean),
        reason: `write ${args.path ?? "file"}`,
      };

    case "apply_patch":
      return {
        toolClass: ToolClass.WRITE,
        paths: [args.target_file].filter(Boolean),
        reason: "apply patch",
      };

    case "exec_command": {
      const cmd = args.cmd ?? "";
      // Check destructive first
      for (const pat of DESTRUCTIVE_CMD_PATTERNS) {
        if (pat.test(cmd)) {
          return {
            toolClass: ToolClass.DESTRUCTIVE,
            paths: extractPathsFromCmd(cmd),
            reason: `destructive: ${cmd.slice(0, 60)}`,
          };
        }
      }
      // Check read-only
      for (const pat of READ_CMD_PATTERNS) {
        if (pat.test(cmd)) {
          return {
            toolClass: ToolClass.READ,
            paths: extractPathsFromCmd(cmd),
            reason: `read-only cmd: ${cmd.slice(0, 60)}`,
          };
        }
      }
      // Default to exec
      return {
        toolClass: ToolClass.EXEC,
        paths: extractPathsFromCmd(cmd),
        reason: `exec: ${cmd.slice(0, 60)}`,
      };
    }

    case "kubectl_get":
    case "kubectl_logs":
    case "kubectl_describe":
    case "kubectl_events":
      return {
        toolClass: ToolClass.READ,
        paths: [],
        reason: `kubectl ${toolName.replace("kubectl_", "")}`,
      };

    case "update_plan":
    case "done":
      return {
        toolClass: ToolClass.READ,
        paths: [],
        reason: toolName,
      };

    default:
      return {
        toolClass: ToolClass.EXEC,
        paths: [],
        reason: `unknown tool: ${toolName}`,
      };
  }
}

// ── Path validation ───────────────────────────────────────────────────────────

/**
 * Check if a path touches a protected location.
 *
 * @param {string} path
 * @param {string[]} [carveouts] — allowed paths that override protection
 * @returns {{ protected: boolean, pattern: string | null }}
 */
export function isProtectedPath(path, carveouts = []) {
  if (!path) return { protected: false, pattern: null };

  // Carveouts override protection
  for (const allowed of carveouts) {
    if (path.startsWith(allowed)) return { protected: false, pattern: null };
  }

  for (const pat of PROTECTED_PATH_PATTERNS) {
    if (pat.test(path)) {
      return { protected: true, pattern: pat.source };
    }
  }
  return { protected: false, pattern: null };
}

/**
 * Check if a path is within the allowed workspace.
 *
 * @param {string} path
 * @param {string} workspace
 * @returns {boolean}
 */
export function isInWorkspace(path, workspace) {
  if (!path || !workspace) return false;
  const resolved = resolve(path);
  const ws = resolve(workspace);
  return resolved.startsWith(ws) || resolved.startsWith("/tmp");
}

// ── Policy evaluation ─────────────────────────────────────────────────────────

/**
 * Permission amendments track per-class approvals within a session.
 * Unlike the old session-wide boolean, approving a write does not
 * imply approval for destructive operations.
 *
 * @typedef {Object} PermissionAmendments
 * @property {Set<string>} approvedClasses — tool classes approved this session
 * @property {Set<string>} approvedTools — specific tools approved this session
 * @property {string[]} pathCarveouts — paths exempted from protection
 */

/**
 * Create a fresh permission amendments tracker.
 * @returns {PermissionAmendments}
 */
export function createAmendments() {
  return {
    approvedClasses: new Set(),
    approvedTools: new Set(),
    pathCarveouts: [],
  };
}

/**
 * Evaluate a tool action against the current policy.
 *
 * @param {string} policyMode — one of PolicyMode values
 * @param {string} toolName
 * @param {object} args
 * @param {PermissionAmendments} amendments
 * @param {string} [workspace] — current workspace path
 * @returns {{ decision: string, toolClass: string, reason: string, protectedPath: string | null }}
 */
export function evaluatePolicy(
  policyMode,
  toolName,
  args,
  amendments,
  workspace,
) {
  const {
    toolClass,
    paths: rawPaths,
    reason,
  } = classifyToolAction(toolName, args);

  // Normalize all paths relative to workspace before any policy check.
  // This prevents "../outside" from bypassing workspace boundaries.
  const paths = rawPaths.map((p) =>
    p.startsWith("/")
      ? resolve(p)
      : workspace
        ? resolve(workspace, p)
        : resolve(p),
  );

  // Build effective carveouts: workspace itself + /tmp + user-defined
  const effectiveCarveouts = [...(amendments.pathCarveouts ?? [])];
  if (workspace) {
    effectiveCarveouts.push(resolve(workspace));
  }
  effectiveCarveouts.push("/tmp");

  // Check protected paths for write/exec/destructive operations
  let protectedPath = null;
  if (toolClass !== ToolClass.READ) {
    for (const p of paths) {
      const check = isProtectedPath(p, effectiveCarveouts);
      if (check.protected) {
        protectedPath = p;
        return {
          decision: Decision.DENY,
          toolClass,
          reason: `protected path: ${p} (${check.pattern})`,
          protectedPath: p,
        };
      }
    }

    // Enforce workspace boundary: non-read operations on paths outside
    // the workspace (and /tmp) are denied regardless of policy mode.
    if (workspace) {
      for (const p of paths) {
        if (!isInWorkspace(p, workspace)) {
          return {
            decision: Decision.DENY,
            toolClass,
            reason: `outside workspace: ${p}`,
            protectedPath: p,
          };
        }
      }
    }
  }

  // Policy mode evaluation
  switch (policyMode) {
    case PolicyMode.FULL_ACCESS:
      return { decision: Decision.ALLOW, toolClass, reason, protectedPath };

    case PolicyMode.READ_ONLY:
      if (toolClass === ToolClass.READ) {
        return { decision: Decision.ALLOW, toolClass, reason, protectedPath };
      }
      return {
        decision: Decision.DENY,
        toolClass,
        reason: `read-only mode: ${reason}`,
        protectedPath,
      };

    case PolicyMode.GRANULAR:
      if (toolClass === ToolClass.READ) {
        return { decision: Decision.ALLOW, toolClass, reason, protectedPath };
      }
      // Always ask for non-read operations in granular mode
      return { decision: Decision.ASK, toolClass, reason, protectedPath };

    case PolicyMode.AUTO:
    default: {
      // Read is always auto-approved
      if (toolClass === ToolClass.READ) {
        return { decision: Decision.ALLOW, toolClass, reason, protectedPath };
      }

      // Destructive always requires confirmation
      if (toolClass === ToolClass.DESTRUCTIVE) {
        return { decision: Decision.ASK, toolClass, reason, protectedPath };
      }

      // Write and exec: ask once per class, then auto-approve
      if (amendments.approvedClasses.has(toolClass)) {
        return { decision: Decision.ALLOW, toolClass, reason, protectedPath };
      }
      if (amendments.approvedTools.has(toolName)) {
        return { decision: Decision.ALLOW, toolClass, reason, protectedPath };
      }

      return { decision: Decision.ASK, toolClass, reason, protectedPath };
    }
  }
}

/**
 * Record an approval for a tool class (ask-once-per-class pattern).
 *
 * @param {PermissionAmendments} amendments
 * @param {string} toolClass
 * @param {string} [toolName] — optionally approve a specific tool
 */
export function recordApproval(amendments, toolClass, toolName) {
  amendments.approvedClasses.add(toolClass);
  if (toolName) {
    amendments.approvedTools.add(toolName);
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function extractPathsFromCmd(cmd) {
  const paths = [];
  // Match absolute paths
  const absMatches = cmd.match(/(?:^|\s)(\/[^\s;|&>]+)/g);
  if (absMatches) {
    for (const m of absMatches) {
      paths.push(m.trim());
    }
  }
  // Match relative paths starting with ./ or ../
  const relMatches = cmd.match(/(?:^|\s)(\.\.?\/[^\s;|&>]+)/g);
  if (relMatches) {
    for (const m of relMatches) {
      paths.push(m.trim());
    }
  }
  return paths;
}

/**
 * Format a policy decision for display.
 *
 * @param {{ decision: string, toolClass: string, reason: string }} result
 * @returns {string}
 */
export function formatPolicyDecision(result) {
  const icon =
    result.decision === Decision.ALLOW
      ? "\u2705"
      : result.decision === Decision.DENY
        ? "\u274C"
        : "\u2753";
  return `${icon} [${result.toolClass}] ${result.reason}`;
}
