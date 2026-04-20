/**
 * AI API client supporting OpenAI and Anthropic providers.
 * Reads provider + api_key + model from config passed in.
 */

const OPENAI_URL = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_URL = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION = "2023-06-01";
// Thinking models (glm-4, deepseek-r1, o1, …) can take 60-120s for a response.
const TIMEOUT_MS = 120_000;

// ── HTTP connection reuse ───────────────────────────────────────────────────
// Node.js 22+ has global dispatcher with keep-alive by default.
// Explicitly set for older versions.
import { Agent } from "node:http";
import { Agent as HttpsAgent } from "node:https";
const httpAgent = new Agent({ keepAlive: true });
const httpsAgent = new HttpsAgent({ keepAlive: true });

// ── Retry logic ──────────────────────────────────────────────────────────────

const MAX_RETRIES = 3;

const RETRYABLE_STATUS_CODES = new Set([408, 429, 500, 502, 503, 504]);

function isRetryableError(err) {
  if (err.name === "AbortError") return true; // timeout
  const statusMatch = err.message?.match(/error (\d+)/i);
  if (statusMatch && RETRYABLE_STATUS_CODES.has(Number(statusMatch[1]))) {
    return true;
  }
  if (/ECONNRESET|ECONNREFUSED|ETIMEDOUT|fetch failed/i.test(err.message)) {
    return true;
  }
  return false;
}

function backoffDelay(attempt, retryAfterSec = null) {
  if (retryAfterSec !== null && retryAfterSec > 0) {
    return Math.min(retryAfterSec * 1000, 60_000);
  }
  return Math.min(Math.pow(2, attempt) * 1000, 30_000);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

/**
 * Wrap an async API call function with retry + exponential backoff.
 * Only retries on transient errors (timeout, 429, 5xx, network).
 * Terminal errors (auth, invalid request) fail immediately.
 */
async function withRetry(fn, label = "API call") {
  let lastError;
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    try {
      return await fn();
    } catch (err) {
      lastError = err;
      if (attempt < MAX_RETRIES && isRetryableError(err)) {
        // Respect Retry-After header if attached to the error
        const retryAfter = err.retryAfterSec ?? null;
        const delay = backoffDelay(attempt, retryAfter);
        process.stderr.write(
          `  [retry] ${label} failed (${err.message}), retrying in ${delay / 1000}s (${attempt + 1}/${MAX_RETRIES})\n`,
        );
        await sleep(delay);
        continue;
      }
      throw err;
    }
  }
  throw lastError;
}

// ── Token usage tracking ────────────────────────────────────────────────────
// Last API call's usage is stored here so callers can emit it.
let _lastUsage = null;

/**
 * Get the token usage from the last API call.
 * @returns {{ prompt_tokens: number, completion_tokens: number, total_tokens: number } | null}
 */
export function getLastUsage() {
  const u = _lastUsage;
  _lastUsage = null;
  return u;
}

export { isRetryableError, backoffDelay };

// ── Thinking / reasoning support ────────────────────────────────────────────

/**
 * Model name patterns that indicate thinking/reasoning capability.
 * Used for auto-enabling thinking when no explicit reasoning_effort is set.
 */
const THINKING_MODEL_PATTERNS = [
  /^o1/i, // OpenAI o1-series
  /^o3/i, // OpenAI o3-series
  /^o4/i, // OpenAI o4-series
  /deepseek-r/i, // DeepSeek R-series
  /glm-z/i, // ZhipuAI Z-series (thinking by default)
  /glm-4-thinking/i, // ZhipuAI explicit thinking models
  /glm-5/i, // ZhipuAI GLM-5 series
  /claude-3-7/i, // Anthropic extended thinking
  /claude-4/i, // Anthropic claude-4 series
];

/** Returns true if the model name suggests built-in thinking/reasoning support. */
function supportsThinking(model) {
  return THINKING_MODEL_PATTERNS.some((p) => p.test(model ?? ""));
}

