import { test } from "node:test";
import assert from "node:assert/strict";
import { normalizeOpenAICompatibleUrl } from "../../lib/api-client.js";

test("normalizeOpenAICompatibleUrl preserves a full chat completions endpoint", () => {
  const url = normalizeOpenAICompatibleUrl("https://example.test/v1/chat/completions");
  assert.equal(url, "https://example.test/v1/chat/completions");
});

test("normalizeOpenAICompatibleUrl appends chat completions to a root-compatible base url", () => {
  const url = normalizeOpenAICompatibleUrl("https://ark.cn-beijing.volces.com/api/v3/");
  assert.equal(url, "https://ark.cn-beijing.volces.com/api/v3/chat/completions");
});

test("normalizeOpenAICompatibleUrl appends chat completions to a versioned base path", () => {
  const url = normalizeOpenAICompatibleUrl("https://example.test/v1");
  assert.equal(url, "https://example.test/v1/chat/completions");
});
