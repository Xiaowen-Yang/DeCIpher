import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.DECIPHER_CONFIG_DIR = await mkdtemp(
  join(tmpdir(), "decipher-mission-memory-"),
);

const {
  getMissionMemoryDir,
  buildMissionMemoryRecord,
  persistMissionMemory,
  listMissionMemories,
  loadMissionMemory,
} = await import("../../lib/mission-memory.js");

test("buildMissionMemoryRecord captures mission, stop reason, and files touched", () => {
  const record = buildMissionMemoryRecord({
    target_path: "/tmp/workload",
    mission: {
      type: "benchmark_run",
      goal: "Run the HPL benchmark",
      stop_boundary: "benchmark_completed",
    },
    summary: {
      files_touched: ["Dockerfile", "run-hpl.sh"],
      files_written_back: ["Dockerfile"],
    },
    stop_reason: "needs_clarification",
    last_verification_state: "RUN_FAIL",
  });

  assert.equal(record.target_path, "/tmp/workload");
  assert.equal(record.mission_type, "benchmark_run");
  assert.equal(record.mission_goal, "Run the HPL benchmark");
  assert.equal(record.stop_reason, "needs_clarification");
  assert.deepEqual(record.files_touched, ["Dockerfile", "run-hpl.sh"]);
});

test("persistMissionMemory writes a mission record that can be listed and loaded", async () => {
  const persisted = await persistMissionMemory({
    target_path: "/tmp/workload",
    mission: {
      type: "build_start",
      goal: "Build and start this container",
      stop_boundary: "container_running",
    },
    summary: {
      files_touched: ["Dockerfile"],
      files_written_back: [],
    },
    stop_reason: null,
    last_verification_state: "PASS",
  });

  assert.match(persisted.id, /mission-/);
  assert.match(persisted.path, new RegExp(`${getMissionMemoryDir().replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));

  const listed = await listMissionMemories();
  assert.equal(listed.length, 1);
  assert.equal(listed[0].id, persisted.id);

  const loaded = await loadMissionMemory(persisted.id);
  assert.equal(loaded.mission_goal, "Build and start this container");
  assert.equal(loaded.last_verification_state, "PASS");
});

test("mission-memory cleanup", async () => {
  await rm(process.env.DECIPHER_CONFIG_DIR, { recursive: true, force: true });
});
