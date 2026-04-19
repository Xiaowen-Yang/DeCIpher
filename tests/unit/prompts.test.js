import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const ROOT = "/Users/doudouda/Downloads/Personal_doc/Study/Proj/DeCIpher";

async function readPrompt(name) {
  return readFile(join(ROOT, "prompts", name), "utf8");
}

test("v2 prompt set includes plan, generate, and clarify contracts", async () => {
  const [plan, generate, clarify, complete] = await Promise.all([
    readPrompt("plan.md"),
    readPrompt("generate.md"),
    readPrompt("clarify.md"),
    readPrompt("complete.md"),
  ]);

  assert.match(plan, /mission/i);
  assert.match(generate, /generate/i);
  assert.match(clarify, /clarif/i);
  assert.match(complete, /complete|completion|stop boundary/i);
});

test("repair prompts reflect mission-oriented v2 positioning", async () => {
  const [triage, fix] = await Promise.all([
    readPrompt("triage.md"),
    readPrompt("fix.md"),
  ]);

  assert.match(triage, /repair subsystem|mission/i);
  assert.match(fix, /repair subsystem|mission/i);
});
