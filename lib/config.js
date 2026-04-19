import { readFile, writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";

const configDir =
  process.env.DECIPHER_CONFIG_DIR ?? join(homedir(), ".decipher");
const configPath = join(configDir, "config.json");
const historyPath = join(configDir, "history.jsonl");
const sessionPath = join(configDir, "session.json");

const VALID_PROVIDERS = new Set(["openai", "anthropic", "custom"]);
const VALID_APPROVAL_POLICIES = new Set(["on-request", "on-failure", "never"]);
const NULLABLE_CONFIG_KEYS = new Set(["api_key", "base_url", "notification_command"]);

export const MODEL_CONFIG_KEYS = ["provider", "model", "base_url", "api_key"];
export const SETTING_CONFIG_KEYS = [
  ...MODEL_CONFIG_KEYS,
  "approval_policy",
  "max_iterations",
  "notification_command",
];

export const CONFIG_DEFAULTS = {
  provider: "openai",
  model: "gpt-4o",
  api_key: null,
  base_url: null,
  max_iterations: 3,
  approval_policy: "on-request",
  notification_command: null,
};

export function getConfigDir() {
  return configDir;
}

export function getConfigPath() {
  return configPath;
}

export function getHistoryPath() {
  return historyPath;
}

export function getSessionPath() {
  return sessionPath;
}

export function normalizeConfigKey(key) {
  return key.replace(/-/g, "_");
}

export function canUpdateConfigKey(key, scope = "setting") {
  const normalizedKey = normalizeConfigKey(key);
  const allowedKeys = scope === "model" ? MODEL_CONFIG_KEYS : SETTING_CONFIG_KEYS;
  return allowedKeys.includes(normalizedKey);
}

export function maskSecret(value) {
  if (value == null) return null;
  const str = String(value);
  if (str.length < 6) return "***";
  return `${str.slice(0, 2)}-***${str.slice(-2)}`.replace("--***", "-***");
}

export function maskConfig(config) {
  return {
    ...config,
    api_key: maskSecret(config.api_key),
  };
}

export function coerceConfigValue(key, value) {
  const normalizedKey = normalizeConfigKey(key);
  const text = String(value ?? "").trim();

  if (NULLABLE_CONFIG_KEYS.has(normalizedKey) && /^(default|none|null)$/i.test(text)) {
    return null;
  }

  if (normalizedKey === "max_iterations") {
    const parsed = Number.parseInt(text, 10);
    if (!Number.isInteger(parsed) || parsed <= 0) {
      throw new Error("Invalid max_iterations. Expected a positive integer.");
    }
    return parsed;
  }

  return text;
}

export function validateConfigUpdates(updates) {
  if ("provider" in updates && updates.provider != null) {
    if (!VALID_PROVIDERS.has(updates.provider)) {
      throw new Error(
        `Invalid provider '${updates.provider}'. Expected one of: ${[...VALID_PROVIDERS].join(", ")}`,
      );
    }
  }

  if ("base_url" in updates && updates.base_url) {
    let parsed;
    try {
      parsed = new URL(updates.base_url);
    } catch {
      throw new Error("Invalid base_url. Expected a full http(s) URL.");
    }
    if (!["http:", "https:"].includes(parsed.protocol)) {
      throw new Error("Invalid base_url. Expected an http(s) URL.");
    }
  }

  if ("approval_policy" in updates && updates.approval_policy != null) {
    if (!VALID_APPROVAL_POLICIES.has(updates.approval_policy)) {
      throw new Error(
        `Invalid approval_policy '${updates.approval_policy}'. Expected one of: ${[...VALID_APPROVAL_POLICIES].join(", ")}`,
      );
    }
  }

  if ("max_iterations" in updates && updates.max_iterations != null) {
    if (!Number.isInteger(updates.max_iterations) || updates.max_iterations <= 0) {
      throw new Error("Invalid max_iterations. Expected a positive integer.");
    }
  }
}

export async function readConfig() {
  let raw;
  try {
    raw = await readFile(configPath, "utf8");
  } catch (err) {
    if (err.code === "ENOENT") {
      // Config file does not exist yet — silent fallback to defaults
      return { ...CONFIG_DEFAULTS };
    }
    // Permission error or other OS-level issue
    console.error(`Warning: Could not read config (${err.code ?? err.message}). Using defaults.`);
    return { ...CONFIG_DEFAULTS };
  }

  try {
    return { ...CONFIG_DEFAULTS, ...JSON.parse(raw) };
  } catch {
    console.error(
      `Warning: Config file contains invalid JSON: ${configPath}`,
    );
    console.error(
      `  Rename or delete it to reset: mv ${configPath} ${configPath}.bak`,
    );
    return { ...CONFIG_DEFAULTS };
  }
}

export async function writeConfig(updates) {
  const normalizedUpdates = Object.fromEntries(
    Object.entries(updates).map(([key, value]) => [normalizeConfigKey(key), value]),
  );
  validateConfigUpdates(normalizedUpdates);

  const current = await readConfig();
  const next = { ...current, ...normalizedUpdates };
  await mkdir(configDir, { recursive: true });
  await writeFile(configPath, JSON.stringify(next, null, 2), "utf8");
  return next;
}
