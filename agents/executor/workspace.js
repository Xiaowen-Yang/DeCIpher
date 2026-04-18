/**
 * Workspace manager — creates isolated temp copies of broken/ directories and
 * writes back only the repaired files on success.
 *
 * All loop iterations operate exclusively inside the temp workspace so the
 * original scenario directory is never mutated during a run.
 */

import { mkdtemp, cp, copyFile, readFile, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";
import { tmpdir } from "node:os";

/**
 * Create a temp workspace copy of the broken/ directory.
 * @param {string} brokenDir  Absolute path to scenarios/<name>/broken/
 * @param {string} scenarioId Used for the temp dir name prefix
 * @returns {Promise<string>} Absolute path to the temp workspace
 */
export async function createWorkspace(brokenDir, scenarioId) {
  const workspace = await mkdtemp(join(tmpdir(), `decipher-${scenarioId}-`));
  await cp(brokenDir, workspace, { recursive: true });
  return workspace;
}

/**
 * Write back only the specified files from the workspace to the original
 * broken/ directory. File-scoped — does not overwrite the whole directory.
 *
 * @param {string} workspace       Temp workspace path
 * @param {string} brokenDir       Original broken/ directory
 * @param {string[]} relPaths      Relative file paths to write back (e.g. ["Dockerfile"])
 * @returns {Promise<string[]>}    List of files successfully written back
 */
export async function writeBack(workspace, brokenDir, relPaths) {
  const written = [];
  for (const rel of relPaths) {
    const src = join(workspace, rel);
    const dst = join(brokenDir, rel);
    try {
      await copyFile(src, dst);
      written.push(rel);
    } catch (err) {
      // Log but do not abort — partial write-back is still useful
      process.stderr.write(`  [workspace] write-back skipped for ${rel}: ${err.message}\n`);
    }
  }
  return written;
}

/**
 * Read file contents from the workspace for use in the fixer prompt.
 * @param {string} workspace
 * @param {string[]} relPaths
 * @returns {Promise<Array<{path: string, content: string}>>}
 */
export async function readWorkspaceFiles(workspace, relPaths) {
  const files = [];
  for (const rel of relPaths) {
    try {
      const content = await readFile(join(workspace, rel), "utf8");
      files.push({ path: rel, content });
    } catch { /* file may not exist */ }
  }
  return files;
}

/**
 * Safely remove the workspace directory.
 * On failure (e.g. permission denied), logs the path so the user can inspect.
 * @param {string} workspace
 * @param {boolean} preserve If true, skip removal and report path instead
 */
export async function cleanupWorkspace(workspace, preserve = false) {
  if (preserve) {
    return workspace; // caller shows this to the user
  }
  const { rm } = await import("node:fs/promises");
  try {
    await rm(workspace, { recursive: true, force: true });
  } catch (err) {
    process.stderr.write(`  [workspace] cleanup failed: ${workspace} — ${err.message}\n`);
  }
  return null;
}
