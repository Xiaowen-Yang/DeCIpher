import { test } from "node:test";
import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.DECIPHER_CONFIG_DIR = join(tmpdir(), `decipher-cli-test-${Date.now()}`);

const {
  buildCliModelView,
  buildCliSettingsView,
  buildCliStatusSnapshot,
  buildCliReviewSnapshot,
  buildCliTranscriptView,
  buildCliArtifactsView,
  buildCliPlanView,
  suggestCliSlashCommand,
  decideCliInteraction,
} = await import("../../lib/cli-surface.js");

test("model view shows only model-scoped config and masks the api key", () => {
  const view = buildCliModelView({
    provider: "custom",
    model: "deepseek-v3-2-251201",
    base_url: "https://example.test/v1",
    api_key: "sk-1234567890",
    approval_policy: "never",
  });

  assert.deepEqual(view, {
    provider: "custom",
    model: "deepseek-v3-2-251201",
    base_url: "https://example.test/v1",
    api_key: "sk-***90",
  });
});

test("settings view includes masked config plus execution visibility", () => {
  const view = buildCliSettingsView(
    {
      provider: "openai",
      model: "gpt-4o",
      base_url: null,
      api_key: "sk-1234567890",
      approval_policy: "on-request",
      max_iterations: 3,
      notification_command: null,
    },
    {
      lastRunResult: {
        workspace: "/tmp/workspace",
      },
    },
    null,
  );

  assert.equal(view.provider, "openai");
  assert.equal(view.api_key, "sk-***90");
  assert.equal(view.approval_policy, "on-request");
  assert.equal(view.max_iterations, 3);
  assert.equal(view.execution_visibility.temp_workspace_path, "/tmp/workspace");
  assert.match(view.execution_visibility.persistence.history_path, /history\.jsonl$/);
});

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

test("status snapshot surfaces current mission summary when available", () => {
  const snapshot = buildCliStatusSnapshot(
    {
      provider: "openai",
      model: "gpt-4o",
      base_url: null,
      approval_policy: "never",
      max_iterations: 5,
    },
    {
      approved: true,
      approvalPolicy: "never",
      currentTarget: { path: "/tmp/workload" },
      currentMission: {
        type: "benchmark_run",
        goal: "Run the HPL benchmark on this machine",
        stop_boundary: "benchmark_completed",
      },
      lastVerificationResult: "RUN_FAIL",
      lastRunResult: null,
    },
  );

  assert.equal(snapshot.mission_type, "benchmark_run");
  assert.equal(snapshot.mission_goal, "Run the HPL benchmark on this machine");
  assert.equal(snapshot.mission_stop_boundary, "benchmark_completed");
});

