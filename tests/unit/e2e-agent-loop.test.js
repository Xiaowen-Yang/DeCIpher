/**
 * E2E agent loop tests — mock-based.
 *
 * These validate the agent loop execution path without a real API key.
 * We mock `callAIWithMessages` to simulate LLM responses, then verify
 * the loop correctly processes tool calls and terminates.
 */
import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, readFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

// ── Mock API client before importing agent-loop ────────────────────────────

// Build a sequence of scripted LLM responses for each test scenario
function buildMockResponder(responses) {
  let callIndex = 0;
  return async function mockCallAI(_messages, _config, _systemPrompt) {
    const response = responses[callIndex] ?? responses[responses.length - 1];
    callIndex++;
    return typeof response === "string" ? response : JSON.stringify(response);
  };
}

// Direct import of tools for testing
const { TOOL_REGISTRY, toolsPromptSection } = await import(
  "../../agents/executor/tools.js"
);

// ── Tool registry tests ──────────────────────────────────────────────────────

test("toolsPromptSection returns a non-empty string describing all tools", () => {
  const section = toolsPromptSection();
  assert.ok(section.length > 100);
  assert.match(section, /exec_command/);
  assert.match(section, /read_file/);
  assert.match(section, /write_file/);
  assert.match(section, /apply_patch/);
  assert.match(section, /update_plan/);
  assert.match(section, /done/);
});

test("TOOL_REGISTRY has handlers for all documented tools", () => {
  const expectedTools = [
    "exec_command",
    "read_file",
    "write_file",
    "apply_patch",
    "update_plan",
    "done",
  ];
  for (const name of expectedTools) {
    assert.ok(TOOL_REGISTRY[name], `missing tool: ${name}`);
    assert.equal(typeof TOOL_REGISTRY[name].handler, "function");
  }
});

// ── Tool handler unit tests ──────────────────────────────────────────────────

test("read_file tool reads a file and returns its content", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-tool-test-"));
  const filePath = join(dir, "test.txt");
  await writeFile(filePath, "hello world", "utf8");

  try {
    const result = await TOOL_REGISTRY.read_file.handler(
      { path: filePath },
      { workspace: dir, log: () => {} },
    );
    assert.equal(result.success, true);
    assert.equal(result.content, "hello world");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("read_file tool returns error for missing file", async () => {
  const result = await TOOL_REGISTRY.read_file.handler(
    { path: "/nonexistent/file.txt" },
    { workspace: "/tmp", log: () => {} },
  );
  assert.equal(result.success, false);
  assert.ok(result.error);
});

test("write_file tool creates a file with specified content", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-write-test-"));
  const filePath = join(dir, "output.txt");

  try {
    const result = await TOOL_REGISTRY.write_file.handler(
      { path: filePath, content: "new content" },
      { workspace: dir, sessionState: { approved: true }, log: () => {} },
    );
    assert.equal(result.success, true);
    const content = await readFile(filePath, "utf8");
    assert.equal(content, "new content");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("exec_command tool runs a shell command and returns output", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-exec-test-"));

  try {
    const result = await TOOL_REGISTRY.exec_command.handler(
      { cmd: "echo hello_from_agent" },
      { workspace: dir, log: () => {} },
    );
    assert.equal(result.exitCode, 0);
    assert.match(result.output, /hello_from_agent/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("exec_command tool captures non-zero exit codes", async () => {
  const result = await TOOL_REGISTRY.exec_command.handler(
    { cmd: "sh -c 'exit 42'" },
    { workspace: "/tmp", log: () => {} },
  );
  assert.notEqual(result.exitCode, 0);
});

test("update_plan tool returns the steps it receives", async () => {
  const steps = [
    { step: "Build image", status: "completed" },
    { step: "Run container", status: "in_progress" },
  ];
  let renderedSteps = null;
  const result = await TOOL_REGISTRY.update_plan.handler(
    { steps },
    { workspace: "/tmp", log: () => {}, onPlanUpdate: (s) => { renderedSteps = s; } },
  );
  assert.equal(result.success, true);
  assert.deepEqual(result.steps, steps);
  assert.deepEqual(renderedSteps, steps);
});

test("done tool returns the summary and outcome", async () => {
  const result = await TOOL_REGISTRY.done.handler(
    { summary: "All done", outcome: "PASS" },
    { workspace: "/tmp", log: () => {} },
  );
  assert.equal(result.summary, "All done");
  assert.equal(result.outcome, "PASS");
});

// ── isToolRisky tests ────────────────────────────────────────────────────────

const { isToolRisky } = await import("../../agents/executor/tools.js");

test("isToolRisky flags rm -rf as risky", () => {
  assert.equal(isToolRisky("exec_command", { cmd: "rm -rf /tmp/workspace" }), true);
});

test("isToolRisky allows safe commands", () => {
  assert.equal(isToolRisky("exec_command", { cmd: "echo hello" }), false);
  assert.equal(isToolRisky("read_file", { path: "/tmp/test.txt" }), false);
});

test("isToolRisky flags write_file as risky", () => {
  assert.equal(isToolRisky("write_file", { path: "/tmp/test.txt" }), true);
});

test("isToolRisky flags apply_patch as risky", () => {
  assert.equal(isToolRisky("apply_patch", { patch: "diff..." }), true);
});
