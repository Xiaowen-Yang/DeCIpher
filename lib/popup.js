/**
 * Terminal popup renderer for command palettes and path completion.
 *
 * Uses ONLY relative cursor movement (up/down) — never save/restore cursor.
 * Save/restore breaks when the terminal scrolls (typing near bottom of screen).
 */

import pc from "picocolors";

const MAX_VISIBLE = 8;

/**
 * @typedef {object} PopupItem
 * @property {string} label
 * @property {string} [description]
 */

export function createPopup() {
  let renderedLines = 0;
  const out = process.stdout;
  const isTTY = out.isTTY;

  function render(items, selectedIndex = 0) {
    if (!isTTY) return;

    // Clear previous render first
    clear();

    if (items.length === 0) return;

    const cols = out.columns || 80;
    const visible = items.slice(0, MAX_VISIBLE);
    const hasMore = items.length > MAX_VISIBLE;

    const lines = [];
    for (let i = 0; i < visible.length; i++) {
      const item = visible[i];
      const isSelected = i === selectedIndex;
      const label = item.label ?? "";
      const desc = item.description ?? "";

      const labelWidth = Math.min(20, Math.floor(cols * 0.3));
      const paddedLabel = label.padEnd(labelWidth);
      const descWidth = Math.max(10, cols - labelWidth - 6);
      const truncDesc =
        desc.length > descWidth ? desc.slice(0, descWidth - 1) + "…" : desc;

      if (isSelected) {
        lines.push(
          `  ${pc.bold(pc.cyan(paddedLabel))}  ${pc.white(truncDesc)}`,
        );
      } else {
        lines.push(`  ${pc.cyan(paddedLabel)}  ${pc.dim(truncDesc)}`);
      }
    }

    if (hasMore) {
      lines.push(`  ${pc.dim(`… ${items.length - MAX_VISIBLE} more`)}`);
    }

    // Render: move to next line, write each line, move back up
    // Each line: \n (move down) + \x1b[2K (clear line) + content
    let seq = "";
    for (const line of lines) {
      seq += `\n\x1b[2K${line}`;
    }
    // Move cursor back up to the input line
    seq += `\x1b[${lines.length}A`;
    // Move cursor to the end of the current input (carriage return + right)
    seq += `\r`;

    out.write(seq);
    renderedLines = lines.length;

    // Restore cursor position on the input line by triggering readline refresh
    // This is done by the caller (rl._refreshLine)
  }

  function clear() {
    if (!isTTY || renderedLines === 0) return;

    // Move down, clear each line, move back up
    let seq = "";
    for (let i = 0; i < renderedLines; i++) {
      seq += `\n\x1b[2K`;
    }
    seq += `\x1b[${renderedLines}A\r`;

    out.write(seq);
    renderedLines = 0;
  }

  function getRenderedCount() {
    return renderedLines;
  }

  return { render, clear, getRenderedCount };
}

/**
 * Filter slash commands using fuzzy subsequence matching.
 */
export function filterCommandsForPopup(prefix, commands) {
  const query = prefix.replace("/", "").toLowerCase();

  if (!query) {
    return commands.map((c) => ({
      label: c.name,
      description: c.description,
    }));
  }

  return commands
    .filter((c) => {
      const name = c.name.slice(1).toLowerCase();
      let qi = 0;
      for (let ni = 0; ni < name.length && qi < query.length; ni++) {
        if (name[ni] === query[qi]) qi++;
      }
      return qi === query.length;
    })
    .map((c) => ({
      label: c.name,
      description: c.description,
    }));
}

/**
 * Filter directory entries for path completion popup.
 */
export function filterPathsForPopup(partial) {
  try {
    const path = require("node:path");
    const fs = require("node:fs");
    const os = require("node:os");

    const expanded = partial.startsWith("~")
      ? partial.replace("~", os.homedir())
      : partial;

    let dir, prefix;
    try {
      if (fs.statSync(expanded).isDirectory()) {
        dir = expanded;
        prefix = "";
      } else {
        dir = path.dirname(expanded);
        prefix = path.basename(expanded);
      }
    } catch {
      dir = path.dirname(expanded);
      prefix = path.basename(expanded);
    }

    const entries = fs.readdirSync(dir).filter((e) => !e.startsWith("."));
    return entries
      .filter((e) => e.toLowerCase().startsWith(prefix.toLowerCase()))
      .slice(0, 20)
      .map((e) => {
        const full = path.join(
          dir === expanded ? partial : path.dirname(partial),
          e,
        );
        const isDir = fs.statSync(path.join(dir, e)).isDirectory();
        return {
          label: isDir ? full + "/" : full,
          description: isDir ? "directory" : "file",
        };
      });
  } catch {
    return [];
  }
}
