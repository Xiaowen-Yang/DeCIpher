import { test } from "node:test";
import assert from "node:assert/strict";
import { access, readFile, rm } from "node:fs/promises";
import { constants } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

process.env.DECIPHER_CONFIG_DIR = join(
  tmpdir(),
  `decipher-session-store-${Date.now()}`,
);

const {
  getSessionStorePath,
  loadSessionSnapshot,
  persistSessionSnapshot,
  clearSessionSnapshot,
  maskSessionSnapshot,
  formatSessionSnapshot,
} = await import("../../lib/session-store.js");

test("loadSessionSnapshot returns null when no snapshot has been persisted", async () => {
  const snapshot = await loadSessionSnapshot();
  assert.equal(snapshot, null);
});

test("persistSessionSnapshot writes a snapshot that loadSessionSnapshot returns", async () => {
  const snapshot = {
    scenarioPath: "/tmp/scenarios/docker-copy-path-bug",
    executionMode: "docker_build",
    iteration: 2,
    config: {
      provider: "openai",
      api_key: "sk-1234567890",
    },
    result: {
      classification: "missing-file",
      confidence: 0.92,
    },
  };

  const persisted = await persistSessionSnapshot(snapshot);
  const loaded = await loadSessionSnapshot();

  assert.deepEqual(persisted, snapshot);
  assert.deepEqual(loaded, snapshot);
});

test("clearSessionSnapshot removes the persisted snapshot file", async () => {
  await persistSessionSnapshot({ scenarioPath: "/tmp/example", iteration: 1 });

  await clearSessionSnapshot();

  await assert.rejects(
    access(getSessionStorePath(), constants.F_OK),
    { code: "ENOENT" },
  );
  await assert.equal(await loadSessionSnapshot(), null);
});

test("maskSessionSnapshot redacts nested secret fields without changing other values", () => {
  const masked = maskSessionSnapshot({
    scenarioPath: "/tmp/example",
    config: {
      provider: "openai",
      api_key: "sk-1234567890",
      base_url: "https://example.test/v1",
    },
    auth: {
      token: "ghp_abcdefghijklmnopqrstuvwxyz",
      authorization: "Bearer super-secret-token",
    },
  });

  assert.equal(masked.scenarioPath, "/tmp/example");
  assert.equal(masked.config.provider, "openai");
  assert.equal(masked.config.base_url, "https://example.test/v1");
  assert.equal(masked.config.api_key, "sk-***90");
  assert.equal(masked.auth.token, "gh-***yz");
  assert.equal(masked.auth.authorization, "Be-***en");
});

test("formatSessionSnapshot returns pretty JSON and masks secrets in public mode", async () => {
  const snapshot = {
    scenarioPath: "/tmp/example",
    config: {
      api_key: "sk-1234567890",
    },
  };

  await persistSessionSnapshot(snapshot);
  const formatted = formatSessionSnapshot(await loadSessionSnapshot(), {
    public: true,
  });

  assert.match(formatted, /\n  "scenarioPath": "\/tmp\/example"/);
  assert.match(formatted, /"api_key": "sk-\*\*\*90"/);
  assert.doesNotMatch(formatted, /1234567890/);
});

test("session-store cleanup", async () => {
  await rm(process.env.DECIPHER_CONFIG_DIR, { recursive: true, force: true });
});