/**
 * Resolve the effective reasoning effort for a config.
 * Explicit config wins; otherwise auto-detect from model name.
 */
function resolveReasoningEffort(config) {
  if (config.reasoning_effort) return config.reasoning_effort;
  if (supportsThinking(config.model)) return "medium";
  return null;
}

/**
 * Build provider-specific reasoning / thinking params.
 * @param {"low"|"medium"|"high"|null|undefined} effort
 * @param {"openai"|"anthropic"|string} provider
 * @param {string} [model] — used to pick ZhipuAI-style params
 * @returns {object} — merge into the request body
 */
function buildReasoningParams(effort, provider, model = "") {
  if (!effort) return {};
  if (provider === "anthropic") {
    // Anthropic extended thinking: budget_tokens maps to effort level
    const budgetMap = { low: 1024, medium: 4096, high: 16384 };
    return {
      thinking: {
        type: "enabled",
        budget_tokens: budgetMap[effort] ?? 4096,
      },
    };
  }
  // ZhipuAI GLM models use enable_thinking
  if (/glm/i.test(model)) {
    return { enable_thinking: true };
  }
  // OpenAI-compatible: reasoning_effort param (o-series models)
  return { reasoning_effort: effort };
}

/** Create an API error with Retry-After header attached if present. */
function apiError(response, body) {
  const err = new Error(
    `${response.url.includes("anthropic") ? "Anthropic" : "OpenAI"} API error ${response.status}: ${body}`,
  );
  const retryAfter = response.headers?.get?.("retry-after");
  if (retryAfter) {
    const sec = parseInt(retryAfter, 10);
    err.retryAfterSec = isNaN(sec) ? null : sec;
  }
  return err;
}

export function normalizeOpenAICompatibleUrl(baseUrl) {
  const parsed = new URL(baseUrl);
  const pathname = parsed.pathname.replace(/\/+$/, "");

  if (
    pathname.endsWith("/chat/completions") ||
    pathname.endsWith("/v1/messages") ||
    pathname.endsWith("/responses")
  ) {
    return parsed.toString();
  }

  parsed.pathname = `${pathname || ""}/chat/completions`.replace(
    /\/{2,}/g,
    "/",
  );
  return parsed.toString();
}

/**
 * Call the configured AI provider with streaming.
 * Calls `onDelta(text)` for each token as it arrives.
 * Returns the full accumulated response text.
 *
 * @param {Array<{role: string, content: string}>} messages
 * @param {object} config - { provider, api_key, model, base_url }
 * @param {string} [systemPrompt]
 * @param {(delta: string) => void} [onDelta] - streaming callback
 * @returns {Promise<string>}
 */
export async function callAIWithMessagesStreaming(
  messages,
  config,
  systemPrompt = "",
  onDelta = null,
) {
  const { provider, api_key, model, base_url } = config;

  if (!api_key) {
    throw new Error(
      "No API key configured. Run: decipher config set api-key <key>",
    );
  }

  // If no delta callback, fall back to non-streaming
  if (!onDelta) {
    return callAIWithMessages(messages, config, systemPrompt);
  }

  const effort = resolveReasoningEffort(config);
  const reasoningParams = buildReasoningParams(effort, provider, model ?? "");

  const fn = () => {
    if (base_url) {
      return streamOpenAIWithMessages(
        messages,
        systemPrompt,
        api_key,
        model ?? "gpt-4o",
        normalizeOpenAICompatibleUrl(base_url),
        onDelta,
        reasoningParams,
      );
    }

    if (provider === "anthropic") {
      return streamAnthropicWithMessages(
        messages,
        systemPrompt,
        api_key,
        model ?? "claude-sonnet-4-20250514",
        onDelta,
        reasoningParams,
      );
    }

    return streamOpenAIWithMessages(
      messages,
      systemPrompt,
      api_key,
      model ?? "gpt-4o",
      OPENAI_URL,
      onDelta,
      reasoningParams,
    );
  };

  return withRetry(fn, "agent turn (streaming)");
}

