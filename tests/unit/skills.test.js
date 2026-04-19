import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const ROOT = "/Users/doudouda/Downloads/Personal_doc/Study/Proj/DeCIpher";

async function readSkill(name) {
  return readFile(join(ROOT, "skills", name, "SKILL.md"), "utf8");
}

test("v2 skill set frames domain knowledge as subsystems or mission support, not the full product", async () => {
  const [ci, docker, env, verify] = await Promise.all([
    readSkill("ci-triage"),
    readSkill("docker-debug"),
    readSkill("env-bootstrap"),
    readSkill("verify-gate"),
  ]);

  assert.match(ci, /repair subsystem|mission/i);
  assert.match(docker, /repair subsystem|mission/i);
  assert.match(env, /generation subsystem|mission|bootstrap/i);
  assert.match(verify, /mission|verification layer/i);
});
