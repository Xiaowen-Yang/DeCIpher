/**
 * Workspace manager.
 *
 * Greenfield missions get an empty temp workspace.
 * All other missions work directly on user files — no temp copies.
 * Rollback is via git, not temp workspace preservation.
 */

import { mkdtemp, readFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

/**
 * Create an empty temp workspace for greenfield missions.
 * The agent starts from nothing and generates all files.
 * @param {string} scenarioId Used for the temp dir name prefix
 * @returns {Promise<string>} Absolute path to the temp workspace
 */
export async function createEmptyWorkspace(scenarioId) {
  return mkdtemp(join(tmpdir(), `decipher-${scenarioId}-`));
}

/**
 * Read file contents from a directory for use in prompts.
 * @param {string} dir
 * @param {string[]} relPaths
 * @returns {Promise<Array<{path: string, content: string}>>}
 */
export async function readWorkspaceFiles(dir, relPaths) {
  const files = [];
  for (const rel of relPaths) {
    try {
      const content = await readFile(join(dir, rel), "utf8");
      files.push({ path: rel, content });
    } catch {
      /* file may not exist */
    }
  }
  return files;
}
