import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

const {
  resolveTarget,
  askApproval,
  decideResumeAction,
  resolveScenarioRuntime,
} = await import("../../agents/executor/index.js");

test("resolveTarget detects a scenario directory from natural-language input", async () => {
  const root = await mkdtemp(join(tmpdir(), "decipher-executor-"));
  const scenarioDir = join(root, "scenarios", "sample-scenario");
  await mkdir(scenarioDir, { recursive: true });
  await writeFile(
    join(scenarioDir, "metadata.json"),
    JSON.stringify({ id: "sample", category: "docker" }),
    "utf8",
  );

  try {
    const target = await resolveTarget(
      `please repair "${scenarioDir}" build this container`,
    );
    assert.ok(target);
    assert.equal(target.type, "scenario");
    assert.equal(target.meta.id, "sample");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("resolveTarget detects a scenario directory after punctuation in natural-language input", async () => {
  const root = await mkdtemp(join(tmpdir(), "decipher-executor-"));
  const scenarioDir = join(root, "scenarios", "sample-scenario");
  await mkdir(scenarioDir, { recursive: true });
  await writeFile(
    join(scenarioDir, "metadata.json"),
    JSON.stringify({ id: "sample", category: "docker" }),
    "utf8",
  );

  try {
    const target = await resolveTarget(
      `把这个container run起来。${scenarioDir}`,
    );
    assert.ok(target);
    assert.equal(target.type, "scenario");
    assert.equal(target.path, scenarioDir);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("resolveTarget detects a Dockerfile path", async () => {
  const root = await mkdtemp(join(tmpdir(), "decipher-dockerfile-"));
  const dockerfilePath = join(root, "Dockerfile");
  await writeFile(dockerfilePath, "FROM node:20-alpine\n", "utf8");

  try {
    const target = await resolveTarget(`fix this Dockerfile ${dockerfilePath}`);
    assert.ok(target);
    assert.equal(target.type, "dockerfile");
    assert.equal(target.path, dockerfilePath);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("askApproval auto-approves never policy without prompting", async () => {
  let prompted = false;
  const approved = await askApproval(
    {
      question: () => {
        prompted = true;
      },
    },
    {
      approved: false,
      approvalPolicy: "never",
    },
  );

  assert.equal(approved, true);
  assert.equal(prompted, false);
});

test("askApproval auto-approves on-failure policy without prompting", async () => {
  let prompted = false;
  const approved = await askApproval(
    {
      question: () => {
        prompted = true;
      },
    },
    {
      approved: false,
      approvalPolicy: "on-failure",
    },
  );

  assert.equal(approved, true);
  assert.equal(prompted, false);
});

test("askApproval prompts once for on-request policy", async () => {
  let prompts = 0;
  const rl = {
    question: (_prompt, cb) => {
      prompts += 1;
      cb("y");
    },
  };

  const state = { approved: false, approvalPolicy: "on-request" };
  const approved = await askApproval(rl, state);

  assert.equal(approved, true);
  assert.equal(state.approved, true);
  assert.equal(prompts, 1);
});

test("decideResumeAction resumes clarification state without requiring a target path", () => {
  const action = decideResumeAction({
    resumable: true,
    stop_reason: "needs_clarification",
    mission: {
      type: "clarify",
      goal: "help me",
      stop_boundary: "clarified",
    },
    plan: {
      requires_clarification: true,
      clarification_question: "What do you want DeCIpher to do exactly?",
      steps: [],
    },
  });

  assert.equal(action.mode, "clarify");
  assert.match(action.question, /what do you want/i);
});

test("decideResumeAction resumes target execution when a target path exists", () => {
  const action = decideResumeAction({
    resumable: true,
    target_path: "/tmp/scenario",
    last_verification_state: "RUN_FAIL",
  });

  assert.equal(action.mode, "execute_target");
  assert.equal(action.targetPath, "/tmp/scenario");
});

test("resolveScenarioRuntime keeps runtime loop for scenarios with explicit execution_mode", () => {
  const runtime = resolveScenarioRuntime({
    id: "docker-copy-path-bug",
    category: "docker",
    execution_mode: "docker_run",
    broken_files: ["Dockerfile"],
  });

  assert.equal(runtime.kind, "runtime");
  assert.equal(runtime.meta.execution_mode, "docker_run");
});

test("resolveScenarioRuntime upgrades legacy docker scenarios into docker_build runtime loops", () => {
  const runtime = resolveScenarioRuntime({
    id: "docker-entrypoint-permission",
    category: "docker",
    broken_files: ["Dockerfile"],
  });

  assert.equal(runtime.kind, "runtime");
  assert.equal(runtime.meta.execution_mode, "docker_build");
  assert.deepEqual(runtime.meta.repair_target_files, ["Dockerfile"]);
});

test("resolveScenarioRuntime keeps ci scenarios on the structural repair loop", () => {
  const runtime = resolveScenarioRuntime({
    id: "ci-python-version-drift",
    category: "ci",
    auto_fixable: true,
    broken_files: [".github/workflows/ci.yml"],
  });

  assert.equal(runtime.kind, "structural");
});

test("resolveScenarioRuntime keeps non-auto-fixable env scenarios on the structural/manual path", () => {
  const runtime = resolveScenarioRuntime({
    id: "env-missing-node",
    category: "env",
    auto_fixable: false,
    broken_files: [".nvmrc", "package.json"],
  });

  assert.equal(runtime.kind, "structural");
});