/**
 * Call the configured AI provider with a multi-turn messages array.
 * Used by the agent loop for stateful tool-calling conversations.
 *
 * @param {Array<{role: string, content: string}>} messages
 * @param {object} config - { provider, api_key, model, base_url }
 * @param {string} [systemPrompt] - optional system prompt (injected before user messages)
 * @returns {Promise<string>}
 */
export async function callAIWithMessages(messages, config, systemPrompt = "") {
  const { provider, api_key, model, base_url } = config;

  if (!api_key) {
    throw new Error(
      "No API key configured. Run: decipher config set api-key <key>",
    );
  }

  const effort = resolveReasoningEffort(config);
  const reasoningParams = buildReasoningParams(effort, provider, model ?? "");

  const fn = () => {
    if (base_url) {
      return callOpenAIWithMessages(
        messages,
        systemPrompt,
        api_key,
        model ?? "gpt-4o",
        normalizeOpenAICompatibleUrl(base_url),
        reasoningParams,
      );
    }

    if (provider === "anthropic") {
      return callAnthropicWithMessages(
        messages,
        systemPrompt,
        api_key,
        model ?? "claude-sonnet-4-20250514",
        reasoningParams,
      );
    }

    return callOpenAIWithMessages(
      messages,
      systemPrompt,
      api_key,
      model ?? "gpt-4o",
      OPENAI_URL,
      reasoningParams,
    );
  };

  return withRetry(fn, "agent turn");
}

/**
 * Call the configured AI provider with native tool calling.
 * Returns a structured response that may contain text, tool calls, or both.
 *
 * @param {Array<{role: string, content: string}>} messages
 * @param {Array} tools — provider-specific tool definitions (use buildTools from tool-registry.js)
 * @param {object} config — { provider, api_key, model, base_url }
 * @param {string} [systemPrompt]
 * @returns {Promise<ToolCallResponse>}
 *
 * @typedef {object} ToolCallResponse
 * @property {"text"|"tool_use"} type — whether the response is text or tool calls
 * @property {string|null} content — text content (if any)
 * @property {Array<{id: string, name: string, input: object}>} toolCalls — tool calls (if any)
 */
export async function callAIWithTools(
  messages,
  tools,
  config,
  systemPrompt = "",
) {
  const { provider, api_key, model, base_url } = config;

  if (!api_key) {
    throw new Error(
      "No API key configured. Run: decipher config set api-key <key>",
    );
  }

  const effort = resolveReasoningEffort(config);
  const reasoningParams = buildReasoningParams(effort, provider, model ?? "");

  const fn = () => {
    if (base_url || provider !== "anthropic") {
      const url = base_url
        ? normalizeOpenAICompatibleUrl(base_url)
        : OPENAI_URL;
      return callOpenAIWithTools(
        messages,
        tools,
        systemPrompt,
        api_key,
        model ?? "gpt-4o",
        url,
        reasoningParams,
      );
    }
    return callAnthropicWithTools(
      messages,
      tools,
      systemPrompt,
      api_key,
      model ?? "claude-sonnet-4-20250514",
      reasoningParams,
    );
  };

  return withRetry(fn, "tool-calling turn");
}

/**
 * Call the configured AI provider with a prompt.
 * @param {string} prompt - Full prompt text
 * @param {object} config - { provider, api_key, model, base_url }
 * @returns {Promise<string>} - The AI response text
 */
export async function callAI(prompt, config) {
  const { provider, api_key, model, base_url } = config;

  if (!api_key) {
    throw new Error(
      "No API key configured. Run: decipher config set api-key <key>",
    );
  }

  const fn = () => {
    if (base_url) {
      return callOpenAI(
        prompt,
        api_key,
        model ?? "gpt-4o",
        normalizeOpenAICompatibleUrl(base_url),
      );
    }

    if (provider === "anthropic") {
      return callAnthropic(
        prompt,
        api_key,
        model ?? "claude-sonnet-4-20250514",
      );
    }

    return callOpenAI(prompt, api_key, model ?? "gpt-4o", OPENAI_URL);
  };

  return withRetry(fn, "analysis");
}

