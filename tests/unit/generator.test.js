import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, mkdir, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

const { parseGenerateResponse, buildContextSummary } = await import(
  "../../agents/executor/generator.js"
);

// ── parseGenerateResponse ─────────────────────────────────────────────────────

test("parseGenerateResponse parses valid JSON with generated_files", () => {
  const raw = JSON.stringify({
    generated_files: [{ path: "Dockerfile", content: "FROM ubuntu:22.04\n" }],
    rationale: "Standard Dockerfile for ubuntu base image",
    needs_clarification: null,
  });

  const result = parseGenerateResponse(raw);

  assert.equal(result.generated_files.length, 1);
  assert.equal(result.generated_files[0].path, "Dockerfile");
  assert.equal(result.generated_files[0].content, "FROM ubuntu:22.04\n");
  assert.equal(result.rationale, "Standard Dockerfile for ubuntu base image");
  assert.equal(result.needs_clarification, null);
});

test("parseGenerateResponse strips markdown code fences before parsing", () => {
  const raw =
    "```json\n" +
    JSON.stringify({
      generated_files: [{ path: "ci.yml", content: "name: CI\n" }],
      rationale: "minimal workflow",
      needs_clarification: null,
    }) +
    "\n```";

  const result = parseGenerateResponse(raw);

  assert.equal(result.generated_files.length, 1);
  assert.equal(result.generated_files[0].path, "ci.yml");
});

test("parseGenerateResponse handles needs_clarification response", () => {
  const raw = JSON.stringify({
    generated_files: [],
    rationale: "",
    needs_clarification: "Which base OS should the Dockerfile target?",
  });

  const result = parseGenerateResponse(raw);

  assert.equal(result.generated_files.length, 0);
  assert.ok(result.needs_clarification);
  assert.match(result.needs_clarification, /base OS/i);
});

test("parseGenerateResponse returns safe defaults on invalid JSON", () => {
  const raw = "not valid json at all";

  const result = parseGenerateResponse(raw);

  assert.deepEqual(result.generated_files, []);
  assert.equal(typeof result.needs_clarification, "string");
  assert.ok(result.needs_clarification.length > 0);
});

test("parseGenerateResponse handles missing needs_clarification field as null", () => {
  const raw = JSON.stringify({
    generated_files: [{ path: "Makefile", content: "all:\n\t echo ok\n" }],
    rationale: "basic Makefile",
  });

  const result = parseGenerateResponse(raw);

  assert.equal(result.needs_clarification, null);
  assert.equal(result.generated_files.length, 1);
});

// ── buildContextSummary ───────────────────────────────────────────────────────

test("buildContextSummary returns a non-empty string for a directory with files", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-gen-ctx-"));
  try {
    await writeFile(join(dir, "package.json"), '{"name":"test"}', "utf8");
    await writeFile(join(dir, "index.js"), "console.log(1);", "utf8");

    const summary = await buildContextSummary(dir);

    assert.ok(typeof summary === "string");
    assert.ok(summary.length > 0);
    assert.ok(summary.includes("package.json"));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("buildContextSummary surfaces Dockerfile content in summary", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-gen-ctx-"));
  try {
    await writeFile(
      join(dir, "Dockerfile"),
      "FROM node:20-alpine\nWORKDIR /app\n",
      "utf8",
    );

    const summary = await buildContextSummary(dir);

    assert.ok(summary.includes("FROM node:20-alpine"));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("buildContextSummary handles an empty directory without throwing", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-gen-ctx-empty-"));
  try {
    const summary = await buildContextSummary(dir);

    assert.ok(typeof summary === "string");
    assert.ok(summary.includes("empty") || summary.includes(dir));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("buildContextSummary handles a missing directory gracefully", async () => {
  const summary = await buildContextSummary("/tmp/decipher-definitely-does-not-exist-xyzzy");

  assert.ok(typeof summary === "string");
  assert.ok(summary.includes("unreadable") || summary.includes("missing"));
});
