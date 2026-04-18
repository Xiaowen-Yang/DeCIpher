import { exec } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, copyFile } from "node:fs/promises";
import pc from "picocolors";

const execAsync = promisify(exec);

const ENV_CHECKS = [
  { name: "Node.js", command: "node --version", minMajor: 18 },
  { name: "pnpm", command: "pnpm --version", minMajor: 8 },
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
 */
export async function checkEnvironment() {
  const items = [];
  let allPassed = true;

  for (const check of ENV_CHECKS) {
    const result = await runCommand(check.command);
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
      console.log("  Install: npm install -g pnpm");
    }
    console.log("");
  }
}

/**
 * Apply a unified diff patch to a target file.
 * Creates a .bak backup first.
 */
export async function applyPatch(patch, targetFile) {
  // Backup the original
  await copyFile(targetFile, `${targetFile}.bak`);

  const lines = patch.split("\n");
  const original = await readFile(targetFile, "utf8");
  const originalLines = original.split("\n");

  const removals = new Set();
  const additions = [];
  let targetLine = 0;

  let lastRemovalLine = null;

  for (const line of lines) {
    if (line.startsWith("@@")) {
      // Parse hunk header: @@ -L,S +L,S @@
      const match = line.match(/@@ -(\d+)/);
      if (match) targetLine = parseInt(match[1], 10) - 1;
      lastRemovalLine = null;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      lastRemovalLine = targetLine;
      removals.add(targetLine);
      targetLine++;
    } else if (line.startsWith("+") && !line.startsWith("+++")) {
      // Associate addition with the removal position (or current position if no prior removal)
      const insertAt = lastRemovalLine !== null ? lastRemovalLine : targetLine;
      additions.push({ at: insertAt, content: line.slice(1) });
    } else if (!line.startsWith("---") && !line.startsWith("+++")) {
      lastRemovalLine = null;
      targetLine++;
    }
  }

  const result = [];
  for (let i = 0; i < originalLines.length; i++) {
    if (removals.has(i)) {
      // Insert additions at this position
      const adds = additions.filter((a) => a.at === i);
      for (const add of adds) result.push(add.content);
      // Skip the removed line
    } else {
      result.push(originalLines[i]);
    }
  }

  await writeFile(targetFile, result.join("\n"), "utf8");
}

/**
 * Run verification command from scenario metadata and return structured result.
 */
export async function runVerification(verificationCommand, options = {}) {
  console.log(`  Running: ${verificationCommand}`);
  const result = await runCommand(verificationCommand, options);
  const passed = result.exitCode === 0;

  return {
    command: verificationCommand,
    exit_code: result.exitCode,
    stdout_excerpt: result.stdout.split("\n").slice(0, 5).join("\n"),
    result: passed ? "PASS" : "FAIL",
  };
}
