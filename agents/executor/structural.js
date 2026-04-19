import { readFile, mkdtemp, cp, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import pc from "picocolors";
import { triageLog } from "../triage/index.js";
import { proposeFix } from "../fixer/index.js";
import { applyPatch, runVerification } from "../verifier/index.js";

const MAX_ITERATIONS = 3;

export async function runStructuralScenarioLoop(scenarioPath, meta, config) {
  const absPath = resolve(scenarioPath);
  const logFile = meta.log_file ? join(absPath, meta.log_file) : null;
  const brokenDir = join(absPath, "broken");

  console.log(pc.dim(`\nLoading scenario: ${meta.id}`));
  console.log(
    pc.dim(
      `Category: ${meta.category} | Expected: ${meta.expected_classification}\n`,
    ),
  );

  if (!logFile) {
    return buildReport({
      meta,
      classification: null,
      patchArtifact: null,
      verificationArtifact: {
        command: "N/A",
        exit_code: -1,
        stdout_excerpt: "Scenario has no log_file configured.",
        result: "SKIPPED",
      },
      patchApplied: false,
      stopped: "NEEDS_HUMAN_REVIEW — scenario log_file missing",
    });
  }

  console.log(pc.bold("[1/3] Triaging failure..."));
  const classificationArtifact = await triageLog(
    logFile,
    { category: meta.category },
    config,
  );
  console.log(
    `  Classification: ${pc.yellow(classificationArtifact.classification)} (confidence: ${classificationArtifact.confidence})`,
  );

  if (
    classificationArtifact.needs_more_evidence ||
    classificationArtifact.confidence < 0.7
  ) {
    return buildReport({
      meta,
      classification: classificationArtifact,
      patchArtifact: null,
      verificationArtifact: null,
      patchApplied: false,
      stopped: "NEEDS_HUMAN_REVIEW — confidence too low",
    });
  }

  if (meta.auto_fixable === false) {
    console.log(
      pc.yellow("  Not auto-fixable — providing bootstrap guidance only."),
    );
    console.log(pc.bold("\n[2/3] Fix skipped (auto_fixable: false)"));
    console.log(
      `  → See: ${scenarioPath}/README.md for remediation instructions`,
    );
    console.log(pc.bold("\n[3/3] Verification skipped"));
    console.log(`  → Cannot verify broken state on a functional machine`);
    return buildReport({
      meta,
      classification: classificationArtifact,
      patchArtifact: null,
      verificationArtifact: {
        command: "skipped — requires manual environment setup",
        exit_code: -1,
        stdout_excerpt:
          "Verification skipped: auto_fixable is false. See scenario acceptance.md.",
        result: "SKIPPED",
      },
      patchApplied: false,
      stopped:
        "MANUAL_REMEDIATION_REQUIRED — see scenario README for install instructions",
    });
  }

  const brokenFiles = [];
  for (const relPath of meta.broken_files ?? []) {
    try {
      const content = await readFile(join(brokenDir, relPath), "utf8");
      brokenFiles.push({ path: relPath, content });
    } catch {
      // skip unreadable broken files
    }
  }

  let tmpWorkspace = null;
  try {
    tmpWorkspace = await mkdtemp(join(tmpdir(), `decipher-${meta.id}-`));
    await cp(brokenDir, tmpWorkspace, { recursive: true });
  } catch (err) {
    console.log(pc.dim(`  (workspace setup skipped: ${err.message})`));
    tmpWorkspace = null;
  }

  let patchArtifact = null;
  let verificationArtifact = null;
  let previousPatchSummary = null;
  let patchApplied = false;
  let iterationsRun = 0;

  try {
    for (let iteration = 1; iteration <= MAX_ITERATIONS; iteration++) {
      iterationsRun = iteration;
      console.log(
        pc.bold(
          `\n[2/3] Proposing fix (attempt ${iteration}/${MAX_ITERATIONS})...`,
        ),
      );
      patchArtifact = await proposeFix(
        classificationArtifact,
        { broken_files: brokenFiles },
        config,
      );
      const patchSummary = patchArtifact.affected_files.join(", ");
      console.log(`  Affected: ${patchSummary}`);
      console.log(`  Risk: ${patchArtifact.risk}`);

      if (patchSummary === previousPatchSummary) {
        console.log(pc.yellow("  Same patch attempted twice — stopping."));
        return buildReport({
          meta,
          classification: classificationArtifact,
          patchArtifact,
          verificationArtifact,
          patchApplied,
          stopped: "NEEDS_HUMAN_REVIEW — same patch repeated",
        });
      }
      previousPatchSummary = patchSummary;

      if (patchArtifact.affected_files.length > 2) {
        console.log(
          pc.yellow(
            "  Patch touches more than 2 files — stopping for human review.",
          ),
        );
        return buildReport({
          meta,
          classification: classificationArtifact,
          patchArtifact,
          verificationArtifact,
          patchApplied,
          stopped: "NEEDS_HUMAN_REVIEW — patch scope too large",
        });
      }

      patchApplied = false;
      if (tmpWorkspace && patchArtifact.patch) {
        for (const relPath of meta.broken_files ?? []) {
          const targetFile = join(tmpWorkspace, relPath);
          try {
            await applyPatch(patchArtifact.patch, targetFile);
            patchApplied = true;
          } catch (err) {
            console.log(pc.dim(`  Patch apply note: ${err.message}`));
          }
        }
      }

      console.log(pc.bold("\n[3/3] Verifying fix..."));
      let verifyCmd = meta.verification_command;
      if (tmpWorkspace && patchApplied) {
        verifyCmd = verifyCmd.replace(
          new RegExp(`scenarios/${meta.id}/expected`, "g"),
          tmpWorkspace,
        );
      }

      verificationArtifact = await runVerification(verifyCmd);
      console.log(
        `  Result: ${verificationArtifact.result === "PASS" ? pc.green("PASS") : pc.red("FAIL")}`,
      );

      if (verificationArtifact.result === "PASS") {
        break;
      }
    }
  } finally {
    if (tmpWorkspace) {
      try {
        await rm(tmpWorkspace, { recursive: true, force: true });
      } catch {
        // ignore cleanup errors
      }
    }
  }

  return buildReport({
    meta,
    classification: classificationArtifact,
    patchArtifact,
    verificationArtifact,
    patchApplied,
    stopped: null,
    iterationsRun,
  });
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