async function withTimeout(promise, ms) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  try {
    return await promise;
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${ms / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

async function callOpenAI(prompt, apiKey, model, url) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  let response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: [{ role: "user", content: prompt }],
        temperature: 0,
      }),
      signal: controller.signal,
    });
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const error = await response.text();
    throw apiError(response, error);
  }

  const data = await response.json();
  if (data?.usage) {
    _lastUsage = {
      prompt_tokens: data.usage.prompt_tokens ?? 0,
      completion_tokens: data.usage.completion_tokens ?? 0,
      total_tokens: data.usage.total_tokens ?? 0,
    };
  }
  const text = data?.choices?.[0]?.message?.content;
  if (typeof text !== "string") {
    throw new Error(
      `Unexpected API response shape (missing choices[0].message.content): ${JSON.stringify(data).slice(0, 200)}`,
    );
  }
  return text.trim();
}

async function callAnthropic(prompt, apiKey, model) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  let response;
  try {
    response = await fetch(ANTHROPIC_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
      },
      body: JSON.stringify({
        model,
        max_tokens: 1024,
        messages: [{ role: "user", content: prompt }],
      }),
      signal: controller.signal,
    });
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const error = await response.text();
    throw apiError(response, error);
  }

  const data = await response.json();
  if (data?.usage) {
    _lastUsage = {
      prompt_tokens: data.usage.input_tokens ?? 0,
      completion_tokens: data.usage.output_tokens ?? 0,
      total_tokens:
        (data.usage.input_tokens ?? 0) + (data.usage.output_tokens ?? 0),
    };
  }
  const text = data?.content?.[0]?.text;
  if (typeof text !== "string") {
    throw new Error(
      `Unexpected Anthropic response shape (missing content[0].text): ${JSON.stringify(data).slice(0, 200)}`,
    );
  }
  return text.trim();
}

async function callOpenAIWithMessages(
  messages,
  systemPrompt,
  apiKey,
  model,
  url,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const builtMessages = systemPrompt
    ? [{ role: "system", content: systemPrompt }, ...messages]
    : messages;

  let response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: builtMessages,
        temperature: 0,
        ...extraBody,
      }),
      signal: controller.signal,
    });
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const error = await response.text();
    throw apiError(response, error);
  }

  const data = await response.json();
  if (data?.usage) {
    _lastUsage = {
      prompt_tokens: data.usage.prompt_tokens ?? 0,
      completion_tokens: data.usage.completion_tokens ?? 0,
      total_tokens: data.usage.total_tokens ?? 0,
    };
  }
  const text = data?.choices?.[0]?.message?.content;
  if (typeof text !== "string") {
    throw new Error(
      `Unexpected API response shape (missing choices[0].message.content): ${JSON.stringify(data).slice(0, 200)}`,
    );
  }
  return text.trim();
}

