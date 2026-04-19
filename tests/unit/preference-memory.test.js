import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.DECIPHER_CONFIG_DIR = await mkdtemp(
  join(tmpdir(), "decipher-preference-memory-"),
);

const {
  getPreferenceMemoryPath,
  readUserPreferences,
  writeUserPreferences,
  mergeMissionPreferences,
} = await import("../../lib/preference-memory.js");

test("readUserPreferences returns defaults when no preference file exists", async () => {
  const prefs = await readUserPreferences();
  assert.deepEqual(prefs, {});
});

test("writeUserPreferences persists user preferences separately from config", async () => {
  const persisted = await writeUserPreferences({
    preserve_workspaces: true,
    benchmark_mode: "stop_at_container_start",
  });

  assert.equal(persisted.preserve_workspaces, true);
  assert.equal(persisted.benchmark_mode, "stop_at_container_start");
  assert.match(getPreferenceMemoryPath(), /preferences\.json$/);

  const loaded = await readUserPreferences();
  assert.deepEqual(loaded, persisted);
});

test("mergeMissionPreferences overlays mission-specific values on stored preferences", () => {
  const merged = mergeMissionPreferences(
    {
      preserve_workspaces: false,
      benchmark_mode: "run_to_completion",
    },
    {
      benchmark_mode: "stop_at_container_start",
    },
  );

  assert.deepEqual(merged, {
    preserve_workspaces: false,
    benchmark_mode: "stop_at_container_start",
  });
});

test("preference-memory cleanup", async () => {
  await rm(process.env.DECIPHER_CONFIG_DIR, { recursive: true, force: true });
});
