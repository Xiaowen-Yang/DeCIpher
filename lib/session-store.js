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

export async function loadSessionSnapshot() {
  let raw;
  try {
    raw = await readFile(sessionStorePath, "utf8");
  } catch (err) {
    if (err.code === "ENOENT" || err.code === "EACCES" || err.code === "EPERM") {
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
  try {
    await mkdir(decipherHome, { recursive: true });
    await writeFile(sessionStorePath, JSON.stringify(snapshot, null, 2), "utf8");
  } catch (err) {
    if (!["EACCES", "EPERM"].includes(err.code)) {
      throw err;
    }
  }
  return snapshot;
}

export async function clearSessionSnapshot() {
  try {
    await rm(sessionStorePath, { force: true });
  } catch (err) {
    if (!["EACCES", "EPERM"].includes(err.code)) {
      throw err;
    }
  }
}
