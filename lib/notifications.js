import { exec as execCb } from "node:child_process";
import { promisify } from "node:util";

const exec = promisify(execCb);

export async function runCompletionNotification(notificationCommand, payload = {}) {
  if (!notificationCommand) {
    return { skipped: true };
  }

  try {
    const { stdout, stderr } = await exec(notificationCommand, {
      env: {
        ...process.env,
        DECIPHER_STATUS: payload.status ?? "",
        DECIPHER_TARGET_PATH: payload.targetPath ?? "",
        DECIPHER_WORKSPACE_PATH: payload.workspacePath ?? "",
        DECIPHER_STOP_REASON: payload.stopReason ?? "",
      },
      timeout: 5_000,
      shell: true,
    });

    return {
      skipped: false,
      exitCode: 0,
      stdout: stdout.trim(),
      stderr: stderr.trim(),
    };
  } catch (err) {
    return {
      skipped: false,
      exitCode: err.code ?? 1,
      stdout: (err.stdout ?? "").trim(),
      stderr: ((err.stderr ?? "") || err.message).trim(),
    };
  }
}
