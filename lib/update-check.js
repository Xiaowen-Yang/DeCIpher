/**
 * Non-blocking npm update checker.
 *
 * On startup, checks if a newer version of decipher-cli is available on npm.
 * Results are cached for 24 hours in ~/.decipher/update-check.json.
 * Never blocks startup — runs in the background and resolves silently on error.
 *
 * Disable with DECIPHER_NO_UPDATE_CHECK=1.
 */

import { readFile, writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";

const CACHE_FILE = join(homedir(), ".decipher", "update-check.json");
const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours
const REGISTRY_URL = "https://registry.npmjs.org/decipher-cli/latest";
const FETCH_TIMEOUT_MS = 5000;

/**
 * Compare two semver strings. Returns:
 *  -1 if a < b, 0 if equal, 1 if a > b
 */
function compareSemver(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] ?? 0) < (pb[i] ?? 0)) return -1;
    if ((pa[i] ?? 0) > (pb[i] ?? 0)) return 1;
  }
  return 0;
}

/**
 * Read the cached update check result.
 * Returns { latestVersion, checkedAt } or null.
 */
async function readCache() {
  try {
    const raw = await readFile(CACHE_FILE, "utf8");
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

/**
 * Write the update check result to cache.
 */
async function writeCache(latestVersion) {
  try {
    await mkdir(join(homedir(), ".decipher"), { recursive: true });
    await writeFile(
      CACHE_FILE,
      JSON.stringify({ latestVersion, checkedAt: Date.now() }),
      "utf8",
    );
  } catch {
    // Non-critical — ignore cache write errors
  }
}

/**
 * Fetch the latest version from npm registry.
 * Returns the version string or null on failure.
 */
async function fetchLatestVersion() {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
    const res = await fetch(REGISTRY_URL, { signal: controller.signal });
    clearTimeout(timer);
    if (!res.ok) return null;
    const data = await res.json();
    return data.version ?? null;
  } catch {
    return null;
  }
}

/**
 * Check for updates and return a notification message if one is available.
 * Returns a string like "Update available: 0.1.0 → 0.2.0  (npm i -g decipher-cli)"
 * or null if up-to-date or check is skipped.
 *
 * This function is designed to be called with `await` but never blocks for
 * more than FETCH_TIMEOUT_MS (5s). On error, returns null silently.
 *
 * @param {string} currentVersion — the local installed version
 * @returns {Promise<string|null>}
 */
export async function checkForUpdate(currentVersion) {
  // Respect opt-out
  if (process.env.DECIPHER_NO_UPDATE_CHECK === "1") return null;

  // Check cache first
  const cache = await readCache();
  if (cache?.checkedAt && Date.now() - cache.checkedAt < CHECK_INTERVAL_MS) {
    // Use cached result
    if (cache.latestVersion && compareSemver(currentVersion, cache.latestVersion) < 0) {
      return `Update available: ${currentVersion} → ${cache.latestVersion}  (npm i -g decipher-cli)`;
    }
    return null;
  }

  // Fetch from registry
  const latest = await fetchLatestVersion();
  if (!latest) return null;

  await writeCache(latest);

  if (compareSemver(currentVersion, latest) < 0) {
    return `Update available: ${currentVersion} → ${latest}  (npm i -g decipher-cli)`;
  }

  return null;
}

export { compareSemver };
