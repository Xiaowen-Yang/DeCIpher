/**
 * Tool definitions for native function calling (OpenAI / Anthropic).
 *
 * Converts the agent's TOOL_REGISTRY into the format expected by each
 * provider's native tool-calling API.
 *
 * OpenAI:    tools: [{ type: "function", function: { name, description, parameters } }]
 * Anthropic: tools: [{ name, description, input_schema }]
 */

/**
 * Tool definitions with JSON Schema parameters.
 * These mirror TOOL_REGISTRY in agents/executor/tools.js but use
 * proper JSON Schema instead of the informal argsSchema strings.
 */
const TOOL_DEFINITIONS = [
  {
    name: "exec_command",
    description:
      "Run any shell command. Returns stdout, stderr, and exit code. " +
      "Use this to build images, run tests, install deps, inspect files.",
    parameters: {
      type: "object",
      properties: {
        cmd: { type: "string", description: "The shell command to execute" },
        workdir: {
          type: "string",
          description: "Working directory (defaults to workspace)",
        },
      },
      required: ["cmd"],
    },
  },
  {
    name: "read_file",
    description:
      "Read the full content of a file. " +
      "Path may be absolute or relative to the workspace.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "File path to read" },
      },
      required: ["path"],
    },
  },
  {
    name: "write_file",
    description:
      "Write content to a file, creating it (and parent directories) if needed. " +
      "Replaces the entire file. Requires session approval.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "File path to write" },
        content: { type: "string", description: "File content" },
      },
      required: ["path", "content"],
    },
  },
  {
    name: "apply_patch",
    description:
      "Apply a unified diff to one or more files in the workspace. " +
      "The patch must be a valid unified diff. Requires session approval.",
    parameters: {
      type: "object",
      properties: {
        patch: { type: "string", description: "Unified diff content" },
        target_file: {
          type: "string",
          description: "Target file (overrides patch header)",
        },
      },
      required: ["patch"],
    },
  },
  {
    name: "update_plan",
    description: "Update the displayed plan with current step statuses.",
    parameters: {
      type: "object",
      properties: {
        steps: {
          type: "array",
          items: {
            type: "object",
            properties: {
              step: { type: "string" },
              status: {
                type: "string",
                enum: ["pending", "in_progress", "completed", "failed"],
              },
            },
            required: ["step", "status"],
          },
        },
      },
      required: ["steps"],
    },
  },
  {
    name: "kubectl_get",
    description:
      "Run `kubectl get <resource>` to inspect cluster resources. " +
      "Use output=json for machine-readable details, wide for a human overview.",
    parameters: {
      type: "object",
      properties: {
        resource: {
          type: "string",
          description:
            "Resource type (e.g. pods, deployments, services, nodes)",
        },
        namespace: {
          type: "string",
          description: "Kubernetes namespace (omit for default)",
        },
        output: {
          type: "string",
          enum: ["json", "yaml", "wide", "name"],
          description: "Output format",
        },
        selector: {
          type: "string",
          description: "Label selector (e.g. app=nginx)",
        },
      },
      required: ["resource"],
    },
  },
  {
    name: "kubectl_logs",
    description:
      "Fetch logs from a Kubernetes pod or container. " +
      "Use previous=true for crashed containers, tail to limit output.",
    parameters: {
      type: "object",
      properties: {
        pod: { type: "string", description: "Pod name" },
        namespace: { type: "string", description: "Kubernetes namespace" },
        container: {
          type: "string",
          description: "Container name (if pod has multiple)",
        },
        previous: {
          type: "boolean",
          description: "Fetch logs from previous (crashed) container",
        },
        tail: {
          type: "number",
          description: "Limit to last N lines (default 200)",
        },
      },
      required: ["pod"],
    },
  },
  {
    name: "kubectl_describe",
    description:
      "Run `kubectl describe <resource> <name>` to get detailed status, events, " +
      "and conditions. Essential for diagnosing CrashLoopBackOff, Pending pods, etc.",
    parameters: {
      type: "object",
      properties: {
        resource: {
          type: "string",
          description: "Resource type (e.g. pod, deployment, service)",
        },
        name: { type: "string", description: "Resource name" },
        namespace: { type: "string", description: "Kubernetes namespace" },
      },
      required: ["resource", "name"],
    },
  },
  {
    name: "kubectl_events",
    description:
      "List recent Kubernetes events sorted by timestamp. " +
      "Shows warnings, failures, scheduling decisions, and lifecycle events.",
    parameters: {
      type: "object",
      properties: {
        namespace: {
          type: "string",
          description: "Namespace (omit for all namespaces)",
        },
        field_selector: {
          type: "string",
          description: "Field selector (e.g. involvedObject.name=my-pod)",
        },
      },
      required: [],
    },
  },
  {
    name: "done",
    description:
      "Declare the mission complete. Only call this after verifying " +
      "the user's stated goal is satisfied.",
    parameters: {
      type: "object",
      properties: {
        summary: { type: "string", description: "Summary of what was done" },
        outcome: {
          type: "string",
          enum: ["PASS", "FAIL"],
          description: "Whether the goal was achieved",
        },
      },
      required: ["summary", "outcome"],
    },
  },
];

/**
 * Build tool definitions in OpenAI format.
 * @returns {Array<{type: "function", function: {name, description, parameters}}>}
 */
export function buildOpenAITools() {
  return TOOL_DEFINITIONS.map((t) => ({
    type: "function",
    function: {
      name: t.name,
      description: t.description,
      parameters: t.parameters,
    },
  }));
}

/**
 * Build tool definitions in Anthropic format.
 * @returns {Array<{name, description, input_schema}>}
 */
export function buildAnthropicTools() {
  return TOOL_DEFINITIONS.map((t) => ({
    name: t.name,
    description: t.description,
    input_schema: t.parameters,
  }));
}

/**
 * Build tool definitions for the given provider.
 * @param {"openai"|"anthropic"} provider
 */
export function buildTools(provider) {
  return provider === "anthropic" ? buildAnthropicTools() : buildOpenAITools();
}

export { TOOL_DEFINITIONS };
