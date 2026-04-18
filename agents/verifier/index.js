import { exec } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, copyFile } from "node:fs/promises";
import pc from "picocolors";

const execAsync = promisify(exec);

const ENV_CHECKS = [
  { name: "Node.js", command: "node --version", minMajor: 18 },
  { name: "pnpm", command: "pnpm --version", minMajor: 8, fallback: "corepack pnpm --version" },
  { name: "Docker", command: "docker --version", minMajor: null },
];

/**
 * Run a shell command and capture exit code + stdout/stderr.
 */
export async function runCommand(command, options = {}) {
  try {
    const { stdout, stderr } = await execAsync(command, {
      timeout: options.timeout ?? 60_000,
      cwd: options.cwd,
    });
    return { exitCode: 0, stdout: stdout.trim(), stderr: stderr.trim() };
  } catch (err) {
    return {
      exitCode: err.code ?? 1,
      stdout: (err.stdout ?? "").trim(),
      stderr: (err.stderr ?? "").trim(),
    };
  }
}

/**
 * Check all required dev environment dependencies.
 * Supports corepack pnpm as a fallback for pnpm detection.
 */
export async function checkEnvironment() {
  const items = [];
  let allPassed = true;

  for (const check of ENV_CHECKS) {
    let result = await runCommand(check.command);

    // Fallback detection (e.g. corepack pnpm)
    if (result.exitCode !== 0 && check.fallback) {
      const fallbackResult = await runCommand(check.fallback);
      if (fallbackResult.exitCode === 0) {
        result = fallbackResult;
        console.log(pc.dim(`  (detected via fallback: ${check.fallback})`));
      }
    }

    if (result.exitCode !== 0) {
      console.log(`  ${pc.red("✗")} ${check.name.padEnd(10)} not found`);
      items.push({ name: check.name, passed: false, error: "not found" });
      allPassed = false;
      continue;
    }

    const versionStr = result.stdout.replace(/^v/, "").split("\n")[0].trim();
    const major = parseInt(versionStr.split(".")[0], 10);
    const passed = check.minMajor === null || major >= check.minMajor;

    const status = passed ? pc.green("✓") : pc.red("✗");
    const req = check.minMajor ? `(required: >= ${check.minMajor})` : "";
    console.log(
      `  ${status} ${check.name.padEnd(10)} ${versionStr.padEnd(15)} ${req}`,
    );

    items.push({ name: check.name, version: versionStr, passed });
    if (!passed) allPassed = false;
  }

  console.log("");
  if (!allPassed) {
    console.log(pc.yellow("Issues found. Run: node bin/decipher bootstrap"));
  }

  return { allPassed, items };
}

/**
 * Generate bootstrap install instructions for missing dependencies.
 */
export async function generateBootstrapPlan() {
  const { allPassed, items } = await checkEnvironment();
  if (allPassed) {
    console.log(pc.green("All dependencies present. No bootstrap needed."));
    return;
  }

  console.log("[BOOTSTRAP PLAN]\n");
  for (const item of items.filter((i) => !i.passed)) {
    console.log(`Missing: ${item.name}`);
    if (item.name === "Docker") {
      console.log("  macOS:   brew install --cask docker");
      console.log("  Linux:   curl -fsSL https://get.docker.com | sh");
      console.log(
        "  Windows: https://docs.docker.com/desktop/windows/install/",
      );
    } else if (item.name === "Node.js") {
      console.log(
        "  Install: curl -fsSL https://fnm.vercel.app/install | bash && fnm use 18",
      );
    } else if (item.name === "pnpm") {
      console.log("  Global:  npm install -g pnpm");
      console.log("  Corepack (recommended): corepack enable && corepack prepare pnpm@latest --activate");
    }
    console.log("");
  }
}

/**
 * Apply a unified diff patch to a target file.
 * Creates a .bak backup first.
 *
 * Handles both replacement hunks (- then +) and pure insertion hunks (+ only).
 * Pure insertions are inserted BEFORE the line at the target position.
 */
export async function applyPatch(patch, targetFile) {
  // Backup the original
  await copyFile(targetFile, `${targetFile}.bak`);

  const lines = patch.split("\n");
  const original = await readFile(targetFile, "utf8");
  const originalLines = original.split("\n");

  const removals = new Set();
  // Each addition: { at: lineIndex, content: string, isReplacement: boolean }
  // isReplacement=true  → goes in place of the removed line at `at`
  // isReplacement=false → inserted BEFORE the line at `at`
  const additions = [];

  let targetLine = 0;
  let inRemoval = false;
  let lastRemovalLine = -1;

  for (const line of lines) {
    if (line.startsWith("@@")) {
      const match = line.match(/@@ -(\d+)/);
      if (match) targetLine = parseInt(match[1], 10) - 1;
      inRemoval = false;
      lastRemovalLine = -1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      removals.add(targetLine);
      lastRemovalLine = targetLine;
      inRemoval = true;
      targetLine++;
    } else if (line.startsWith("+") && !line.startsWith("+++")) {
      if (inRemoval) {
        // Replacement: emitted in place of the last removed line
        additions.push({ at: lastRemovalLine, content: line.slice(1), isReplacement: true });
      } else {
        // Pure insertion: emitted BEFORE the current targetLine
        additions.push({ at: targetLine, content: line.slice(1), isReplacement: false });
      }
    } else if (!line.startsWith("---") && !line.startsWith("+++")) {
      // Context line — resets removal tracking
      inRemoval = false;
      lastRemovalLine = -1;
      targetLine++;
    }
  }

  const result = [];
  for (let i = 0; i < originalLines.length; i++) {
    // Emit pure insertions scheduled before this line
    for (const add of additions) {
      if (!add.isReplacement && add.at === i) {
        result.push(add.content);
      }
    }

    if (removals.has(i)) {
      // Emit replacement lines in place of the removed line
      for (const add of additions) {
        if (add.isReplacement && add.at === i) {
          result.push(add.content);
        }
      }
      // Skip the original line (it was removed)
    } else {
      result.push(originalLines[i]);
    }
  }

  // Emit any insertions at or beyond end of file
  for (const add of additions) {
    if (!add.isReplacement && add.at >= originalLines.length) {
      result.push(add.content);
    }
  }

  await writeFile(targetFile, result.join("\n"), "utf8");
}

/**
 * Run verification command and return structured result.
 *
 * Determines pass/fail by checking stdout for an explicit "PASS" or "FAIL"
 * marker on the last line — this handles the common `&& echo PASS || echo FAIL`
 * pattern where `echo FAIL` exits 0 and would otherwise mask the failure.
 * Falls back to exit code when no such marker is present.
 */
export async function runVerification(verificationCommand, options = {}) {
  console.log(`  Running: ${verificationCommand}`);
  const result = await runCommand(verificationCommand, options);

  // Check last stdout line for explicit PASS/FAIL marker
  const lastLine = result.stdout.trim().split("\n").pop()?.trim().toUpperCase() ?? "";
  let passed;
  if (lastLine === "PASS") {
    passed = true;
  } else if (lastLine === "FAIL") {
    passed = false;
  } else {
    passed = result.exitCode === 0;
  }

  return {
    command: verificationCommand,
    exit_code: result.exitCode,
    stdout_excerpt: result.stdout.split("\n").slice(0, 5).join("\n"),
    result: passed ? "PASS" : "FAIL",
  };
}
