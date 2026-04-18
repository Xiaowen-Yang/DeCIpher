import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";

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
  const value = usePublicSnapshot
    ? maskSessionSnapshot(snapshot)
    : snapshot;
  return JSON.stringify(value, null, 2);
}

export function buildSessionSummary(snapshot = {}) {
  return {
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
}

export async function loadSessionSnapshot() {
  let raw;
  try {
    raw = await readFile(sessionStorePath, "utf8");
  } catch (err) {
    if (err.code === "ENOENT" || err.code === "EACCES" || err.code === "EPERM" || err.code === "EROFS") {
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

export async function persistSessionSnapshot(snapshot) {
  const next = {
    ...snapshot,
    summary: buildSessionSummary(snapshot),
  };
  try {
    await mkdir(decipherHome, { recursive: true });
    await writeFile(sessionStorePath, JSON.stringify(next, null, 2), "utf8");
  } catch (err) {
    if (!["EACCES", "EPERM", "EROFS"].includes(err.code)) {
      throw err;
    }
  }
  return next;
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
