/**
 * Docker command runners — build, run, and healthcheck.
 *
 * Each runner returns a structured result:
 * {
 *   state:    'BUILD_FAIL' | 'RUN_FAIL' | 'HEALTHCHECK_FAIL' | 'PASS',
 *   output:   string,   // combined stdout + stderr
 *   tag:      string,   // docker image tag used (for cleanup)
 *   containerStarted: boolean,
 *   cleanupPerformed: boolean,
 *   preservedArtifacts?: object|null,
 * }
 */

import { exec as execCb } from "node:child_process";
import { promisify } from "node:util";

const exec = promisify(execCb);

const BUILD_TIMEOUT = 120_000; // 2 min
const RUN_TIMEOUT = 30_000; // 30 s for container start
const STARTUP_GRACE_MS = 3_000;
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

function buildPreservedArtifacts({
  imageTag = null,
  containerName = null,
  status = null,
  mode = null,
  running = null,
}) {
  return {
    mode,
    image_tag: imageTag,
    container_name: containerName,
    container_status: status,
    container_running: running,
  };
}

/**
 * docker build mode — just build the image.
 */
export async function runDockerBuild(workspace, scenarioId, options = {}) {
  const imageTag = tag(scenarioId);
  const keepArtifacts = options.keepArtifacts === true;
  const { exitCode, output } = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );

  if (exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "BUILD_FAIL",
      output,
      tag: null,
      containerStarted: false,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
  }

  if (keepArtifacts) {
    return {
      state: "PASS",
      output,
      tag: imageTag,
      containerStarted: false,
      cleanupPerformed: false,
      preservedArtifacts: buildPreservedArtifacts({
        imageTag,
        mode: "docker_build",
      }),
    };
  }

  await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
  return {
    state: "PASS",
    output,
    tag: null,
    containerStarted: false,
    cleanupPerformed: true,
    preservedArtifacts: null,
  };
}

/**
 * docker_run mode — build, run the container, check exit code and stdout.
 * Used for scenarios where the container crashes on startup.
 */
export async function runDockerRun(workspace, scenarioId, options = {}) {
  const imageTag = tag(scenarioId);
  const containerName = `decipher-run-${Date.now()}`;
  const keepArtifacts = options.keepArtifacts === true;

  // Build
  const buildResult = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );
  if (buildResult.exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "BUILD_FAIL",
      output: buildResult.output,
      tag: null,
      containerStarted: false,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
  }

  const startResult = await safeExec(
    `docker run -d --name ${containerName} ${imageTag} 2>&1`,
    { timeout: RUN_TIMEOUT },
  );

  if (startResult.exitCode !== 0) {
    await safeExec(`docker rm -f ${containerName} 2>/dev/null`);
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "RUN_FAIL",
      output: buildResult.output + "\n" + startResult.output,
      tag: null,
      containerStarted: false,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
  }

  await new Promise((resolve) => setTimeout(resolve, STARTUP_GRACE_MS));

  const inspectResult = await safeExec(
    `docker inspect --format='{{.State.Running}}|{{.State.ExitCode}}|{{.State.Status}}' ${containerName} 2>&1`,
  );
  const logsResult = await safeExec(`docker logs ${containerName} 2>&1`);

  const [runningRaw = "", exitCodeRaw = "", statusRaw = ""] =
    inspectResult.output.trim().replace(/'/g, "").split("|");
  const running = runningRaw === "true";
  const exitCode = Number.parseInt(exitCodeRaw, 10);
  const status = statusRaw || "unknown";

  const combinedOutput = [
    "=== docker build output ===",
    buildResult.output,
    "=== docker run output ===",
    startResult.output,
    `=== container status: ${status} ===`,
    logsResult.output,
  ].join("\n");

  if (!running && exitCode !== 0) {
    await safeExec(`docker stop ${containerName} 2>/dev/null`);
    await safeExec(`docker rm ${containerName} 2>/dev/null`);
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "RUN_FAIL",
      output: combinedOutput,
      tag: null,
      containerStarted: true,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
  }

  if (keepArtifacts) {
    return {
      state: "PASS",
      output: combinedOutput,
      tag: imageTag,
      containerStarted: true,
      cleanupPerformed: false,
      preservedArtifacts: buildPreservedArtifacts({
        imageTag,
        containerName,
        status,
        mode: "docker_run",
        running,
      }),
    };
  }

  await safeExec(`docker stop ${containerName} 2>/dev/null`);
  await safeExec(`docker rm ${containerName} 2>/dev/null`);
  await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);

  return {
    state: "PASS",
    output: combinedOutput,
    tag: null,
    containerStarted: true,
    cleanupPerformed: true,
    preservedArtifacts: null,
  };
}