async function callAnthropicWithMessages(
  messages,
  systemPrompt,
  apiKey,
  model,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const body = {
    model,
    max_tokens: 4096,
    messages,
    ...extraBody,
  };
  // Anthropic prompt caching: mark system prompt as ephemeral for 20-25% savings
  if (systemPrompt) {
    body.system = [
      {
        type: "text",
        text: systemPrompt,
        cache_control: { type: "ephemeral" },
      },
    ];
  }

  let response;
  try {
    response = await fetch(ANTHROPIC_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
        "anthropic-beta": "prompt-caching-2024-07-31",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const error = await response.text();
    throw apiError(response, error);
  }

  const data = await response.json();
  if (data?.usage) {
    _lastUsage = {
      prompt_tokens: data.usage.input_tokens ?? 0,
      completion_tokens: data.usage.output_tokens ?? 0,
      total_tokens:
        (data.usage.input_tokens ?? 0) + (data.usage.output_tokens ?? 0),
    };
  }
  const text = data?.content?.[0]?.text;
  if (typeof text !== "string") {
    throw new Error(
      `Unexpected Anthropic response shape (missing content[0].text): ${JSON.stringify(data).slice(0, 200)}`,
    );
  }
  return text.trim();
}

// ── Native tool-calling functions ────────────────────────────────────────────

async function callOpenAIWithTools(
  messages,
  tools,
  systemPrompt,
  apiKey,
  model,
  url,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const builtMessages = systemPrompt
    ? [{ role: "system", content: systemPrompt }, ...messages]
    : messages;

  let response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: builtMessages,
        tools,
        temperature: 0,
        ...extraBody,
      }),
      signal: controller.signal,
    });
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const error = await response.text();
    throw apiError(response, error);
  }

  const data = await response.json();
  if (data?.usage) {
    _lastUsage = {
      prompt_tokens: data.usage.prompt_tokens ?? 0,
      completion_tokens: data.usage.completion_tokens ?? 0,
      total_tokens: data.usage.total_tokens ?? 0,
    };
  }

  const message = data?.choices?.[0]?.message;
  if (!message) {
    throw new Error(
      `Unexpected API response shape: ${JSON.stringify(data).slice(0, 200)}`,
    );
  }

  if (message.tool_calls && message.tool_calls.length > 0) {
    return {
      type: "tool_use",
      content: message.content ?? null,
      toolCalls: message.tool_calls.map((tc) => ({
        id: tc.id,
        name: tc.function.name,
        input: JSON.parse(tc.function.arguments),
      })),
      _rawMessage: message,
    };
  }

  return {
    type: "text",
    content: message.content?.trim() ?? "",
    toolCalls: [],
  };
}

