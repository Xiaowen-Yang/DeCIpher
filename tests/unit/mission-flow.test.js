/**
 * Mission-level validation tests.
 *
 * These tests validate the full V2 flow: mission parsing → planning →
 * route selection → CLI interaction decision. They replace scenario-only
 * regression thinking with mission-level validation.
 */
import { test } from "node:test";
import assert from "node:assert/strict";

const { parseMission } = await import("../../lib/mission.js");
const { createMissionPlan, selectMissionRoute } = await import(
  "../../agents/planner/index.js"
);
const { decideCliInteraction } = await import("../../lib/cli-surface.js");

// ── Full mission flows ──────────────────────────────────────────────────────

test("repair flow: fix CI failure → repair plan → execute_target with fix action", () => {
  const mission = parseMission("fix this CI pipeline failure");
  assert.equal(mission.type, "repair");
  assert.equal(mission.domain, "ci");

  const plan = createMissionPlan(mission);
  assert.equal(plan.selected_subsystem, "repair");
  assert.ok(plan.steps.some((s) => s.id === "classify_failure"));

  const route = selectMissionRoute(plan, {
    path: "/tmp/ci-scenario",
    type: "scenario",
  });
  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "fix");

  const interaction = decideCliInteraction({ route });
  assert.equal(interaction.mode, "execute_target");
  assert.equal(interaction.action, "fix");
});

test("build flow: build container → build plan → execute_target with docker_build", () => {
  const mission = parseMission("build this Docker container");
  assert.equal(mission.type, "build");
  assert.equal(mission.stop_boundary, "image_built");

  const plan = createMissionPlan(mission);
  assert.ok(plan.steps.some((s) => s.id === "verify_image_built"));
  assert.ok(!plan.steps.some((s) => s.id === "start_container"));

  const route = selectMissionRoute(plan, {
    path: "/tmp/Dockerfile",
    type: "dockerfile",
  });
  assert.equal(route.action, "docker_build");
});

test("build_start flow: build and start → plan includes start + verify running", () => {
  const mission = parseMission("build and run this container");
  assert.equal(mission.type, "build_start");
  assert.equal(mission.stop_boundary, "container_running");

  const plan = createMissionPlan(mission);
  assert.ok(plan.steps.some((s) => s.id === "start_container"));
  assert.ok(plan.steps.some((s) => s.id === "verify_container_running"));
});

test("benchmark flow: run HPL → plan includes benchmark + result collection", () => {
  const mission = parseMission("run the HPL benchmark in Docker");
  assert.equal(mission.type, "benchmark_run");
  assert.equal(mission.stop_boundary, "benchmark_completed");

  const plan = createMissionPlan(mission);
  assert.ok(plan.steps.some((s) => s.id === "run_benchmark"));
  assert.ok(plan.steps.some((s) => s.id === "collect_benchmark_result"));

  const route = selectMissionRoute(plan, {
    path: "/tmp/hpl-scenario",
    type: "scenario",
  });
  assert.equal(route.action, "benchmark_run");
});

test("generate flow: create Dockerfile → generation plan → generate action", () => {
  const mission = parseMission("generate a Dockerfile for this project");
  assert.equal(mission.type, "generate");
  assert.equal(mission.stop_boundary, "files_generated");

  const plan = createMissionPlan(mission);
  assert.equal(plan.selected_subsystem, "generation");
  assert.ok(plan.steps.some((s) => s.id === "generate_files"));

  const route = selectMissionRoute(plan, {
    path: "/tmp/project",
    type: "nodejs",
  });
  assert.equal(route.action, "generate");
});

test("tune flow: keep tuning → optimization plan with user_stop boundary", () => {
  const mission = parseMission("keep tuning and rerun the benchmark");
  assert.equal(mission.type, "benchmark_tune");
  assert.equal(mission.stop_boundary, "user_stop");

  const plan = createMissionPlan(mission);
  assert.ok(plan.steps.some((s) => s.id === "adjust_parameters"));
});

// ── Clarification gating ────────────────────────────────────────────────────

test("ambiguous input triggers clarification gate", () => {
  const mission = parseMission("please help");
  assert.equal(mission.type, "clarify");
  assert.equal(mission.requires_clarification, true);

  const plan = createMissionPlan(mission);
  assert.equal(plan.requires_clarification, true);
  assert.equal(plan.steps.length, 0);

  // With no target, route should clarify
  const route = selectMissionRoute(plan, null);
  assert.equal(route.mode, "clarify");

  const interaction = decideCliInteraction({ route });
  assert.equal(interaction.mode, "clarify");
  assert.ok(interaction.question);
});

test("ambiguous mission with target resolved → inferred action instead of blocking", () => {
  const mission = parseMission("please help");
  const plan = createMissionPlan(mission);

  // Even with unclear plan, a resolved target allows inferred execution
  const route = selectMissionRoute(plan, {
    path: "/tmp/Dockerfile",
    type: "dockerfile",
  });
  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "docker_build");
  assert.equal(route.inferred, true);
});

test("conversational input (question) routes to conversation mode", () => {
  const interaction = decideCliInteraction({
    route: null,
    input: "what does this Dockerfile do?",
  });
  assert.equal(interaction.mode, "conversation");
});

test("non-question free text without a route triggers clarification", () => {
  const interaction = decideCliInteraction({
    route: null,
    input: "something random",
  });
  assert.equal(interaction.mode, "clarify");
});

// ── Mission boundary enforcement ────────────────────────────────────────────

test("build-only mission does not include start_container step", () => {
  const mission = parseMission("build this Docker image");
  const plan = createMissionPlan(mission);
  assert.ok(!plan.steps.some((s) => s.id === "start_container"));
  assert.ok(!plan.steps.some((s) => s.id === "run_benchmark"));
});

test("build_start mission does not include benchmark steps", () => {
  const mission = parseMission("build and start this container");
  const plan = createMissionPlan(mission);
  assert.ok(!plan.steps.some((s) => s.id === "run_benchmark"));
  assert.ok(!plan.steps.some((s) => s.id === "collect_benchmark_result"));
});

test("benchmark_run mission includes all steps from build through benchmark", () => {
  const mission = parseMission("run the benchmark on this machine");
  const plan = createMissionPlan(mission);
  assert.ok(plan.steps.some((s) => s.id === "build_container"));
  assert.ok(plan.steps.some((s) => s.id === "start_container"));
  assert.ok(plan.steps.some((s) => s.id === "run_benchmark"));
});

// ── Mission type transition tracking ────────────────────────────────────────

test("updateMissionFromUserInput tracks type transitions", async () => {
  const { updateMissionFromUserInput } = await import("../../lib/mission.js");
  const first = parseMission("build this container");
  const second = updateMissionFromUserInput(first, "now run the benchmark too");
  assert.equal(second.type, "benchmark_run");
  assert.equal(second.previous_type, "build");
});
