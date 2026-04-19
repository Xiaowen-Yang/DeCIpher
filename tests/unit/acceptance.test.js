/**
 * Acceptance runner tests.
 *
 * Validates the greenfield scenario verification engine:
 * file_exists, file_contains, command checks.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

const { runAcceptanceChecks, loadAcceptanceChecks } = await import(
  "../../agents/verifier/acceptance.js"
);

// ── file_exists checks ──────────────────────────────────────────────────────

test("file_exists passes when the file exists", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));
  await writeFile(join(dir, "Dockerfile"), "FROM ubuntu:22.04\n");

  try {
    const report = await runAcceptanceChecks(
      [{ id: "df", type: "file_exists", path: "Dockerfile", description: "Dockerfile exists" }],
      dir,
    );
    assert.equal(report.passed, true);
    assert.equal(report.results[0].passed, true);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("file_exists fails when the file is missing", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));

  try {
    const report = await runAcceptanceChecks(
      [{ id: "df", type: "file_exists", path: "Dockerfile", description: "Dockerfile exists" }],
      dir,
    );
    assert.equal(report.passed, false);
    assert.equal(report.results[0].passed, false);
    assert.match(report.results[0].detail, /not found/i);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

// ── file_contains checks ────────────────────────────────────────────────────

test("file_contains passes when substring is found", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));
  await writeFile(join(dir, "Dockerfile"), "FROM ubuntu:22.04\nRUN apt-get install hpcc\n");

  try {
    const report = await runAcceptanceChecks(
      [{ id: "hpcc", type: "file_contains", path: "Dockerfile", contains: "hpcc", description: "Has hpcc" }],
      dir,
    );
    assert.equal(report.passed, true);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("file_contains fails when substring is not found", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));
  await writeFile(join(dir, "Dockerfile"), "FROM ubuntu:22.04\n");

  try {
    const report = await runAcceptanceChecks(
      [{ id: "hpcc", type: "file_contains", path: "Dockerfile", contains: "hpcc", description: "Has hpcc" }],
      dir,
    );
    assert.equal(report.passed, false);
    assert.match(report.results[0].detail, /does not contain/i);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

// ── command checks ──────────────────────────────────────────────────────────

test("command check passes when exit code is 0", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));

  try {
    const report = await runAcceptanceChecks(
      [{ id: "echo", type: "command", command: "echo hello", expect_exit: 0, description: "Echo works" }],
      dir,
    );
    assert.equal(report.passed, true);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("command check fails when exit code is non-zero", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));

  try {
    const report = await runAcceptanceChecks(
      [{ id: "fail", type: "command", command: "sh -c 'exit 1'", expect_exit: 0, description: "Should pass" }],
      dir,
    );
    assert.equal(report.passed, false);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("command check validates stdout contains", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));

  try {
    const report = await runAcceptanceChecks(
      [{ id: "grep", type: "command", command: "echo container_ok", expect_stdout_contains: "container_ok", description: "Output check" }],
      dir,
    );
    assert.equal(report.passed, true);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("command check fails when stdout does not contain expected string", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));

  try {
    const report = await runAcceptanceChecks(
      [{ id: "grep", type: "command", command: "echo wrong_output", expect_stdout_contains: "container_ok", description: "Output check" }],
      dir,
    );
    assert.equal(report.passed, false);
    assert.match(report.results[0].detail, /does not contain/i);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

// ── Mixed checks ────────────────────────────────────────────────────────────

test("mixed checks report partial pass count correctly", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));
  await writeFile(join(dir, "Dockerfile"), "FROM ubuntu:22.04\n");

  try {
    const report = await runAcceptanceChecks(
      [
        { id: "exists", type: "file_exists", path: "Dockerfile", description: "File exists" },
        { id: "missing", type: "file_exists", path: "run.sh", description: "Script exists" },
        { id: "cmd", type: "command", command: "echo ok", expect_exit: 0, description: "Cmd ok" },
      ],
      dir,
    );
    assert.equal(report.passed, false);
    assert.equal(report.total, 3);
    assert.equal(report.pass_count, 2);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

// ── loadAcceptanceChecks ────────────────────────────────────────────────────

test("loadAcceptanceChecks reads checks from acceptance.json", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-accept-"));
  await writeFile(
    join(dir, "acceptance.json"),
    JSON.stringify({
      checks: [
        { id: "test", type: "file_exists", path: "x.txt", description: "X exists" },
      ],
    }),
  );

  try {
    const checks = await loadAcceptanceChecks(dir);
    assert.equal(checks.length, 1);
    assert.equal(checks[0].id, "test");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

// ── Greenfield scenario routing ─────────────────────────────────────────────

test("resolveScenarioRuntime routes greenfield scenarios to runtime kind", async () => {
  const { resolveScenarioRuntime } = await import(
    "../../agents/executor/index.js"
  );
  const runtime = resolveScenarioRuntime({
    id: "hpl-from-scratch",
    category: "docker",
    start_state: "empty",
    mission_type: "greenfield",
  });

  assert.equal(runtime.kind, "runtime");
  assert.equal(runtime.meta.execution_mode, "agent");
});

test("createMissionPlan builds greenfield steps for greenfield missions", async () => {
  const { createMissionPlan } = await import(
    "../../agents/planner/index.js"
  );
  const plan = createMissionPlan({
    type: "greenfield",
    goal: "Run HPL in Docker from scratch",
    domain: "container",
    stop_boundary: "benchmark_completed",
  });

  assert.equal(plan.selected_subsystem, "generation");
  assert.ok(plan.steps.some((s) => s.id === "understand_goal"));
  assert.ok(plan.steps.some((s) => s.id === "generate_all_files"));
  assert.ok(plan.steps.some((s) => s.id === "build_and_test"));
  assert.ok(plan.steps.some((s) => s.id === "debug_and_iterate"));
  assert.ok(plan.steps.some((s) => s.id === "verify_outcome"));
});

test("createEmptyWorkspace creates an empty temp directory", async () => {
  const { createEmptyWorkspace } = await import(
    "../../agents/executor/workspace.js"
  );
  const { readdir, rm: rmDir } = await import("node:fs/promises");

  const ws = await createEmptyWorkspace("test-greenfield");
  try {
    const files = await readdir(ws);
    assert.equal(files.length, 0);
    assert.match(ws, /test-greenfield/);
  } finally {
    await rmDir(ws, { recursive: true, force: true });
  }
});