async function callAnthropicWithTools(
  messages,
  tools,
  systemPrompt,
  apiKey,
  model,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const body = {
    model,
    max_tokens: 4096,
    messages,
    tools,
    ...extraBody,
  };
  if (systemPrompt) {
    body.system = [
      {
        type: "text",
        text: systemPrompt,
        cache_control: { type: "ephemeral" },
      },
    ];
  }

  let response;
  try {
    response = await fetch(ANTHROPIC_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
        "anthropic-beta": "prompt-caching-2024-07-31",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (err) {
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const error = await response.text();
    throw apiError(response, error);
  }

  const data = await response.json();
  if (data?.usage) {
    _lastUsage = {
      prompt_tokens: data.usage.input_tokens ?? 0,
      completion_tokens: data.usage.output_tokens ?? 0,
      total_tokens:
        (data.usage.input_tokens ?? 0) + (data.usage.output_tokens ?? 0),
    };
  }

  const content = data?.content ?? [];
  const textBlocks = content
    .filter((b) => b.type === "text")
    .map((b) => b.text)
    .join("\n");
  const toolUseBlocks = content.filter((b) => b.type === "tool_use");

  if (toolUseBlocks.length > 0) {
    return {
      type: "tool_use",
      content: textBlocks || null,
      toolCalls: toolUseBlocks.map((b) => ({
        id: b.id,
        name: b.name,
        input: b.input,
      })),
    };
  }

  return {
    type: "text",
    content: textBlocks.trim(),
    toolCalls: [],
  };
}

// ── Streaming tool-calling implementations ─────────────────────────────────

/**
 * Call the configured AI provider with native tool calling and streaming.
 * Returns a structured response with text content and/or tool calls.
 * Streams text deltas via onDelta callback.
 *
 * @param {Array} messages
 * @param {Array} tools — provider-specific tool definitions
 * @param {object} config — { provider, api_key, model, base_url, reasoning_effort }
 * @param {string} [systemPrompt]
 * @param {(delta: string) => void} [onDelta] — streaming callback for text content
 * @param {(reasoning: string) => void} [onReasoning] — streaming callback for reasoning/thinking
 * @returns {Promise<ToolCallStreamResult>}
 *
 * @typedef {object} ToolCallStreamResult
 * @property {"text"|"tool_use"} type
 * @property {string|null} content — accumulated text content
 * @property {Array<{id: string, name: string, input: object}>} toolCalls
 * @property {{prompt_tokens: number, completion_tokens: number, total_tokens: number}|null} usage
 */
export async function callAIWithToolsStreaming(
  messages,
  tools,
  config,
  systemPrompt = "",
  onDelta = null,
  onReasoning = null,
) {
  const { provider, api_key, model, base_url } = config;

  if (!api_key) {
    throw new Error(
      "No API key configured. Run: decipher config set api-key <key>",
    );
  }

  const effort = resolveReasoningEffort(config);
  const reasoningParams = buildReasoningParams(effort, provider, model ?? "");

  const fn = () => {
    if (base_url || provider !== "anthropic") {
      const url = base_url
        ? normalizeOpenAICompatibleUrl(base_url)
        : OPENAI_URL;
      return streamOpenAIWithTools(
        messages,
        tools,
        systemPrompt,
        api_key,
        model ?? "gpt-4o",
        url,
        onDelta,
        reasoningParams,
      );
    }
    return streamAnthropicWithTools(
      messages,
      tools,
      systemPrompt,
      api_key,
      model ?? "claude-sonnet-4-20250514",
      onDelta,
      onReasoning,
      reasoningParams,
    );
  };

  return withRetry(fn, "tool-calling turn (streaming)");
}

async function streamOpenAIWithTools(
  messages,
  tools,
  systemPrompt,
  apiKey,
  model,
  url,
  onDelta,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const builtMessages = systemPrompt
    ? [{ role: "system", content: systemPrompt }, ...messages]
    : messages;

  let response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: builtMessages,
        tools,
        temperature: 0,
        stream: true,
        stream_options: { include_usage: true },
        ...extraBody,
      }),
      signal: controller.signal,
    });
  } catch (err) {
    clearTimeout(timer);
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  }

  if (!response.ok) {
    clearTimeout(timer);
    const error = await response.text();
    throw apiError(response, error);
  }

  let textContent = "";
  // Accumulate tool calls: index -> { id, name, arguments (string) }
  const toolCallAccum = new Map();
  let usage = null;

  try {
    for await (const data of parseSSE(response)) {
      try {
        const parsed = JSON.parse(data);

        if (parsed.usage) {
          usage = {
            prompt_tokens: parsed.usage.prompt_tokens ?? 0,
            completion_tokens: parsed.usage.completion_tokens ?? 0,
            total_tokens: parsed.usage.total_tokens ?? 0,
          };
        }

        const choice = parsed.choices?.[0];
        if (!choice) continue;

        const delta = choice.delta;
        if (!delta) continue;

        // Text content delta
        if (delta.content) {
          textContent += delta.content;
          if (onDelta) onDelta(delta.content);
        }

        // Tool call deltas
        if (delta.tool_calls) {
          for (const tc of delta.tool_calls) {
            const idx = tc.index ?? 0;
            if (!toolCallAccum.has(idx)) {
              toolCallAccum.set(idx, {
                id: tc.id ?? "",
                name: tc.function?.name ?? "",
                arguments: "",
              });
            }
            const accum = toolCallAccum.get(idx);
            if (tc.id) accum.id = tc.id;
            if (tc.function?.name) accum.name = tc.function.name;
            if (tc.function?.arguments)
              accum.arguments += tc.function.arguments;
          }
        }
      } catch {
        // Skip malformed JSON chunks
      }
    }
  } finally {
    clearTimeout(timer);
  }

  if (usage) _lastUsage = usage;

  // Parse accumulated tool calls
  const toolCalls = [];
  for (const [, accum] of [...toolCallAccum.entries()].sort(
    (a, b) => a[0] - b[0],
  )) {
    try {
      toolCalls.push({
        id: accum.id,
        name: accum.name,
        input: JSON.parse(accum.arguments),
      });
    } catch {
      // Skip tool calls with unparseable arguments
    }
  }

  return {
    type: toolCalls.length > 0 ? "tool_use" : "text",
    content: textContent || null,
    toolCalls,
    usage,
  };
}

