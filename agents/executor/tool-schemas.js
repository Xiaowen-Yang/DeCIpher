/**
 * JSON Schema tool definitions for native function calling.
 *
 * Provides tool schemas in both OpenAI and Anthropic format.
 * Used by callAIWithToolsStreaming() to send tools alongside messages
 * instead of embedding tool descriptions in the system prompt.
 */

// ── Tool definitions (provider-agnostic) ────────────────────────────────────

const TOOL_DEFINITIONS = {
  exec_command: {
    description:
      "Run any shell command. Returns stdout, stderr, and exit code. " +
      "Use this to build images, run tests, install deps, inspect files.",
    parameters: {
      type: "object",
      properties: {
        cmd: {
          type: "string",
          description: "The shell command to execute",
        },
        workdir: {
          type: "string",
          description:
            "Optional working directory override (default: workspace)",
        },
      },
      required: ["cmd"],
    },
  },

  read_file: {
    description:
      "Read the full content of a file. Path may be absolute or relative to workspace.",
    parameters: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "File path (absolute or relative to workspace)",
        },
      },
      required: ["path"],
    },
  },

  write_file: {
    description:
      "Write content to a file, creating it and parent directories if needed. " +
      "Replaces entire file content. Requires approval.",
    parameters: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "File path (absolute or relative to workspace)",
        },
        content: {
          type: "string",
          description: "The complete file content to write",
        },
      },
      required: ["path", "content"],
    },
  },

  apply_patch: {
    description:
      "Apply a unified diff to files in the workspace. " +
      "Must be valid unified diff format (--- a/file, +++ b/file). " +
      "Prefer write_file for small config files. Requires approval.",
    parameters: {
      type: "object",
      properties: {
        patch: {
          type: "string",
          description: "Unified diff content",
        },
        target_file: {
          type: "string",
          description: "Target file path (if not in patch headers)",
        },
      },
      required: ["patch"],
    },
  },

  kubectl_get: {
    description:
      "Run kubectl get to inspect cluster resources. " +
      "Use output=json for machine-readable, wide for human overview.",
    parameters: {
      type: "object",
      properties: {
        resource: {
          type: "string",
          description:
            "Resource type: pods, deployments, services, nodes, etc.",
        },
        namespace: { type: "string", description: "Kubernetes namespace" },
        output: {
          type: "string",
          enum: ["json", "yaml", "wide", "name"],
          description: "Output format",
        },
        selector: { type: "string", description: "Label selector" },
      },
      required: ["resource"],
    },
  },

  kubectl_logs: {
    description:
      "Fetch logs from a Kubernetes pod. Use previous=true for crashed containers.",
    parameters: {
      type: "object",
      properties: {
        pod: { type: "string", description: "Pod name" },
        namespace: { type: "string", description: "Kubernetes namespace" },
        container: { type: "string", description: "Container name in pod" },
        previous: {
          type: "boolean",
          description: "Fetch logs from previous instance",
        },
        tail: {
          type: "integer",
          description: "Number of lines from end (default 200)",
        },
      },
      required: ["pod"],
    },
  },

  kubectl_describe: {
    description:
      "Run kubectl describe for detailed resource information including events and conditions.",
    parameters: {
      type: "object",
      properties: {
        resource: {
          type: "string",
          description: "Resource type (pod, deployment, etc.)",
        },
        name: { type: "string", description: "Resource name" },
        namespace: { type: "string", description: "Kubernetes namespace" },
      },
      required: ["resource", "name"],
    },
  },

  kubectl_events: {
    description:
      "List recent Kubernetes events sorted by timestamp. Shows warnings, failures, scheduling.",
    parameters: {
      type: "object",
      properties: {
        namespace: { type: "string", description: "Kubernetes namespace" },
        field_selector: {
          type: "string",
          description: "Field selector filter",
        },
      },
    },
  },

  update_plan: {
    description: "Update the displayed plan with current step statuses.",
    parameters: {
      type: "object",
      properties: {
        steps: {
          type: "array",
          items: {
            type: "object",
            properties: {
              step: { type: "string", description: "Step description" },
              status: {
                type: "string",
                enum: ["pending", "in_progress", "completed", "failed"],
              },
            },
            required: ["step", "status"],
          },
          description: "Plan steps with their current status",
        },
      },
      required: ["steps"],
    },
  },

  done: {
    description:
      "Declare the mission complete. Provide a detailed summary. " +
      "PASS = goal achieved, FAIL = could not achieve, PARTIAL = some steps succeeded.",
    parameters: {
      type: "object",
      properties: {
        summary: {
          type: "string",
          description: "Detailed summary of what was accomplished",
        },
        outcome: {
          type: "string",
          enum: ["PASS", "FAIL", "PARTIAL"],
          description: "Mission outcome",
        },
        files_modified: {
          type: "array",
          items: { type: "string" },
          description: "List of file paths that were modified",
        },
        errors_encountered: {
          type: "array",
          items: { type: "string" },
          description: "List of error descriptions if any",
        },
        next_steps: {
          type: "array",
          items: { type: "string" },
          description: "Suggestions for follow-up if FAIL/PARTIAL",
        },
      },
      required: ["summary", "outcome"],
    },
  },
};

