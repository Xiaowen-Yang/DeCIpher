import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { getConfigDir } from "./config.js";

export function getPreferenceMemoryPath() {
  return join(getConfigDir(), "preferences.json");
}

export async function readUserPreferences() {
  try {
    const raw = await readFile(getPreferenceMemoryPath(), "utf8");
    return JSON.parse(raw);
  } catch (err) {
    if (err.code === "ENOENT") {
      return {};
    }
    throw err;
  }
}

export async function writeUserPreferences(updates = {}) {
  const current = await readUserPreferences();
  const next = { ...current, ...updates };
  await mkdir(getConfigDir(), { recursive: true });
  await writeFile(getPreferenceMemoryPath(), JSON.stringify(next, null, 2), "utf8");
  return next;
}

export function mergeMissionPreferences(stored = {}, missionSpecific = {}) {
  return {
    ...stored,
    ...missionSpecific,
  };
}