async function streamAnthropicWithTools(
  messages,
  tools,
  systemPrompt,
  apiKey,
  model,
  onDelta,
  onReasoning,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const body = {
    model,
    max_tokens: 4096,
    messages,
    tools,
    stream: true,
    ...extraBody,
  };
  if (systemPrompt) {
    body.system = [
      {
        type: "text",
        text: systemPrompt,
        cache_control: { type: "ephemeral" },
      },
    ];
  }

  let response;
  try {
    response = await fetch(ANTHROPIC_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
        "anthropic-beta": "prompt-caching-2024-07-31",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (err) {
    clearTimeout(timer);
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  }

  if (!response.ok) {
    clearTimeout(timer);
    const error = await response.text();
    throw apiError(response, error);
  }

  let textContent = "";
  // Track content blocks: index -> { type, id, name, inputJson }
  const contentBlocks = new Map();
  let usage = null;

  try {
    for await (const data of parseSSE(response)) {
      try {
        const event = JSON.parse(data);

        if (event.type === "message_start" && event.message?.usage) {
          const u = event.message.usage;
          usage = {
            prompt_tokens: u.input_tokens ?? 0,
            completion_tokens: u.output_tokens ?? 0,
            total_tokens: (u.input_tokens ?? 0) + (u.output_tokens ?? 0),
          };
        }

        if (event.type === "message_delta" && event.usage) {
          // Update with final output token count
          if (usage) {
            usage.completion_tokens =
              event.usage.output_tokens ?? usage.completion_tokens;
            usage.total_tokens = usage.prompt_tokens + usage.completion_tokens;
          }
        }

        if (event.type === "content_block_start") {
          const block = event.content_block;
          const idx = event.index ?? 0;
          if (block.type === "tool_use") {
            contentBlocks.set(idx, {
              type: "tool_use",
              id: block.id,
              name: block.name,
              inputJson: "",
            });
          } else if (block.type === "thinking") {
            contentBlocks.set(idx, { type: "thinking", text: "" });
          } else if (block.type === "text") {
            contentBlocks.set(idx, { type: "text", text: "" });
          }
        }

        if (event.type === "content_block_delta") {
          const idx = event.index ?? 0;
          const block = contentBlocks.get(idx);
          if (!block) continue;

          if (event.delta?.type === "text_delta" && block.type === "text") {
            const text = event.delta.text ?? "";
            block.text += text;
            textContent += text;
            if (onDelta) onDelta(text);
          }

          if (
            event.delta?.type === "thinking_delta" &&
            block.type === "thinking"
          ) {
            const thinking = event.delta.thinking ?? "";
            block.text += thinking;
            if (onReasoning) onReasoning(thinking);
          }

          if (
            event.delta?.type === "input_json_delta" &&
            block.type === "tool_use"
          ) {
            block.inputJson += event.delta.partial_json ?? "";
          }
        }
      } catch {
        // Skip malformed JSON chunks
      }
    }
  } finally {
    clearTimeout(timer);
  }

  if (usage) _lastUsage = usage;

  // Parse tool calls from content blocks
  const toolCalls = [];
  for (const [, block] of [...contentBlocks.entries()].sort(
    (a, b) => a[0] - b[0],
  )) {
    if (block.type === "tool_use") {
      try {
        toolCalls.push({
          id: block.id,
          name: block.name,
          input: block.inputJson ? JSON.parse(block.inputJson) : {},
        });
      } catch {
        // Skip unparseable tool inputs
      }
    }
  }

  return {
    type: toolCalls.length > 0 ? "tool_use" : "text",
    content: textContent || null,
    toolCalls,
    usage,
  };
}

// ── Streaming implementations ───────────────────────────────────────────────

/**
 * Parse SSE (Server-Sent Events) stream line by line.
 * Yields data strings from `data: ...` lines.
 */
async function* parseSSE(response) {
  const decoder = new TextDecoder();
  let buffer = "";
  for await (const chunk of response.body) {
    buffer += decoder.decode(chunk, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith("data: ")) {
        const data = trimmed.slice(6);
        if (data === "[DONE]") return;
        yield data;
      }
    }
  }
}

async function streamOpenAIWithMessages(
  messages,
  systemPrompt,
  apiKey,
  model,
  url,
  onDelta,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const builtMessages = systemPrompt
    ? [{ role: "system", content: systemPrompt }, ...messages]
    : messages;

  let response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: builtMessages,
        temperature: 0,
        stream: true,
        stream_options: { include_usage: true },
        ...extraBody,
      }),
      signal: controller.signal,
    });
  } catch (err) {
    clearTimeout(timer);
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  }

  if (!response.ok) {
    clearTimeout(timer);
    const error = await response.text();
    throw apiError(response, error);
  }

  let accumulated = "";
  try {
    for await (const data of parseSSE(response)) {
      try {
        const parsed = JSON.parse(data);
        // Extract usage from final chunk
        if (parsed.usage) {
          _lastUsage = {
            prompt_tokens: parsed.usage.prompt_tokens ?? 0,
            completion_tokens: parsed.usage.completion_tokens ?? 0,
            total_tokens: parsed.usage.total_tokens ?? 0,
          };
        }
        const delta = parsed.choices?.[0]?.delta?.content;
        if (delta) {
          accumulated += delta;
          onDelta(delta);
        }
      } catch {
        // Skip malformed JSON chunks
      }
    }
  } finally {
    clearTimeout(timer);
  }

  return accumulated.trim();
}

