import { test } from "node:test";
import assert from "node:assert/strict";

const { parseMission } = await import("../../lib/mission.js");
const { createMissionPlan, selectMissionRoute } =
  await import("../../agents/planner/index.js");

test("createMissionPlan builds repair-oriented steps for repair missions", () => {
  const mission = parseMission("fix this CI failure");
  const plan = createMissionPlan(mission);

  assert.equal(plan.selected_subsystem, "repair");
  assert.equal(plan.requires_clarification, false);
  assert.ok(plan.steps.some((step) => step.id === "reproduce_failure"));
  assert.ok(plan.steps.some((step) => step.id === "verify_repair"));
});

test("createMissionPlan includes generation and runtime steps for build/start missions", () => {
  const mission = parseMission("build and start this container");
  const plan = createMissionPlan(mission);

  assert.equal(plan.selected_subsystem, "generation_or_repair");
  assert.ok(plan.steps.some((step) => step.id === "inspect_target"));
  assert.ok(plan.steps.some((step) => step.id === "generate_or_repair_assets"));
  assert.ok(plan.steps.some((step) => step.id === "start_container"));
  assert.ok(plan.steps.some((step) => step.id === "verify_container_running"));
});

test("createMissionPlan stops at image build when the mission is build-only", () => {
  const mission = parseMission("build this Docker container");
  const plan = createMissionPlan(mission);

  assert.equal(plan.selected_subsystem, "generation_or_repair");
  assert.ok(plan.steps.some((step) => step.id === "build_container"));
  assert.ok(plan.steps.some((step) => step.id === "verify_image_built"));
  assert.equal(
    plan.steps.some((step) => step.id === "start_container"),
    false,
  );
});

test("createMissionPlan extends to benchmark execution when requested", () => {
  const mission = parseMission("run the HPL benchmark on this machine");
  const plan = createMissionPlan(mission);

  assert.equal(plan.selected_subsystem, "generation_or_repair");
  assert.ok(plan.steps.some((step) => step.id === "run_benchmark"));
  assert.ok(plan.steps.some((step) => step.id === "collect_benchmark_result"));
});

test("createMissionPlan returns clarification state for ambiguous missions", () => {
  const mission = parseMission("please help");
  const plan = createMissionPlan(mission);

  assert.equal(plan.requires_clarification, true);
  assert.match(plan.clarification_question, /what do you want/i);
  assert.equal(plan.steps.length, 0);
});

test("selectMissionRoute chooses execute_target for actionable scenario repair missions", () => {
  const mission = parseMission("fix this CI failure");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, {
    path: "/tmp/scenario",
    type: "scenario",
  });

  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "fix");
});

test("selectMissionRoute chooses docker_build for actionable Dockerfile build missions", () => {
  const mission = parseMission("build this Docker container");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, {
    path: "/tmp/Dockerfile",
    type: "dockerfile",
  });

  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "docker_build");
});

test("createMissionPlan builds generation steps for generate missions", () => {
  const mission = parseMission("generate a Dockerfile for this project");
  const plan = createMissionPlan(mission);

  assert.equal(plan.selected_subsystem, "generation");
  assert.equal(plan.requires_clarification, false);
  assert.ok(plan.steps.some((step) => step.id === "inspect_target"));
  assert.ok(plan.steps.some((step) => step.id === "generate_files"));
  assert.ok(plan.steps.some((step) => step.id === "verify_generated"));
});

test("selectMissionRoute asks for a concrete target when mission is actionable but no target is resolved", () => {
  const mission = parseMission("build this Docker container");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, null);

  assert.equal(route.mode, "clarify");
  assert.match(route.question, /which directory|which file|target/i);
});

test("selectMissionRoute returns action: generate for a generation plan with a nodejs target", () => {
  const mission = parseMission("generate a Dockerfile for this project");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, {
    path: "/tmp/my-app",
    type: "nodejs",
  });

  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "generate");
});

test("selectMissionRoute returns action: generate for a generation plan with a scenario target", () => {
  const mission = parseMission("generate a GitHub Actions workflow");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, {
    path: "/tmp/ci-missing-workflow",
    type: "scenario",
    meta: { id: "ci-missing-workflow", mission_type: "generate" },
  });

  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "generate");
});

test("selectMissionRoute returns benchmark_run for benchmark missions", () => {
  const mission = parseMission("run the HPL benchmark on this machine");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, {
    path: "/tmp/hpl-scenario",
    type: "scenario",
  });

  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "benchmark_run");
});

test("selectMissionRoute returns benchmark_run for benchmark_tune missions", () => {
  const mission = parseMission("keep tuning and rerun the benchmark");
  const plan = createMissionPlan(mission);
  const route = selectMissionRoute(plan, {
    path: "/tmp/hpl-scenario",
    type: "scenario",
  });

  assert.equal(route.mode, "execute_target");
  assert.equal(route.action, "benchmark_run");
});
