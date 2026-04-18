import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname } from "node:path";

function normalizeHistoryEntry(entry) {
  if (typeof entry === "string") {
    return { text: entry };
  }

  if (!entry || typeof entry !== "object" || typeof entry.text !== "string") {
    return null;
  }

  return { text: entry.text };
}

export async function loadHistoryEntries(historyPath) {
  let raw;
  try {
    raw = await readFile(historyPath, "utf8");
  } catch (err) {
    if (err.code === "ENOENT" || err.code === "EACCES" || err.code === "EPERM") {
      return [];
    }
    throw err;
  }

  const entries = [];
  for (const line of raw.split("\n")) {
    if (!line.trim()) {
      continue;
    }

    try {
      const parsed = JSON.parse(line);
      const entry = normalizeHistoryEntry(parsed);
      if (entry) {
        entries.push(entry);
      }
    } catch {
      // Ignore malformed JSONL rows so one bad line does not break history.
    }
  }

  return entries;
}

export async function appendHistoryEntry(historyPath, entry) {
  const normalized = normalizeHistoryEntry(entry);
  if (!normalized) {
    throw new TypeError("History entries must be a string or an object with a text field.");
  }

  try {
    await mkdir(dirname(historyPath), { recursive: true });
    await appendFile(historyPath, `${JSON.stringify(normalized)}\n`, "utf8");
  } catch (err) {
    if (!["EACCES", "EPERM"].includes(err.code)) {
      throw err;
    }
  }
  return normalized;
}

export function toReadlineHistory(entries) {
  return entries
    .map((entry) => normalizeHistoryEntry(entry))
    .filter(Boolean)
    .map((entry) => entry.text)
    .reverse();
}

export function findReverseHistoryMatches(entries, query) {
  const matches = [];
  const seen = new Set();

  for (const entry of [...entries].reverse()) {
    const normalized = normalizeHistoryEntry(entry);
    if (!normalized) {
      continue;
    }

    if (!normalized.text.includes(query) || seen.has(normalized.text)) {
      continue;
    }

    seen.add(normalized.text);
    matches.push(normalized.text);
  }

  return matches;
}
