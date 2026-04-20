import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";
import { persistMissionMemory } from "./mission-memory.js";

const decipherHome =
  process.env.DECIPHER_CONFIG_DIR ?? join(homedir(), ".decipher");
const sessionStorePath = join(decipherHome, "session.json");

const SECRET_KEY_PATTERN = /(api[_-]?key|token|secret|authorization|password)/i;

export function getSessionStorePath() {
  return sessionStorePath;
}

function maskSecret(value) {
  if (value == null) return value;

  const text = String(value);
  if (text.length < 6) return "***";

  return `${text.slice(0, 2)}-***${text.slice(-2)}`.replace("--***", "-***");
}

function maskValue(key, value) {
  if (value == null) return value;

  if (SECRET_KEY_PATTERN.test(key)) {
    return maskSecret(value);
  }

  if (Array.isArray(value)) {
    return value.map((item) => maskValue("", item));
  }

  if (typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([entryKey, entryValue]) => [
        entryKey,
        maskValue(entryKey, entryValue),
      ]),
    );
  }

  return value;
}

export function maskSessionSnapshot(snapshot) {
  if (snapshot == null) return snapshot;
  return maskValue("", snapshot);
}

export function formatSessionSnapshot(snapshot, options = {}) {
  const { public: usePublicSnapshot = false } = options;
  const value = usePublicSnapshot ? maskSessionSnapshot(snapshot) : snapshot;
  return JSON.stringify(value, null, 2);
}

export function buildSessionSummary(snapshot = {}) {
  const summary = {
    target_path: snapshot.target_path ?? null,
    target_type: snapshot.target_type ?? null,
    scenario_id: snapshot.scenario_id ?? null,
    execution_mode: snapshot.execution_mode ?? null,
    iteration: snapshot.iteration ?? 0,
    last_verification_state: snapshot.last_verification_state ?? null,
    stop_reason: snapshot.stop_reason ?? null,
    classification: snapshot.classification?.classification ?? null,
    confidence: snapshot.classification?.confidence ?? null,
    files_touched: snapshot.repair_target_files ?? [],
    files_written_back: snapshot.written_back ?? [],
  };

  const missionType = snapshot.mission?.type ?? snapshot.mission_type;
  if (missionType) {
    summary.mission_type = missionType;
  }

  const missionStopBoundary =
    snapshot.mission?.stop_boundary ?? snapshot.mission_stop_boundary;
  if (missionStopBoundary) {
    summary.mission_stop_boundary = missionStopBoundary;
  }

  const missionSummary = snapshot.mission_summary ?? snapshot.mission?.goal;
  if (missionSummary) {
    summary.mission_summary = missionSummary;
  }

  if (snapshot.plan?.steps?.length) {
    summary.plan_step_ids = snapshot.plan.steps.map((step) => step.id);
  }

  if (snapshot.plan?.requires_clarification) {
    summary.requires_clarification = true;
  }

  if (snapshot.plan?.clarification_question) {
    summary.clarification_question = snapshot.plan.clarification_question;
  }

  if (snapshot.artifact_refs) {
    summary.artifact_refs = snapshot.artifact_refs;
  }

  return summary;
}

export function buildArtifactRefs(snapshot = {}) {
  const refs = {};

  if (snapshot.workspace_path) {
    refs.workspace_path = snapshot.workspace_path;
  }

  if (snapshot.transcript_path) {
    refs.transcript_path = snapshot.transcript_path;
  }

  if (snapshot.preserved_artifacts) {
    refs.preserved_artifacts = snapshot.preserved_artifacts;
  }

  return Object.keys(refs).length > 0 ? refs : null;
}

export function buildResumableCheckpoint(snapshot = {}) {
  return {
    target_path: snapshot.target_path ?? null,
    target_type: snapshot.target_type ?? null,
    mission_type:
      snapshot.mission?.type ?? snapshot.summary?.mission_type ?? null,
    mission_summary:
      snapshot.mission_summary ??
      snapshot.mission?.goal ??
      snapshot.summary?.mission_summary ??
      null,
    mission_stop_boundary:
      snapshot.mission?.stop_boundary ??
      snapshot.summary?.mission_stop_boundary ??
      null,
    iteration: snapshot.iteration ?? 0,
    last_verification_state: snapshot.last_verification_state ?? null,
    plan_step_ids:
      snapshot.plan?.steps?.map((step) => step.id) ??
      snapshot.summary?.plan_step_ids ??
      [],
    workspace_path: snapshot.workspace_path ?? null,
    written_back: snapshot.written_back ?? [],
    resumable: snapshot.resumable ?? false,
    artifact_refs: snapshot.artifact_refs ?? buildArtifactRefs(snapshot),
  };
}

