import { test } from "node:test";
import assert from "node:assert/strict";
import {
  normalizeOpenAICompatibleUrl,
  isRetryableError,
  backoffDelay,
} from "../../lib/api-client.js";

test("normalizeOpenAICompatibleUrl preserves a full chat completions endpoint", () => {
  const url = normalizeOpenAICompatibleUrl(
    "https://example.test/v1/chat/completions",
  );
  assert.equal(url, "https://example.test/v1/chat/completions");
});

test("normalizeOpenAICompatibleUrl appends chat completions to a root-compatible base url", () => {
  const url = normalizeOpenAICompatibleUrl(
    "https://ark.cn-beijing.volces.com/api/v3/",
  );
  assert.equal(
    url,
    "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
  );
});

test("normalizeOpenAICompatibleUrl appends chat completions to a versioned base path", () => {
  const url = normalizeOpenAICompatibleUrl("https://example.test/v1");
  assert.equal(url, "https://example.test/v1/chat/completions");
});

// ── Retry logic ──────────────────────────────────────────────────────────────

test("isRetryableError returns true for timeout errors", () => {
  assert.equal(
    isRetryableError({ name: "AbortError", message: "aborted" }),
    true,
  );
});

test("isRetryableError returns true for 429 rate limit", () => {
  assert.equal(
    isRetryableError({ message: "API error 429: rate limited" }),
    true,
  );
});

test("isRetryableError returns true for 500/502/503 server errors", () => {
  assert.equal(isRetryableError({ message: "API error 500: internal" }), true);
  assert.equal(
    isRetryableError({ message: "API error 502: bad gateway" }),
    true,
  );
  assert.equal(
    isRetryableError({ message: "API error 503: unavailable" }),
    true,
  );
});

test("isRetryableError returns true for network errors", () => {
  assert.equal(isRetryableError({ message: "fetch failed: ECONNRESET" }), true);
  assert.equal(isRetryableError({ message: "ETIMEDOUT" }), true);
});

test("isRetryableError returns false for auth errors (401)", () => {
  assert.equal(
    isRetryableError({ message: "API error 401: unauthorized" }),
    false,
  );
});

test("isRetryableError returns false for bad request (400)", () => {
  assert.equal(isRetryableError({ message: "API error 400: invalid" }), false);
});

test("backoffDelay uses exponential growth capped at 30s", () => {
  assert.equal(backoffDelay(0), 1000);
  assert.equal(backoffDelay(1), 2000);
  assert.equal(backoffDelay(2), 4000);
  assert.equal(backoffDelay(3), 8000);
  assert.equal(backoffDelay(10), 30000); // capped
});
