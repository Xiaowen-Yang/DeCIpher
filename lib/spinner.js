/**
 * Animated spinner for long-running operations.
 * Shows elapsed time and a cycling dot, similar to Claude Code's "Nucleating…" indicator.
 *
 * Usage:
 *   const sp = startSpinner("Triaging failure");
 *   await doWork();
 *   sp.stop("done");         // prints final line
 *   sp.stop();               // clears line
 */

import pc from "picocolors";

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const INTERVAL = 80; // ms per frame

/**
 * Start a spinner in the terminal.
 *
 * @param {string} label  Text shown beside the spinner
 * @param {{ tokens?: boolean }} opts
 * @returns {{ stop(message?: string): void, update(label: string): void }}
 */
export function startSpinner(label, opts = {}) {
  let frame = 0;
  let elapsed = 0;
  let currentLabel = label;
  let stopped = false;

  const isTTY = process.stdout.isTTY;

  function render() {
    if (!isTTY || stopped) return;
    const spinner = pc.cyan(FRAMES[frame % FRAMES.length]);
    const time = pc.dim(`${(elapsed / 1000).toFixed(1)}s`);
    const line = `  ${spinner} ${pc.dim(currentLabel + "…")} ${time}`;
    // Clear to end of line and overwrite
    process.stdout.write(`\r\x1b[K${line}`);
  }

  const timer = setInterval(() => {
    frame++;
    elapsed += INTERVAL;
    render();
  }, INTERVAL);

  render();

  return {
    update(newLabel) {
      currentLabel = newLabel;
    },

    stop(message) {
      if (stopped) return;
      stopped = true;
      clearInterval(timer);

      if (isTTY) {
        process.stdout.write(`\r\x1b[K`); // clear spinner line
        if (message) {
          const time = pc.dim(`(${(elapsed / 1000).toFixed(1)}s)`);
          console.log(`  ${pc.green("✓")} ${message} ${time}`);
        }
      }
      // When stdout is not a TTY (server-mode), skip console output entirely.
      // In server-mode, console.log is redirected to stderr, and the agent-bridge
      // would wrap it as ServerMessage::Error — corrupting the TUI display.
    },
  };
}