// ── Provider-specific formatters ────────────────────────────────────────────

/**
 * Build tool definitions in OpenAI format.
 * @returns {Array} tools array for OpenAI API
 */
export function buildOpenAITools() {
  return Object.entries(TOOL_DEFINITIONS).map(([name, def]) => ({
    type: "function",
    function: {
      name,
      description: def.description,
      parameters: def.parameters,
    },
  }));
}

/**
 * Build tool definitions in Anthropic format.
 * @returns {Array} tools array for Anthropic API
 */
export function buildAnthropicTools() {
  return Object.entries(TOOL_DEFINITIONS).map(([name, def]) => ({
    name,
    description: def.description,
    input_schema: def.parameters,
  }));
}

/**
 * Build tools for the given provider.
 * @param {"openai"|"anthropic"|string} provider
 * @returns {Array}
 */
export function buildToolsForProvider(provider) {
  return provider === "anthropic"
    ? buildAnthropicTools()
    : buildOpenAITools();
}

// ── Message formatting helpers ──────────────────────────────────────────────
// These abstract provider-specific message formats so the agent loop
// doesn't need to know about OpenAI vs Anthropic message structure.

/**
 * Format the assistant's tool-calling response as a message for the history.
 * @param {"openai"|"anthropic"|string} provider
 * @param {Array<{id: string, name: string, input: object}>} toolCalls
 * @param {string|null} textContent - any text content alongside tool calls
 * @returns {object} message object to push to messages array
 */
export function formatAssistantToolCallMessage(
  provider,
  toolCalls,
  textContent,
) {
  if (provider === "anthropic") {
    const content = [];
    if (textContent) {
      content.push({ type: "text", text: textContent });
    }
    for (const tc of toolCalls) {
      content.push({
        type: "tool_use",
        id: tc.id,
        name: tc.name,
        input: tc.input,
      });
    }
    return { role: "assistant", content };
  }

  // OpenAI format
  return {
    role: "assistant",
    content: textContent ?? null,
    tool_calls: toolCalls.map((tc) => ({
      id: tc.id,
      type: "function",
      function: {
        name: tc.name,
        arguments: JSON.stringify(tc.input),
      },
    })),
  };
}

/**
 * Format a tool result as a message for the history.
 * @param {"openai"|"anthropic"|string} provider
 * @param {string} toolCallId - the tool call ID to correlate
 * @param {string} content - the result text
 * @param {boolean} isError - whether the result is an error
 * @returns {object} message object to push to messages array
 */
export function formatToolResultMessage(
  provider,
  toolCallId,
  content,
  isError = false,
) {
  if (provider === "anthropic") {
    return {
      role: "user",
      content: [
        {
          type: "tool_result",
          tool_use_id: toolCallId,
          content,
          ...(isError ? { is_error: true } : {}),
        },
      ],
    };
  }

  // OpenAI format
  return {
    role: "tool",
    tool_call_id: toolCallId,
    content,
  };
}

/**
 * Format a plain text user message.
 * Works the same for both providers.
 */
export function formatUserMessage(content) {
  return { role: "user", content };
}

/**
 * Format a plain text assistant message (for non-tool responses).
 * Works the same for both providers.
 */
export function formatAssistantMessage(content) {
  return { role: "assistant", content };
}

export { TOOL_DEFINITIONS };
