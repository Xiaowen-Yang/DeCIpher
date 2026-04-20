/**
 * Token-aware conversation compaction.
 *
 * When the agent's conversation history approaches the model's context limit,
 * this module summarizes the middle of the conversation using the LLM.
 * After compaction, a reference context reminder is injected so the model
 * retains awareness of workspace state and mission progress.
 *
 * V4: Uses actual token counts from API responses instead of char estimation.
 */

import { readFile } from "node:fs/promises";
import { callAI } from "./api-client.js";

const COMPACT_PROMPT_PATH = new URL("../prompts/compact.md", import.meta.url)
  .pathname;

// ── Model context windows ──────��─────────────────────────────────────────────

const MODEL_CONTEXT_WINDOWS = {
  // OpenAI
  "gpt-4o": 128_000,
  "gpt-4o-mini": 128_000,
  "gpt-4-turbo": 128_000,
  "gpt-4-turbo-preview": 128_000,
  "gpt-4": 8_192,
  "gpt-3.5-turbo": 16_385,
  o1: 200_000,
  "o1-mini": 128_000,
  "o1-pro": 200_000,
  o3: 200_000,
  "o3-mini": 200_000,
  "o4-mini": 200_000,
  // Anthropic — dated and alias forms
  "claude-opus-4-6": 200_000,
  "claude-sonnet-4-6": 200_000,
  "claude-sonnet-4-20250514": 200_000,
  "claude-opus-4-20250514": 200_000,
  "claude-haiku-4-5-20251001": 200_000,
  "claude-3-5-sonnet-20241022": 200_000,
  "claude-3-5-haiku-20241022": 200_000,
  "claude-3-opus-20240229": 200_000,
  "claude-sonnet-4-5-20250514": 200_000,
};

const DEFAULT_CONTEXT_WINDOW = 128_000;
const COMPACT_THRESHOLD = 0.75; // Compact at 75% of context window

/**
 * Get the context window size for a model.
 * @param {string} model
 * @returns {number}
 */
export function getContextWindow(model) {
  if (!model) return DEFAULT_CONTEXT_WINDOW;
  // Exact match first
  if (MODEL_CONTEXT_WINDOWS[model]) return MODEL_CONTEXT_WINDOWS[model];
  // Prefix match for versioned model names
  for (const [key, value] of Object.entries(MODEL_CONTEXT_WINDOWS)) {
    if (model.startsWith(key)) return value;
  }
  return DEFAULT_CONTEXT_WINDOW;
}

/**
 * Estimate content length for messages with mixed content types.
 * Used only as a rough fallback when real token counts are unavailable.
 * @param {Array} messages
 * @returns {number} estimated tokens
 */
function estimateTokensFallback(messages) {
  let chars = 0;
  for (const m of messages) {
    if (typeof m.content === "string") {
      chars += m.content.length;
    } else if (Array.isArray(m.content)) {
      for (const block of m.content) {
        if (typeof block === "string") chars += block.length;
        else if (block.text) chars += block.text.length;
        else if (block.content) chars += block.content.length;
        else chars += JSON.stringify(block).length;
      }
    }
  }
  return Math.ceil(chars / 4);
}

/**
 * Determine if compaction is needed based on actual token usage.
 *
 * @param {number} promptTokens — actual prompt tokens from the last API response
 * @param {string} model — model name for context window lookup
 * @returns {boolean}
 */
export function shouldCompact(promptTokens, model) {
  const contextWindow = getContextWindow(model);
  return promptTokens > contextWindow * COMPACT_THRESHOLD;
}

/**
 * Compact a conversation history using LLM summarization.
 *
 * Keeps the first message (mission context) and last `keepRecent` messages.
 * Summarizes everything in between using the LLM. After compaction, injects
 * a reference context reminder so the model retains workspace awareness.
 *
 * @param {Array} messages
 * @param {object} config — API config (provider, api_key, model)
 * @param {object} [options]
 * @param {number} [options.keepRecent=6] — number of recent messages to preserve
 * @param {string} [options.workspaceReminder] — optional workspace state to reinject
 * @returns {Promise<{messages: Array, beforeTokens: number, afterTokens: number}>}
 */
export async function compactMessages(messages, config, options = {}) {
  const keepRecent = options.keepRecent ?? 6;

  if (messages.length <= keepRecent + 2) {
    const tokens = estimateTokensFallback(messages);
    return { messages, beforeTokens: tokens, afterTokens: tokens };
  }

  const beforeTokens = estimateTokensFallback(messages);

  const keepFirst = messages.slice(0, 1);
  const recent = messages.slice(-keepRecent);
  const middle = messages.slice(1, -keepRecent);

  // Build the compaction prompt
  let promptTemplate;
  try {
    promptTemplate = await readFile(COMPACT_PROMPT_PATH, "utf8");
  } catch {
    promptTemplate =
      "Summarize the following conversation history concisely. " +
      "Keep key decisions, findings, errors, and file paths. Discard redundant details.";
  }

  // Serialize middle messages, handling mixed content types
  const historyText = middle
    .map((m) => {
      const content =
        typeof m.content === "string"
          ? m.content
          : Array.isArray(m.content)
            ? m.content
                .map((b) => {
                  if (typeof b === "string") return b;
                  if (b.text) return b.text;
                  if (b.content) return b.content;
                  if (b.type === "tool_use")
                    return `[tool_use: ${b.name}(${JSON.stringify(b.input).slice(0, 200)})]`;
                  if (b.type === "tool_result")
                    return `[tool_result: ${(b.content ?? "").slice(0, 300)}]`;
                  return JSON.stringify(b).slice(0, 200);
                })
                .join("\n")
            : JSON.stringify(m.content).slice(0, 500);
      return `[${m.role}] ${content}`;
    })
    .join("\n\n---\n\n");

  const compactionPrompt = `${promptTemplate}\n\n## Conversation to compact (${middle.length} messages)\n\n${historyText}`;

  const summary = await callAI(compactionPrompt, config);

  const compacted = [
    ...keepFirst,
    {
      role: "user",
      content: `[Context compacted — ${middle.length} messages summarized]\n\n${summary}`,
    },
  ];

  // Reinject workspace state reminder if provided
  if (options.workspaceReminder) {
    compacted.push({
      role: "user",
      content: `[Reference context after compaction]\n\n${options.workspaceReminder}`,
    });
  }

  compacted.push(...recent);

  const afterTokens = estimateTokensFallback(compacted);
  return { messages: compacted, beforeTokens, afterTokens };
}
