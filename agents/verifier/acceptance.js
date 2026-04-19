/**
 * Acceptance runner for greenfield scenarios.
 *
 * Greenfield missions start from nothing — no broken/expected dirs.
 * Instead of comparing file diffs, we run a sequence of acceptance checks
 * against the workspace the agent produced:
 *
 *   - file_exists  — verify a file was created
 *   - file_contains — verify a file contains a substring
 *   - command       — run a shell command and check exit / stdout
 *
 * acceptance.json schema:
 *   { checks: [ { id, type, ...typeSpecificFields, description } ] }
 */

import { access, readFile } from "node:fs/promises";
import { join } from "node:path";
import { exec as execCb } from "node:child_process";
import { promisify } from "node:util";
import pc from "picocolors";

const execAsync = promisify(execCb);
const DEFAULT_TIMEOUT = 120_000;

/**
 * @typedef {object} AcceptanceCheck
 * @property {string} id
 * @property {'file_exists'|'file_contains'|'command'} type
 * @property {string} [path]                — for file_exists / file_contains
 * @property {string} [contains]            — for file_contains
 * @property {string} [command]             — for command checks
 * @property {number} [expect_exit]         — expected exit code (default 0)
 * @property {string} [expect_stdout_contains] — substring expected in stdout
 * @property {number} [timeout]             — ms, default 120s
 * @property {string} description           — human-readable purpose
 */

/**
 * @typedef {object} CheckResult
 * @property {string}  id
 * @property {boolean} passed
 * @property {string}  description
 * @property {string}  [detail]    — error message on failure
 */

/**
 * Load acceptance checks from a scenario's acceptance.json file.
 */
export async function loadAcceptanceChecks(scenarioPath) {
  const filePath = join(scenarioPath, "acceptance.json");
  const raw = await readFile(filePath, "utf8");
  const parsed = JSON.parse(raw);
  return parsed.checks ?? [];
}

/**
 * Run a single acceptance check against a workspace.
 *
 * @param {AcceptanceCheck} check
 * @param {string} workspace  — the agent's working directory
 * @returns {Promise<CheckResult>}
 */
async function runSingleCheck(check, workspace) {
  const base = { id: check.id, description: check.description };

  switch (check.type) {
    case "file_exists": {
      const target = join(workspace, check.path);
      try {
        await access(target);
        return { ...base, passed: true };
      } catch {
        return {
          ...base,
          passed: false,
          detail: `File not found: ${check.path}`,
        };
      }
    }

    case "file_contains": {
      const target = join(workspace, check.path);
      try {
        const content = await readFile(target, "utf8");
        const found = content.includes(check.contains);
        return found
          ? { ...base, passed: true }
          : {
              ...base,
              passed: false,
              detail: `File ${check.path} does not contain "${check.contains}"`,
            };
      } catch (err) {
        return {
          ...base,
          passed: false,
          detail: `Cannot read ${check.path}: ${err.message}`,
        };
      }
    }

    case "command": {
      const timeout = check.timeout ?? DEFAULT_TIMEOUT;
      try {
        const { stdout, stderr } = await execAsync(check.command, {
          cwd: workspace,
          timeout,
        });
        const combined = stdout + stderr;

        if (
          check.expect_exit !== undefined &&
          check.expect_exit !== null &&
          check.expect_exit !== 0
        ) {
          // Non-zero expected — this check expects failure, but exec succeeded
          return {
            ...base,
            passed: false,
            detail: `Expected exit ${check.expect_exit} but got 0`,
          };
        }

        if (
          check.expect_stdout_contains &&
          !combined.includes(check.expect_stdout_contains)
        ) {
          return {
            ...base,
            passed: false,
            detail: `Output does not contain "${check.expect_stdout_contains}". Got: ${stdout.slice(0, 200)}`,
          };
        }

        return { ...base, passed: true };
      } catch (err) {
        const exitCode = err.code ?? 1;
        if (
          check.expect_exit !== undefined &&
          check.expect_exit !== null &&
          exitCode === check.expect_exit
        ) {
          return { ...base, passed: true };
        }
        return {
          ...base,
          passed: false,
          detail: `Command failed (exit ${exitCode}): ${(err.stderr ?? err.message ?? "").slice(0, 300)}`,
        };
      }
    }

    default:
      return {
        ...base,
        passed: false,
        detail: `Unknown check type: ${check.type}`,
      };
  }
}

/**
 * Run all acceptance checks and return a structured report.
 *
 * @param {AcceptanceCheck[]} checks
 * @param {string} workspace
 * @returns {Promise<{ passed: boolean, total: number, results: CheckResult[] }>}
 */
export async function runAcceptanceChecks(checks, workspace) {
  const results = [];

  for (const check of checks) {
    console.log(
      pc.dim(`  [${results.length + 1}/${checks.length}] ${check.description}`),
    );
    const result = await runSingleCheck(check, workspace);
    results.push(result);

    const icon = result.passed ? pc.green("✓") : pc.red("✗");
    console.log(`  ${icon} ${check.description}`);
    if (!result.passed && result.detail) {
      console.log(pc.dim(`    ${result.detail}`));
    }
  }

  const passCount = results.filter((r) => r.passed).length;
  const allPassed = passCount === results.length;

  return {
    passed: allPassed,
    total: results.length,
    pass_count: passCount,
    results,
  };
}

/**
 * Print a final acceptance summary.
 */
export function printAcceptanceSummary(report) {
  const divider = pc.dim("─".repeat(60));
  console.log(`\n${divider}`);
  console.log(
    pc.bold(
      `\n  Acceptance: ${report.pass_count}/${report.total} checks passed`,
    ),
  );

  if (report.passed) {
    console.log(pc.bold(pc.green("  MISSION COMPLETE")));
  } else {
    const failed = report.results.filter((r) => !r.passed);
    console.log(pc.bold(pc.red(`  ${failed.length} check(s) failed:`)));
    for (const f of failed) {
      console.log(`    ${pc.red("✗")} ${f.description}`);
      if (f.detail) console.log(pc.dim(`      ${f.detail}`));
    }
  }

  console.log(`\n${divider}\n`);
}