async function streamAnthropicWithMessages(
  messages,
  systemPrompt,
  apiKey,
  model,
  onDelta,
  extraBody = {},
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const body = {
    model,
    max_tokens: 4096,
    messages,
    stream: true,
    ...extraBody,
  };
  if (systemPrompt) {
    body.system = [
      {
        type: "text",
        text: systemPrompt,
        cache_control: { type: "ephemeral" },
      },
    ];
  }

  let response;
  try {
    response = await fetch(ANTHROPIC_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
        "anthropic-beta": "prompt-caching-2024-07-31",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (err) {
    clearTimeout(timer);
    if (err.name === "AbortError") {
      throw new Error(`API request timed out after ${TIMEOUT_MS / 1000}s`);
    }
    throw err;
  }

  if (!response.ok) {
    clearTimeout(timer);
    const error = await response.text();
    throw apiError(response, error);
  }

  let accumulated = "";
  try {
    for await (const data of parseSSE(response)) {
      try {
        const event = JSON.parse(data);
        if (event.type === "content_block_delta") {
          const delta = event.delta?.text;
          if (delta) {
            accumulated += delta;
            onDelta(delta);
          }
        }
        if (event.type === "message_delta" && event.usage) {
          _lastUsage = {
            prompt_tokens: 0,
            completion_tokens: event.usage.output_tokens ?? 0,
            total_tokens: event.usage.output_tokens ?? 0,
          };
        }
        if (event.type === "message_start" && event.message?.usage) {
          const u = event.message.usage;
          _lastUsage = {
            prompt_tokens: u.input_tokens ?? 0,
            completion_tokens: u.output_tokens ?? 0,
            total_tokens: (u.input_tokens ?? 0) + (u.output_tokens ?? 0),
          };
        }
      } catch {
        // Skip malformed JSON chunks
      }
    }
  } finally {
    clearTimeout(timer);
  }

  return accumulated.trim();
}
