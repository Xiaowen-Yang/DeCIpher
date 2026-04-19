import { test } from "node:test";
import assert from "node:assert/strict";

const { parseMission, updateMissionFromUserInput } =
  await import("../../lib/mission.js");

test("parseMission maps build-and-start style requests to a bounded container mission", () => {
  const mission = parseMission("I need to build and start this container");

  assert.equal(mission.type, "build_start");
  assert.equal(mission.stop_boundary, "container_running");
  assert.equal(mission.requires_clarification, false);
  assert.equal(mission.domain, "container");
});

test("parseMission maps build-only container requests to an image_built boundary", () => {
  const mission = parseMission("build this Docker container");

  assert.equal(mission.type, "build");
  assert.equal(mission.stop_boundary, "image_built");
  assert.equal(mission.requires_clarification, false);
  assert.equal(mission.domain, "container");
});

test("parseMission maps benchmark execution requests to benchmark_run", () => {
  const mission = parseMission(
    "I need to run the HPL benchmark on this machine",
  );

  assert.equal(mission.type, "benchmark_run");
  assert.equal(mission.stop_boundary, "benchmark_completed");
  assert.equal(mission.domain, "benchmark");
  assert.equal(mission.requires_clarification, false);
});

test("parseMission maps tuning language to an optimization mission", () => {
  const mission = parseMission("keep tuning and rerunning the benchmark");

  assert.equal(mission.type, "benchmark_tune");
  assert.equal(mission.stop_boundary, "user_stop");
  assert.equal(mission.requires_clarification, false);
});

test("parseMission asks for clarification when no actionable goal is present", () => {
  const mission = parseMission("can you help me with this?");

  assert.equal(mission.type, "clarify");
  assert.equal(mission.requires_clarification, true);
  assert.match(mission.clarification_question, /what do you want/i);
});

test("parseMission maps Dockerfile generation requests to the generate type", () => {
  const mission = parseMission("generate a Dockerfile for this project");

  assert.equal(mission.type, "generate");
  assert.equal(mission.stop_boundary, "files_generated");
  assert.equal(mission.domain, "container");
  assert.equal(mission.requires_clarification, false);
});

test("parseMission maps CI config generation requests to the generate type with ci domain", () => {
  const mission = parseMission(
    "create a GitHub Actions workflow for this repo",
  );

  assert.equal(mission.type, "generate");
  assert.equal(mission.stop_boundary, "files_generated");
  assert.equal(mission.domain, "ci");
  assert.equal(mission.requires_clarification, false);
});

test("updateMissionFromUserInput extends an existing build mission when the user asks for benchmark execution", () => {
  const current = parseMission("build and start this container");
  const updated = updateMissionFromUserInput(
    current,
    "now run the benchmark too",
  );

  assert.equal(updated.type, "benchmark_run");
  assert.equal(updated.stop_boundary, "benchmark_completed");
  assert.equal(updated.previous_type, "build_start");
});