test("status snapshot surfaces resumable clarification and plan breadcrumbs from persisted state", () => {
  const snapshot = buildCliStatusSnapshot(
    {
      provider: "openai",
      model: "gpt-4o",
      base_url: null,
      approval_policy: "on-request",
      max_iterations: 5,
    },
    {
      approved: false,
      approvalPolicy: "on-request",
      currentTarget: null,
      currentMission: null,
      lastVerificationResult: null,
      lastRunResult: null,
    },
    {
      target_path: null,
      summary: {
        mission_type: "clarify",
        mission_summary: "Help me with this workload",
        mission_stop_boundary: "clarified",
        plan_step_ids: ["inspect_target"],
        requires_clarification: true,
        clarification_question: "What do you want DeCIpher to do exactly?",
      },
      stop_reason: "needs_clarification",
      resumable: true,
    },
  );

  assert.equal(snapshot.resumable, true);
  assert.equal(snapshot.stop_reason, "needs_clarification");
  assert.deepEqual(snapshot.plan_step_ids, ["inspect_target"]);
  assert.equal(snapshot.requires_clarification, true);
  assert.match(snapshot.clarification_question, /what do you want/i);
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

test("review snapshot includes mission context and clarification state from persisted session", () => {
  const review = buildCliReviewSnapshot(
    {
      currentTarget: null,
      currentMission: null,
      currentPlan: null,
      lastVerificationResult: null,
      lastRunResult: null,
    },
    {
      target_path: "/tmp/workload",
      mission_summary: "Build and start the container",
      plan: {
        requires_clarification: true,
        clarification_question: "Which image tag should DeCIpher use?",
      },
      last_verification_state: "RUN_FAIL",
      workspace_path: "/tmp/workspace",
      written_back: [],
      patch: "--- a/Dockerfile\n+++ b/Dockerfile",
    },
  );

  assert.equal(review.mission_goal, "Build and start the container");
  assert.equal(review.requires_clarification, true);
  assert.match(review.clarification_question, /image tag/i);
});

test("transcript view prefers in-memory transcript and mission context", () => {
  const view = buildCliTranscriptView({
    currentTarget: { path: "/tmp/workload" },
    currentMission: { goal: "Build and start the container" },
    lastVerificationResult: "RUN_FAIL",
    lastRunResult: {
      transcript: "[executor] build failed\nclassification: path_or_copy_error",
      transcriptPath: "/tmp/workspace/transcript.log",
    },
  });

  assert.equal(view.target, "/tmp/workload");
  assert.equal(view.mission_goal, "Build and start the container");
  assert.equal(view.last_state, "RUN_FAIL");
  assert.equal(view.transcript_path, "/tmp/workspace/transcript.log");
  assert.match(view.transcript, /build failed/);
});

test("transcript view falls back to persisted transcript fields", () => {
  const view = buildCliTranscriptView(
    {
      currentTarget: null,
      currentMission: null,
      lastVerificationResult: null,
      lastRunResult: null,
    },
    {
      target_path: "/tmp/workload",
      mission_summary: "Repair the container build",
      last_verification_state: "BUILD_FAIL",
      artifact_refs: {
        transcript_path: "/tmp/transcript.log",
      },
      transcript: "[executor] reproduced build failure",
    },
  );

  assert.equal(view.target, "/tmp/workload");
  assert.equal(view.mission_goal, "Repair the container build");
  assert.equal(view.last_state, "BUILD_FAIL");
  assert.equal(view.transcript_path, "/tmp/transcript.log");
  assert.match(view.transcript, /reproduced build failure/);
});

test("artifacts view surfaces workspace, artifact refs, and patch preview", () => {
  const view = buildCliArtifactsView(
    {
      currentTarget: { path: "/tmp/workload" },
      currentMission: { goal: "Build the container" },
      lastVerificationResult: "RUN_FAIL",
      lastRunResult: {
        workspace: "/tmp/workspace",
        writtenBack: ["Dockerfile"],
        preservedArtifacts: {
          image_tag: "decipher-img",
        },
        artifactRefs: {
          workspace_path: "/tmp/workspace",
          transcript_path: "/tmp/workspace/transcript.log",
        },
        patch: "--- a/Dockerfile\n+++ b/Dockerfile\n@@ -1 +1 @@\n-FROM node:18\n+FROM node:20",
      },
    },
    null,
  );

  assert.equal(view.target, "/tmp/workload");
  assert.equal(view.workspace, "/tmp/workspace");
  assert.deepEqual(view.written_back, ["Dockerfile"]);
  assert.deepEqual(view.preserved_artifacts, { image_tag: "decipher-img" });
  assert.equal(view.artifact_refs.transcript_path, "/tmp/workspace/transcript.log");
  assert.match(view.patch_preview, /Dockerfile/);
});

test("plan view surfaces mission summary and clarification question when clarification is pending", () => {
  const view = buildCliPlanView(
    {
      currentPlan: {
        requires_clarification: true,
        clarification_question: "Which benchmark should DeCIpher run?",
        steps: [],
      },
      currentMission: {
        goal: "Help me run a benchmark workload",
      },
    },
    null,
  );

  assert.match(view, /Help me run a benchmark workload/);
  assert.match(view, /Which benchmark should DeCIpher run\?/);
});

test("plan view renders mission plan steps from persisted state", () => {
  const view = buildCliPlanView(
    {
      currentPlan: null,
      currentMission: null,
      currentTarget: null,
      lastVerificationResult: null,
      lastRunResult: null,
    },
    {
      mission_summary: "Build and start the container",
      plan: {
        requires_clarification: false,
        steps: [
          { id: "inspect_target", label: "Inspect target and environment" },
          { id: "build_container", label: "Build the container" },
        ],
      },
    },
  );

  assert.match(view, /Build and start the container/);
  assert.match(view, /Inspect target and environment/);
  assert.match(view, /Build the container/);
});

test("unknown slash command suggests nearby valid command", () => {
  assert.equal(suggestCliSlashCommand("settings"), "/setting");
});

test("decideCliInteraction requests clarification for ambiguous missions", () => {
  const decision = decideCliInteraction({
    route: {
      mode: "clarify",
      question: "What do you want DeCIpher to do exactly?",
    },
    input: "please help",
  });

  assert.equal(decision.mode, "clarify");
  assert.match(decision.question, /what do you want/i);
});

test("decideCliInteraction keeps target execution when mission is actionable", () => {
  const decision = decideCliInteraction({
    route: {
      mode: "execute_target",
      action: "fix",
    },
    input: "fix this scenario",
  });

  assert.equal(decision.mode, "execute_target");
  assert.equal(decision.action, "fix");
});

test("decideCliInteraction keeps explicit questions on the conversational path", () => {
  const decision = decideCliInteraction({
    route: null,
    input: "what does /setting do?",
  });

  assert.equal(decision.mode, "conversation");
});

test("decideCliInteraction defaults non-question free text back into mission clarification", () => {
  const decision = decideCliInteraction({
    route: null,
    input: "deploy this workload",
  });

  assert.equal(decision.mode, "clarify");
  assert.match(decision.question, /build, run, repair, or generate/i);
});
