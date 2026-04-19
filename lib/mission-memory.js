import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { getConfigDir } from "./config.js";

export function getMissionMemoryDir() {
  return join(getConfigDir(), "missions");
}

export function buildMissionMemoryRecord(snapshot = {}) {
  const now = new Date().toISOString();
  return {
    id: snapshot.mission_memory_id ?? `mission-${Date.now()}`,
    created_at: snapshot.created_at ?? now,
    updated_at: now,
    target_path: snapshot.target_path ?? null,
    target_type: snapshot.target_type ?? null,
    mission_type: snapshot.mission?.type ?? snapshot.summary?.mission_type ?? null,
    mission_goal: snapshot.mission_summary ?? snapshot.mission?.goal ?? snapshot.summary?.mission_summary ?? null,
    mission_stop_boundary:
      snapshot.mission?.stop_boundary ??
      snapshot.summary?.mission_stop_boundary ??
      null,
    last_verification_state: snapshot.last_verification_state ?? null,
    stop_reason: snapshot.stop_reason ?? null,
    files_touched: snapshot.summary?.files_touched ?? snapshot.repair_target_files ?? [],
    files_written_back: snapshot.summary?.files_written_back ?? snapshot.written_back ?? [],
    artifact_refs: snapshot.artifact_refs ?? null,
  };
}

export async function persistMissionMemory(snapshot = {}) {
  const dir = getMissionMemoryDir();
  const record = buildMissionMemoryRecord(snapshot);
  const path = join(dir, `${record.id}.json`);
  await mkdir(dir, { recursive: true });
  await writeFile(path, JSON.stringify(record, null, 2), "utf8");
  return { ...record, path };
}

export async function loadMissionMemory(id) {
  const path = join(getMissionMemoryDir(), `${id}.json`);
  const raw = await readFile(path, "utf8");
  return JSON.parse(raw);
}

export async function listMissionMemories() {
  const dir = getMissionMemoryDir();
  let names = [];
  try {
    names = await readdir(dir);
  } catch (err) {
    if (err.code === "ENOENT") {
      return [];
    }
    throw err;
  }

  const records = [];
  for (const name of names.filter((item) => item.endsWith(".json")).sort()) {
    try {
      const raw = await readFile(join(dir, name), "utf8");
      records.push(JSON.parse(raw));
    } catch {
      // ignore malformed mission records
    }
  }
  return records.sort((a, b) => String(b.updated_at).localeCompare(String(a.updated_at)));
}
