/**
 * Docker command runners — build, run, and healthcheck.
 *
 * Each runner returns a structured result:
 * {
 *   state:    'BUILD_FAIL' | 'RUN_FAIL' | 'HEALTHCHECK_FAIL' | 'PASS',
 *   output:   string,   // combined stdout + stderr
 *   tag:      string,   // docker image tag used (for cleanup)
 * }
 */

import { exec as execCb } from "node:child_process";
import { promisify } from "node:util";

const exec = promisify(execCb);

const BUILD_TIMEOUT = 120_000; // 2 min
const RUN_TIMEOUT = 30_000; // 30 s for container start
const HC_POLL_INTERVAL = 3_000; // poll every 3 s
const HC_MAX_WAIT = 45_000; // give up after 45 s (5s interval × 3 retries + startup)

function tag(scenarioId) {
  return `decipher-${scenarioId}-${Date.now()}`
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, "-");
}

async function safeExec(cmd, opts = {}) {
  try {
    const { stdout, stderr } = await exec(cmd, {
      timeout: BUILD_TIMEOUT,
      ...opts,
    });
    return { exitCode: 0, output: (stdout + stderr).trim() };
  } catch (err) {
    return {
      exitCode: err.code ?? 1,
      output: (
        (err.stdout ?? "") +
        (err.stderr ?? "") +
        "\n" +
        err.message
      ).trim(),
    };
  }
}

/**
 * docker build mode — just build the image.
 */
export async function runDockerBuild(workspace, scenarioId) {
  const imageTag = tag(scenarioId);
  const { exitCode, output } = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );

  if (exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return { state: "BUILD_FAIL", output, tag: null };
  }

  await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
  return { state: "PASS", output, tag: null };
}

/**
 * docker_run mode — build, run the container, check exit code and stdout.
 * Used for scenarios where the container crashes on startup.
 */
export async function runDockerRun(workspace, scenarioId) {
  const imageTag = tag(scenarioId);

  // Build
  const buildResult = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );
  if (buildResult.exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return { state: "BUILD_FAIL", output: buildResult.output, tag: null };
  }

  // Run (non-interactive, capture output, expect the container to exit on its own)
  const runResult = await safeExec(`docker run --rm ${imageTag} 2>&1`, {
    timeout: RUN_TIMEOUT,
  });

  await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);

  if (runResult.exitCode !== 0) {
    return {
      state: "RUN_FAIL",
      output: buildResult.output + "\n" + runResult.output,
      tag: null,
    };
  }
  return { state: "PASS", output: runResult.output, tag: null };
}

/**
 * healthcheck mode — build, start the container as a daemon, poll the Docker
 * health status, capture container logs on failure.
 *
 * The HEALTHCHECK instruction must be in the Dockerfile for this to work.
 */
export async function runHealthcheck(workspace, scenarioId) {
  const imageTag = tag(scenarioId);
  const containerName = `decipher-hc-${Date.now()}`;

  // Build
  const buildResult = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );
  if (buildResult.exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return { state: "BUILD_FAIL", output: buildResult.output, tag: null };
  }

  // Start daemon
  const startResult = await safeExec(
    `docker run -d --name ${containerName} ${imageTag} 2>&1`,
    { timeout: RUN_TIMEOUT },
  );
  if (startResult.exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "RUN_FAIL",
      output: buildResult.output + "\n" + startResult.output,
      tag: null,
    };
  }

  // Poll health status
  let healthState = "starting";
  let waited = 0;
  while (waited < HC_MAX_WAIT) {
    await new Promise((r) => setTimeout(r, HC_POLL_INTERVAL));
    waited += HC_POLL_INTERVAL;
    const inspectResult = await safeExec(
      `docker inspect --format='{{.State.Health.Status}}' ${containerName} 2>&1`,
    );
    healthState = inspectResult.output.trim().replace(/'/g, "");
    if (healthState === "healthy" || healthState === "unhealthy") break;
  }

  // Capture container logs
  const logsResult = await safeExec(`docker logs ${containerName} 2>&1`);

  // Cleanup
  await safeExec(`docker stop ${containerName} 2>/dev/null`);
  await safeExec(`docker rm ${containerName} 2>/dev/null`);
  await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);

  const allOutput = [
    `=== docker build output ===`,
    buildResult.output,
    `=== container logs ===`,
    logsResult.output,
    `=== health status: ${healthState} ===`,
  ].join("\n");

  if (healthState === "healthy") {
    return { state: "PASS", output: allOutput, tag: null };
  }

  return { state: "HEALTHCHECK_FAIL", output: allOutput, tag: null };
}

/**
 * Dispatch to the correct runner based on execution_mode.
 * @param {'docker_build'|'docker_run'|'healthcheck'} mode
 * @param {string} workspace
 * @param {string} scenarioId
 */
export async function runCommand(mode, workspace, scenarioId) {
  switch (mode) {
    case "docker_run":
      return runDockerRun(workspace, scenarioId);
    case "healthcheck":
      return runHealthcheck(workspace, scenarioId);
    case "docker_build":
    default:
      return runDockerBuild(workspace, scenarioId);
  }
}
