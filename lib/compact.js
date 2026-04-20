/**
 * LLM-driven conversation compaction.
 *
 * When the agent's conversation history grows too large, this module
 * calls the LLM with a compaction prompt to produce a concise summary.
 * The summary replaces the middle of the conversation, keeping the
 * initial context and recent messages intact.
 */

import { readFile } from "node:fs/promises";
import { callAI } from "./api-client.js";

const COMPACT_PROMPT_PATH = new URL(
  "../prompts/compact.md",
  import.meta.url,
).pathname;

/** Approximate token count (4 chars per token). */
function estimateTokens(messages) {
  return messages.reduce((sum, m) => sum + (m.content?.length ?? 0), 0) / 4;
}

/**
 * Compact a conversation history using LLM summarization.
 *
 * Keeps the first message (mission context) and last `keepRecent` messages.
 * Summarizes everything in between using the LLM.
 *
 * @param {Array<{role: string, content: string}>} messages
 * @param {object} config — API config (provider, api_key, model)
 * @param {object} [options]
 * @param {number} [options.keepRecent=6] — number of recent messages to preserve
 * @returns {Promise<{messages: Array, beforeTokens: number, afterTokens: number}>}
 */
export async function compactMessages(messages, config, options = {}) {
  const keepRecent = options.keepRecent ?? 6;

  if (messages.length <= keepRecent + 2) {
    // Not enough to compact
    return {
      messages,
      beforeTokens: estimateTokens(messages),
      afterTokens: estimateTokens(messages),
    };
  }

  const beforeTokens = estimateTokens(messages);

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
      "Keep key decisions, findings, and errors. Discard redundant details.";
  }

  const historyText = middle
    .map((m) => `[${m.role}] ${m.content}`)
    .join("\n\n---\n\n");

  const compactionPrompt = `${promptTemplate}\n\n## Conversation to compact (${middle.length} messages)\n\n${historyText}`;

  // Call the LLM to produce a summary
  const summary = await callAI(compactionPrompt, config);

  const compacted = [
    ...keepFirst,
    {
      role: "user",
      content: `[Context compacted — ${middle.length} messages summarized by LLM]\n\n${summary}`,
    },
    ...recent,
  ];

  const afterTokens = estimateTokens(compacted);

  return { messages: compacted, beforeTokens, afterTokens };
}

/**
 * Check if messages should be compacted based on estimated token count.
 * Returns true when the conversation exceeds ~80% of a typical context window.
 *
 * @param {Array<{role: string, content: string}>} messages
 * @param {number} [maxTokens=128000] — model context window size
 * @returns {boolean}
 */
export function shouldCompact(messages, maxTokens = 128_000) {
  const tokens = estimateTokens(messages);
  return tokens > maxTokens * 0.8;
}
