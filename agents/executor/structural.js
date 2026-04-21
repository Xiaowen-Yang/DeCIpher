import { readFile, mkdtemp, cp, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import pc from "picocolors";

const MAX_ITERATIONS = 3;

// NOTE: runStructuralScenarioLoop was removed in R5 (secondary JS agents deleted).
// The function is preserved as a stub to avoid breaking any references.
export async function runStructuralScenarioLoop(_scenarioPath, _meta, _config) {
  throw new Error(
    "runStructuralScenarioLoop is unavailable: secondary JS agents were removed in R5. " +
      "Use the Rust-native AgentLoop instead.",
  );
}

function mapOutcome(result, stopped) {
  if (stopped && stopped.includes("NEEDS_HUMAN_REVIEW"))
    return "NEEDS_HUMAN_REVIEW";
  if (result === "PASS") return "PASS";
  if (result === "NOT_RUN" || result === "SKIPPED") return "NEEDS_HUMAN_REVIEW";
  return "FAIL";
}

function mapState(result, stopped) {
  if (stopped && stopped.includes("NEEDS_HUMAN_REVIEW"))
    return "NEEDS_HUMAN_REVIEW";
  if (result === "PASS") return "PASS";
  if (result === "FAIL") return "BUILD_FAIL";
  return result ?? "NOT_RUN";
}

export function buildReport({
  meta,
  classification,
  patchArtifact,
  verificationArtifact,
  patchApplied,
  stopped,
  iterationsRun = 1,
}) {
  const evidenceLines = (classification?.root_causes ?? [])
    .filter((rc) => rc.evidence)
    .map((rc) => rc.evidence);

  const patch = patchArtifact?.patch ?? "";
  const result = verificationArtifact?.result ?? "NOT_RUN";

  // False-positive guard: if a patch was proposed but never applied and the
  // verification still reports PASS, this is a false positive — the original
  // broken state cannot legitimately pass a targeted verification command.
  const effectiveResult =
    result === "PASS" &&
    patchArtifact?.patch &&
    !patchApplied &&
    stopped == null
      ? "FAIL"
      : result;

  return {
    // LoopResult-compatible fields
    outcome: mapOutcome(effectiveResult, stopped),
    state: mapState(effectiveResult, stopped),
    writtenBack: [],
    workspace: null,
    iterations: iterationsRun,
    executionMode: "structural",

    // Structural-specific fields
    summary: stopped
      ? stopped
      : effectiveResult === "PASS"
        ? `${meta.id}: ${classification.classification} — fix applied and verified.`
        : `${meta.id}: fix applied but verification failed.`,

    classification: {
      label: classification?.classification ?? "unknown",
      confidence: classification?.confidence ?? 0,
    },

    evidence: evidenceLines,
    patch,
    patch_applied: patchApplied,

    verification: {
      command:
        verificationArtifact?.command ?? meta.verification_command ?? "N/A",
      exit_code: verificationArtifact?.exit_code ?? -1,
      result: effectiveResult,
      excerpt: verificationArtifact?.stdout_excerpt ?? "",
    },

    risk: {
      blast_radius: patchArtifact?.blast_radius ?? "unknown",
      rollback_hint: patchArtifact?.rollback_hint ?? "git checkout -- .",
    },

    next: stopped
      ? "Human review required. See logs for details."
      : effectiveResult === "PASS"
        ? "Commit the fix and re-run your CI pipeline."
        : "Manual intervention required — automated fix unsuccessful.",
  };
}