export async function loadSessionSnapshot() {
  let raw;
  try {
    raw = await readFile(sessionStorePath, "utf8");
  } catch (err) {
    if (
      err.code === "ENOENT" ||
      err.code === "EACCES" ||
      err.code === "EPERM" ||
      err.code === "EROFS"
    ) {
      return null;
    }
    throw err;
  }

  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

// Write lock: serializes concurrent persistSessionSnapshot calls via promise chain.
// Without this, overlapping async writes can produce corrupted session.json.
let _writeLock = Promise.resolve();

export async function persistSessionSnapshot(snapshot) {
  const doWrite = async () => {
    const artifact_refs = snapshot.artifact_refs ?? buildArtifactRefs(snapshot);
    const next = {
      ...snapshot,
      artifact_refs,
      summary: buildSessionSummary({ ...snapshot, artifact_refs }),
      checkpoint: buildResumableCheckpoint({ ...snapshot, artifact_refs }),
    };
    try {
      await mkdir(decipherHome, { recursive: true });
      await writeFile(sessionStorePath, JSON.stringify(next, null, 2), "utf8");
      if (next.mission || next.mission_summary || next.target_path) {
        await persistMissionMemory(next);
      }
    } catch (err) {
      if (!["EACCES", "EPERM", "EROFS"].includes(err.code)) {
        throw err;
      }
    }
    return next;
  };

  // Chain onto the lock — each write waits for the previous to finish
  const result = _writeLock.then(doWrite, doWrite);
  _writeLock = result.catch(() => {});
  return result;
}

export async function clearSessionSnapshot() {
  try {
    await rm(sessionStorePath, { force: true });
  } catch (err) {
    if (!["EACCES", "EPERM", "EROFS"].includes(err.code)) {
      throw err;
    }
  }
}

// ── Transcript persistence ──────────────────────────────────────────────────

const transcriptDir = join(decipherHome, "transcripts");

export function getTranscriptDir() {
  return transcriptDir;
}

/**
 * Persist a transcript string as a timestamped file.
 * Returns the file path on success, null on failure.
 */
export async function persistTranscript(transcript, missionId = "adhoc") {
  if (!transcript) return null;
  const ts = new Date().toISOString().replace(/[:.]/g, "-");
  const filename = `${missionId}-${ts}.log`;
  const filePath = join(transcriptDir, filename);
  try {
    await mkdir(transcriptDir, { recursive: true });
    await writeFile(filePath, transcript, "utf8");
    return filePath;
  } catch {
    return null;
  }
}

/**
 * Load the most recent transcript file.
 */
export async function loadLatestTranscript() {
  try {
    const { readdir: rd } = await import("node:fs/promises");
    const files = (await rd(transcriptDir))
      .filter((f) => f.endsWith(".log"))
      .sort()
      .reverse();
    if (files.length === 0) return null;
    const content = await readFile(join(transcriptDir, files[0]), "utf8");
    return { path: join(transcriptDir, files[0]), content };
  } catch {
    return null;
  }
}

// ── Mission summary compaction ──────────────────────────────────────────────

/**
 * Compact a session snapshot for long sessions by stripping bulky fields
 * while preserving enough state for resume and review.
 * Returns a new object — does not mutate the input.
 */
export function compactSessionSnapshot(snapshot) {
  if (!snapshot) return snapshot;

  const compacted = { ...snapshot };

  // Trim transcript to last 50 lines for the session file
  if (compacted.transcript && typeof compacted.transcript === "string") {
    const lines = compacted.transcript.split("\n");
    if (lines.length > 50) {
      compacted.transcript =
        `[... ${lines.length - 50} earlier lines omitted ...]\n` +
        lines.slice(-50).join("\n");
      compacted.transcript_compacted = true;
    }
  }

  // Strip large classification detail but keep the label
  if (
    compacted.classification &&
    typeof compacted.classification === "object" &&
    compacted.classification.root_causes
  ) {
    compacted.classification = {
      classification:
        compacted.classification.classification ??
        compacted.classification.label,
      confidence: compacted.classification.confidence,
    };
  }

  return compacted;
}

// ── Workspace preservation ──────────────────────────────────────────────────

/**
 * Record a preserved workspace so it can be discovered by /artifacts.
 * Appends to a simple JSON array file.
 */
export async function recordPreservedWorkspace(workspacePath, metadata = {}) {
  if (!workspacePath) return;
  const indexPath = join(decipherHome, "preserved-workspaces.json");
  let records = [];
  try {
    const raw = await readFile(indexPath, "utf8");
    records = JSON.parse(raw);
    if (!Array.isArray(records)) records = [];
  } catch {
    // fresh file
  }

  records.push({
    path: workspacePath,
    timestamp: new Date().toISOString(),
    mission_id: metadata.mission_id ?? null,
    reason: metadata.reason ?? "failure",
  });

  // Keep only last 20 entries
  if (records.length > 20) {
    records = records.slice(-20);
  }

  try {
    await mkdir(decipherHome, { recursive: true });
    await writeFile(indexPath, JSON.stringify(records, null, 2), "utf8");
  } catch {
    // ignore write errors
  }
}

/**
 * List all preserved workspaces.
 */
export async function listPreservedWorkspaces() {
  const indexPath = join(decipherHome, "preserved-workspaces.json");
  try {
    const raw = await readFile(indexPath, "utf8");
    const records = JSON.parse(raw);
    return Array.isArray(records) ? records : [];
  } catch {
    return [];
  }
}