/**
 * healthcheck mode — build, start the container as a daemon, poll the Docker
 * health status, capture container logs on failure.
 *
 * The HEALTHCHECK instruction must be in the Dockerfile for this to work.
 */
export async function runHealthcheck(workspace, scenarioId, options = {}) {
  const imageTag = tag(scenarioId);
  const containerName = `decipher-hc-${Date.now()}`;
  const keepArtifacts = options.keepArtifacts === true;

  // Build
  const buildResult = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );
  if (buildResult.exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "BUILD_FAIL",
      output: buildResult.output,
      tag: null,
      containerStarted: false,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
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
      containerStarted: false,
      cleanupPerformed: true,
      preservedArtifacts: null,
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

  const allOutput = [
    `=== docker build output ===`,
    buildResult.output,
    `=== container logs ===`,
    logsResult.output,
    `=== health status: ${healthState} ===`,
  ].join("\n");

  if (healthState === "healthy") {
    if (keepArtifacts) {
      return {
        state: "PASS",
        output: allOutput,
        tag: imageTag,
        containerStarted: true,
        cleanupPerformed: false,
        preservedArtifacts: buildPreservedArtifacts({
          imageTag,
          containerName,
          status: healthState,
          mode: "healthcheck",
          running: true,
        }),
      };
    }

    await safeExec(`docker stop ${containerName} 2>/dev/null`);
    await safeExec(`docker rm ${containerName} 2>/dev/null`);
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "PASS",
      output: allOutput,
      tag: null,
      containerStarted: true,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
  }

  await safeExec(`docker stop ${containerName} 2>/dev/null`);
  await safeExec(`docker rm ${containerName} 2>/dev/null`);
  await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
  return {
    state: "HEALTHCHECK_FAIL",
    output: allOutput,
    tag: null,
    containerStarted: true,
    cleanupPerformed: true,
    preservedArtifacts: null,
  };
}

const BENCHMARK_TIMEOUT = 300_000; // 5 min — benchmarks can take time

/**
 * benchmark_run mode — build the image, run the container to completion,
 * capture all output. The container must exit 0 for PASS.
 *
 * The benchmark command can be supplied via options.benchmark_cmd or defaults
 * to whatever the image CMD specifies.
 */
export async function runBenchmark(workspace, scenarioId, options = {}) {
  const imageTag = tag(scenarioId);
  const benchmarkCmd = options.benchmark_cmd ?? "";
  const keepArtifacts = options.keepArtifacts === true;

  // Build
  const buildResult = await safeExec(
    `docker build -t ${imageTag} ${workspace} 2>&1`,
    { timeout: BUILD_TIMEOUT },
  );
  if (buildResult.exitCode !== 0) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    return {
      state: "BUILD_FAIL",
      output: buildResult.output,
      tag: null,
      containerStarted: false,
      cleanupPerformed: true,
      preservedArtifacts: null,
    };
  }

  // Run to completion
  const runResult = await safeExec(
    `docker run --rm ${imageTag} ${benchmarkCmd} 2>&1`.trim(),
    { timeout: BENCHMARK_TIMEOUT },
  );

  const combinedOutput = [
    "=== docker build output ===",
    buildResult.output,
    "=== benchmark run output ===",
    runResult.output,
  ].join("\n");

  if (runResult.exitCode !== 0) {
    if (!keepArtifacts) {
      await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
    }
    return {
      state: "BENCHMARK_FAIL",
      output: combinedOutput,
      tag: keepArtifacts ? imageTag : null,
      containerStarted: true,
      cleanupPerformed: !keepArtifacts,
      preservedArtifacts: keepArtifacts
        ? buildPreservedArtifacts({ imageTag, mode: "benchmark_run" })
        : null,
    };
  }

  if (!keepArtifacts) {
    await safeExec(`docker rmi ${imageTag} -f 2>/dev/null`);
  }
  return {
    state: "PASS",
    output: combinedOutput,
    tag: keepArtifacts ? imageTag : null,
    containerStarted: true,
    cleanupPerformed: !keepArtifacts,
    preservedArtifacts: keepArtifacts
      ? buildPreservedArtifacts({ imageTag, mode: "benchmark_run" })
      : null,
  };
}

/**
 * Dispatch to the correct runner based on execution_mode.
 * @param {'docker_build'|'docker_run'|'healthcheck'|'benchmark_run'} mode
 * @param {string} workspace
 * @param {string} scenarioId
 */
export async function runCommand(mode, workspace, scenarioId, options = {}) {
  switch (mode) {
    case "docker_run":
      return runDockerRun(workspace, scenarioId, options);
    case "healthcheck":
      return runHealthcheck(workspace, scenarioId, options);
    case "benchmark_run":
      return runBenchmark(workspace, scenarioId, options);
    case "docker_build":
    default:
      return runDockerBuild(workspace, scenarioId, options);
  }
}
