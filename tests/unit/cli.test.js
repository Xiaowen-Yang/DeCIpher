import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";

async function runCliSession(commands, config = {}) {
  const configDir = await mkdtemp(join(tmpdir(), "decipher-cli-test-"));
  await mkdir(configDir, { recursive: true });
  await writeFile(
    join(configDir, "config.json"),
    JSON.stringify({
      provider: "openai",
      model: "gpt-4o",
      api_key: "sk-test-cli",
      approval_policy: "never",
      ...config,
    }, null, 2),
    "utf8",
  );

  return await new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [resolve("bin/decipher")], {
      cwd: resolve("."),
      env: {
        ...process.env,
        DECIPHER_CONFIG_DIR: configDir,
      },
      stdio: ["pipe", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => { stdout += chunk.toString(); });
    child.stderr.on("data", (chunk) => { stderr += chunk.toString(); });
    child.on("error", reject);
    child.on("close", (code) => {
      resolvePromise({ code, stdout, stderr });
    });

    child.stdin.write(commands.join("\n") + "\n");
    child.stdin.end();
  });
}

test("interactive /status shows approval policy and persistence visibility", async () => {
  const result = await runCliSession(["/status", "/quit"]);

  assert.equal(result.code, 0);
  assert.match(result.stdout, /"approval_policy": "never"/);
  assert.match(result.stdout, /"history_path":/);
  assert.match(result.stdout, /"session_path":/);
});

test("interactive /review prints current review snapshot", async () => {
  const result = await runCliSession(["/review", "/quit"]);

  assert.equal(result.code, 0);
  assert.match(result.stdout, /REVIEW/);
  assert.match(result.stdout, /"would_write_back": \[/);
});

test("unknown slash command suggests nearby valid command", async () => {
  const result = await runCliSession(["/settings", "/quit"]);

  assert.equal(result.code, 0);
  assert.match(result.stdout, /Did you mean:/);
  assert.match(result.stdout, /\/setting/);
});
