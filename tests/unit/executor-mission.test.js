import { test } from "node:test";
import assert from "node:assert/strict";

const { resolveExecutionMode } = await import("../../agents/executor/loop.js");
const { parseMission } = await import("../../lib/mission.js");

test("resolveExecutionMode keeps docker_build for build-only missions", () => {
  const mode = resolveExecutionMode(
    { execution_mode: "docker_build" },
    { currentMission: parseMission("build this Docker container") },
  );

  assert.equal(mode, "docker_build");
});

test("resolveExecutionMode promotes build/start missions to docker_run", () => {
  const mode = resolveExecutionMode(
    { execution_mode: "docker_build" },
    { currentMission: parseMission("build and start this container") },
  );

  assert.equal(mode, "docker_run");
});

test("resolveExecutionMode preserves healthcheck scenarios for repair missions", () => {
  const mode = resolveExecutionMode(
    { execution_mode: "healthcheck" },
    { currentMission: parseMission("fix this Docker failure") },
  );

  assert.equal(mode, "healthcheck");
});
