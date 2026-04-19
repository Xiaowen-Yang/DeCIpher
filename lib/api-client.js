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

function backoffDelay(attempt) {
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
        const delay = backoffDelay(attempt);
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

export { isRetryableError, backoffDelay };

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

  const fn = () => {
    if (base_url) {
      return callOpenAIWithMessages(
        messages,
        systemPrompt,
        api_key,
        model ?? "gpt-4o",
        normalizeOpenAICompatibleUrl(base_url),
      );
    }

    if (provider === "anthropic") {
      return callAnthropicWithMessages(
        messages,
        systemPrompt,
        api_key,
        model ?? "claude-sonnet-4-20250514",
      );
    }

    return callOpenAIWithMessages(
      messages,
      systemPrompt,
      api_key,
      model ?? "gpt-4o",
      OPENAI_URL,
    );
  };

  return withRetry(fn, "agent turn");
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
    throw new Error(`OpenAI API error ${response.status}: ${error}`);
  }

  const data = await response.json();
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
    throw new Error(`Anthropic API error ${response.status}: ${error}`);
  }

  const data = await response.json();
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
    throw new Error(`OpenAI API error ${response.status}: ${error}`);
  }

  const data = await response.json();
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
) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const body = {
    model,
    max_tokens: 4096,
    messages,
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
    throw new Error(`Anthropic API error ${response.status}: ${error}`);
  }

  const data = await response.json();
  const text = data?.content?.[0]?.text;
  if (typeof text !== "string") {
    throw new Error(
      `Unexpected Anthropic response shape (missing content[0].text): ${JSON.stringify(data).slice(0, 200)}`,
    );
  }
  return text.trim();
}
