import { readFile, writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";

const configDir =
  process.env.DECIPHER_CONFIG_DIR ?? join(homedir(), ".decipher");
const configPath = join(configDir, "config.json");

export const CONFIG_DEFAULTS = {
  provider: "openai",
  model: "gpt-4o",
  api_key: null,
  base_url: null,
  max_iterations: 3,
  auto_approve: false,
};

export async function readConfig() {
  try {
    const raw = await readFile(configPath, "utf8");
    return { ...CONFIG_DEFAULTS, ...JSON.parse(raw) };
  } catch {
    return { ...CONFIG_DEFAULTS };
  }
}

export async function writeConfig(updates) {
  const current = await readConfig();
  const next = { ...current, ...updates };
  await mkdir(configDir, { recursive: true });
  await writeFile(configPath, JSON.stringify(next, null, 2), "utf8");
  return next;
}
