import { test } from "node:test";
import assert from "node:assert/strict";

const {
  parseAnalysisResponse,
  fallbackAnalysis,
  buildPriorContext,
  tryRecoverPartialJSON,
  buildScenarioList,
} = await import("../../lib/mission-analyzer.js");

// ── parseAnalysisResponse ─────────────────────────────────────────────────────

test("parseAnalysisResponse: parses a valid analysis JSON", () => {
  const raw = JSON.stringify({
    understood_as: "Build and start the container",
    action: "build_start",
    steps: ["Build image", "Start container", "Verify running"],
    requires_clarification: false,
    clarification_question: null,
  });

  const result = parseAnalysisResponse(raw);
  assert.equal(result.understood_as, "Build and start the container");
  assert.equal(result.action, "build_start");
  assert.deepEqual(result.steps, [
    "Build image",
    "Start container",
    "Verify running",
  ]);
  assert.equal(result.requires_clarification, false);
  assert.equal(result.inferred, false);
});

test("parseAnalysisResponse: strips markdown fences before parsing", () => {
  const raw =
    '```json\n{"understood_as":"Fix the Dockerfile","action":"fix","steps":["Triage","Patch"],"requires_clarification":false,"clarification_question":null}\n```';

  const result = parseAnalysisResponse(raw);
  assert.equal(result.action, "fix");
  assert.equal(result.steps.length, 2);
});

test("parseAnalysisResponse: handles requires_clarification with question", () => {
  const raw = JSON.stringify({
    understood_as: "Run something",
    action: "fix",
    steps: [],
    requires_clarification: true,
    clarification_question: "Which directory should DeCIpher work on?",
  });

  const result = parseAnalysisResponse(raw);
  assert.equal(result.requires_clarification, true);
  assert.match(result.clarification_question, /directory/i);
});

test("parseAnalysisResponse: recovers partial JSON with key fields", () => {
  const raw =
    '{"understood_as": "Build the image", "action": "docker_build", "steps": ["Bu';
  const result = parseAnalysisResponse(raw);
  assert.notEqual(result, null);
  assert.equal(result.understood_as, "Build the image");
  assert.equal(result.action, "docker_build");
});

test("parseAnalysisResponse: returns null for non-JSON input without key fields", () => {
  const result = parseAnalysisResponse("Sorry, I cannot help with that.");
  assert.equal(result, null);
});

test("parseAnalysisResponse: fills defaults for missing fields", () => {
  const raw = JSON.stringify({ understood_as: "Do something" });
  const result = parseAnalysisResponse(raw);
  assert.equal(result.action, "fix");
  assert.deepEqual(result.steps, []);
  assert.equal(result.requires_clarification, false);
});

// ── tryRecoverPartialJSON ────────────────────────────────────────────────────

test("tryRecoverPartialJSON: extracts understood_as and action from truncated JSON", () => {
  const raw =
    '{"understood_as": "Fix CI pipeline", "action": "fix", "steps": ["Inspect';
  const result = tryRecoverPartialJSON(raw);
  assert.notEqual(result, null);
  assert.equal(result.understood_as, "Fix CI pipeline");
  assert.equal(result.action, "fix");
});

test("tryRecoverPartialJSON: returns null for empty or gibberish input", () => {
  assert.equal(tryRecoverPartialJSON(""), null);
  assert.equal(tryRecoverPartialJSON(null), null);
  assert.equal(tryRecoverPartialJSON("no json here"), null);
});

test("tryRecoverPartialJSON: detects requires_clarification true", () => {
  const raw =
    '{"understood_as": "Unclear", "action": "fix", "requires_clarification": true';
  const result = tryRecoverPartialJSON(raw);
  assert.notEqual(result, null);
  assert.equal(result.requires_clarification, true);
});

// ── buildPriorContext ────────────────────────────────────────────────────────

test("buildPriorContext: returns first turn message when no prior analysis", () => {
  const result = buildPriorContext(null);
  assert.match(result, /first turn/i);
});

test("buildPriorContext: includes previous understanding and action", () => {
  const prior = {
    understood_as: "Build the container",
    action: "docker_build",
    steps: ["Inspect", "Build"],
  };
  const result = buildPriorContext(prior);
  assert.match(result, /Build the container/);
  assert.match(result, /docker_build/);
  assert.match(result, /Inspect/);
});

// ── buildScenarioList ────────────────────────────────────────────────────────

test("buildScenarioList: returns a non-empty string with scenario IDs", async () => {
  const list = await buildScenarioList();
  assert.ok(typeof list === "string");
  assert.ok(list.length > 0);
  // Should contain at least one known scenario
  assert.match(list, /docker-copy-path-bug|hpl-build-only|ci-missing-workflow/);
});

// ── fallbackAnalysis ──────────────────────────────────────────────────────────

test("fallbackAnalysis: uses docker_build action for dockerfile targets", () => {
  const target = { path: "/tmp/myapp/Dockerfile", type: "dockerfile" };
  const result = fallbackAnalysis("build this", target);

  assert.equal(result.action, "docker_build");
  assert.equal(result.inferred, true);
  assert.equal(result.requires_clarification, false);
});

test("fallbackAnalysis: uses triage_only for logfile targets", () => {
  const target = { path: "/tmp/ci.log", type: "logfile" };
  const result = fallbackAnalysis("what went wrong", target);

  assert.equal(result.action, "triage_only");
  assert.equal(result.inferred, true);
});

test("fallbackAnalysis: uses fix for scenario targets", () => {
  const target = { path: "/tmp/docker-copy-path-bug", type: "scenario" };
  const result = fallbackAnalysis("run this", target);

  assert.equal(result.action, "fix");
  assert.equal(result.requires_clarification, false);
});

test("fallbackAnalysis: requires clarification when no target is provided", () => {
  const result = fallbackAnalysis("do something useful", null);

  assert.equal(result.requires_clarification, true);
  assert.ok(result.clarification_question);
  assert.equal(result.inferred, true);
});
