/**
 * Git repo URL detection and clone_and_run flow tests.
 */
import { test } from "node:test";
import assert from "node:assert/strict";

const { resolveTarget, extractGitUrl } = await import(
  "../../agents/executor/index.js"
);
const { createMissionPlan } = await import(
  "../../agents/planner/index.js"
);

// ── URL detection ────────────────────────────────────────────────────────────

test("resolveTarget detects GitHub HTTPS URL and returns git_repo target", async () => {
  const target = await resolveTarget(
    "https://github.com/docker/getting-started-app I want to run this",
  );
  assert.equal(target.type, "git_repo");
  assert.equal(target.meta.repoName, "getting-started-app");
  assert.equal(target.meta.host, "github.com");
  assert.equal(target.meta.start_state, "empty");
  assert.equal(target.meta.mission_type, "clone_and_run");
});

test("resolveTarget detects GitHub URL with .git suffix", async () => {
  const target = await resolveTarget(
    "https://github.com/user/myapp.git build this",
  );
  assert.equal(target.type, "git_repo");
  assert.equal(target.meta.repoName, "myapp");
});

test("resolveTarget detects GitLab URL", async () => {
  const target = await resolveTarget(
    "run https://gitlab.com/team/project in Docker",
  );
  assert.equal(target.type, "git_repo");
  assert.equal(target.meta.host, "gitlab.com");
});

test("extractGitUrl parses SSH format", () => {
  const info = extractGitUrl("git@github.com:org/repo.git run this");
  assert.equal(info.repoName, "repo");
  assert.equal(info.host, "github.com");
});

test("extractGitUrl returns null for non-git URLs", () => {
  assert.equal(extractGitUrl("fix the Dockerfile"), null);
  assert.equal(extractGitUrl("https://example.com/page"), null);
});

test("resolveTarget prefers git URL over file paths in mixed input", async () => {
  const target = await resolveTarget(
    'fix https://github.com/user/app and also check ./Dockerfile',
  );
  assert.equal(target.type, "git_repo");
  assert.equal(target.meta.repoName, "app");
});

// ── Planner ──────────────────────────────────────────────────────────────────

test("createMissionPlan builds clone_and_run steps", () => {
  const plan = createMissionPlan({
    type: "clone_and_run",
    goal: "Clone and run in Docker",
    domain: "container",
    stop_boundary: "container_running",
  });
  assert.equal(plan.selected_subsystem, "generation");
  assert.ok(plan.steps.some((s) => s.id === "clone_repo"));
  assert.ok(plan.steps.some((s) => s.id === "read_readme"));
  assert.ok(plan.steps.some((s) => s.id === "build_image"));
  assert.ok(plan.steps.some((s) => s.id === "run_container"));
  assert.ok(plan.steps.some((s) => s.id === "verify_running"));
});
