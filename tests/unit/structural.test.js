import { test } from "node:test";
import assert from "node:assert/strict";

const { buildReport } = await import("../../agents/executor/structural.js");

const BASE_META = {
  id: "test-scenario",
  category: "ci",
  verification_command: "echo PASS",
};

// ── LoopResult-compatible fields ──────────────────────────────────────────────

test("buildReport includes outcome field with PASS for passing verification", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: {
      classification: "config_error",
      confidence: 0.9,
      root_causes: [],
    },
    patchArtifact: {
      patch: "diff",
      affected_files: ["ci.yml"],
      blast_radius: "low",
      rollback_hint: "git checkout",
    },
    verificationArtifact: {
      command: "echo PASS",
      exit_code: 0,
      stdout_excerpt: "PASS",
      result: "PASS",
    },
    patchApplied: true,
    stopped: null,
  });

  assert.equal(report.outcome, "PASS");
});

test("buildReport includes outcome FAIL when verification fails", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: {
      classification: "config_error",
      confidence: 0.9,
      root_causes: [],
    },
    patchArtifact: {
      patch: "diff",
      affected_files: ["ci.yml"],
      blast_radius: "low",
      rollback_hint: "git checkout",
    },
    verificationArtifact: {
      command: "echo FAIL",
      exit_code: 1,
      stdout_excerpt: "FAIL",
      result: "FAIL",
    },
    patchApplied: true,
    stopped: null,
  });

  assert.equal(report.outcome, "FAIL");
});

test("buildReport includes outcome NEEDS_HUMAN_REVIEW when stopped with that label", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: null,
    patchArtifact: null,
    verificationArtifact: null,
    patchApplied: false,
    stopped: "NEEDS_HUMAN_REVIEW — confidence too low",
  });

  assert.equal(report.outcome, "NEEDS_HUMAN_REVIEW");
});

test("buildReport includes state field mapping PASS result", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: {
      classification: "config_error",
      confidence: 0.9,
      root_causes: [],
    },
    patchArtifact: null,
    verificationArtifact: {
      command: "echo PASS",
      exit_code: 0,
      stdout_excerpt: "PASS",
      result: "PASS",
    },
    patchApplied: false,
    stopped: null,
  });

  assert.equal(report.state, "PASS");
});

test("buildReport includes state BUILD_FAIL when verification result is FAIL", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: {
      classification: "config_error",
      confidence: 0.9,
      root_causes: [],
    },
    patchArtifact: null,
    verificationArtifact: {
      command: "echo FAIL",
      exit_code: 1,
      stdout_excerpt: "FAIL",
      result: "FAIL",
    },
    patchApplied: false,
    stopped: null,
  });

  assert.equal(report.state, "BUILD_FAIL");
});

test("buildReport includes writtenBack as empty array", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: null,
    patchArtifact: null,
    verificationArtifact: null,
    patchApplied: false,
    stopped: null,
  });

  assert.ok(Array.isArray(report.writtenBack));
  assert.equal(report.writtenBack.length, 0);
});

test("buildReport includes workspace as null", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: null,
    patchArtifact: null,
    verificationArtifact: null,
    patchApplied: false,
    stopped: null,
  });

  assert.equal(report.workspace, null);
});

test("buildReport includes iterations field as a number", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: null,
    patchArtifact: null,
    verificationArtifact: null,
    patchApplied: false,
    stopped: null,
  });

  assert.equal(typeof report.iterations, "number");
});

test("buildReport includes executionMode as structural", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: null,
    patchArtifact: null,
    verificationArtifact: null,
    patchApplied: false,
    stopped: null,
  });

  assert.equal(report.executionMode, "structural");
});

test("buildReport tracks iterationsRun when provided", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: null,
    patchArtifact: null,
    verificationArtifact: null,
    patchApplied: false,
    stopped: null,
    iterationsRun: 3,
  });

  assert.equal(report.iterations, 3);
});

test("buildReport demotes PASS to FAIL when patch proposed but not applied (false-positive guard)", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: {
      classification: "config_error",
      confidence: 0.9,
      root_causes: [],
    },
    patchArtifact: {
      patch: "diff --git a/f\n",
      affected_files: ["f"],
      blast_radius: "low",
      rollback_hint: "git checkout",
    },
    verificationArtifact: {
      command: "echo PASS",
      exit_code: 0,
      stdout_excerpt: "PASS",
      result: "PASS",
    },
    patchApplied: false,
    stopped: null,
  });

  assert.equal(report.outcome, "FAIL");
  assert.equal(report.state, "BUILD_FAIL");
  assert.equal(report.verification.result, "FAIL");
});

// ── Existing fields preserved ─────────────────────────────────────────────────

test("buildReport preserves summary, classification, evidence, patch, verification, risk, next", () => {
  const report = buildReport({
    meta: BASE_META,
    classification: {
      classification: "config_error",
      confidence: 0.9,
      root_causes: [{ hypothesis: "bad key", evidence: "key missing" }],
    },
    patchArtifact: {
      patch: "--- a\n+++ b\n",
      affected_files: ["ci.yml"],
      blast_radius: "low",
      rollback_hint: "git checkout -- .",
    },
    verificationArtifact: {
      command: "echo PASS",
      exit_code: 0,
      stdout_excerpt: "PASS",
      result: "PASS",
    },
    patchApplied: true,
    stopped: null,
  });

  assert.ok(typeof report.summary === "string" && report.summary.length > 0);
  assert.ok(report.classification.label);
  assert.ok(Array.isArray(report.evidence));
  assert.ok(report.evidence.includes("key missing"));
  assert.ok(typeof report.patch === "string");
  assert.ok(report.verification.result === "PASS");
  assert.ok(report.risk.blast_radius);
  assert.ok(typeof report.next === "string");
});
