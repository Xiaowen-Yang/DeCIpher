import { test } from "node:test";
import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.DECIPHER_CONFIG_DIR = join(tmpdir(), `decipher-cli-test-${Date.now()}`);

const {
  buildCliStatusSnapshot,
  buildCliReviewSnapshot,
  suggestCliSlashCommand,
} = await import("../../lib/cli-surface.js");

test("status snapshot shows approval policy and persistence visibility", () => {
  const snapshot = buildCliStatusSnapshot(
    {
      provider: "openai",
      model: "gpt-4o",
      base_url: null,
      approval_policy: "never",
      max_iterations: 3,
    },
    {
      approved: false,
      approvalPolicy: "never",
      currentTarget: null,
      lastVerificationResult: null,
      lastRunResult: null,
    },
  );

  assert.equal(snapshot.approval_policy, "never");
  assert.match(snapshot.persistence.history_path, /history\.jsonl$/);
  assert.match(snapshot.persistence.session_path, /session\.json$/);
});

test("review snapshot includes would_write_back and patch preview", () => {
  const review = buildCliReviewSnapshot({
    currentTarget: { path: "/tmp/scenario" },
    lastVerificationResult: "PASS",
    lastRunResult: {
      workspace: "/tmp/workspace",
      writtenBack: ["Dockerfile"],
      patch: "--- a/Dockerfile\n+++ b/Dockerfile\n@@ -1 +1 @@\n-foo\n+bar",
      classification: {
        classification: "path_or_copy_error",
        confidence: 0.95,
      },
    },
  });

  assert.deepEqual(review.would_write_back, ["Dockerfile"]);
  assert.equal(review.classification, "path_or_copy_error");
  assert.match(review.patch_preview, /Dockerfile/);
});

test("unknown slash command suggests nearby valid command", () => {
  assert.equal(suggestCliSlashCommand("settings"), "/setting");
});
