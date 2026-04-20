#!/usr/bin/env node
/**
 * npm postinstall script — downloads the pre-built decipher-tui binary
 * for the user's platform from the GitHub Release.
 *
 * Falls back gracefully: if the download fails, DeCIpher still works
 * using the Node.js readline UI.
 */

import { createWriteStream } from "node:fs";
import { chmod, mkdir } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { createRequire } from "node:module";

const __dirname = dirname(fileURLToPath(import.meta.url));
const _require = createRequire(import.meta.url);
const { version } = _require("../package.json");

const REPO = "Xiaowen-Yang/DeCIpher";
const BIN_DIR = join(__dirname, "..", "bin");

function getBinaryName() {
  const platform = process.platform;
  const arch = process.arch;

  const map = {
    "darwin-arm64": "decipher-tui-darwin-arm64",
    "darwin-x64": "decipher-tui-darwin-x64",
    "linux-x64": "decipher-tui-linux-x64",
    "linux-arm64": "decipher-tui-linux-arm64",
    "win32-x64": "decipher-tui-win32-x64.exe",
  };

  return map[`${platform}-${arch}`] ?? null;
}

async function download() {
  const binaryName = getBinaryName();
  if (!binaryName) {
    console.log(
      `  decipher-tui: no pre-built binary for ${process.platform}-${process.arch}, using Node.js UI`,
    );
    return;
  }

  const url = `https://github.com/${REPO}/releases/download/v${version}/${binaryName}`;
  const dest = join(BIN_DIR, "decipher-tui");

  try {
    await mkdir(BIN_DIR, { recursive: true });

    console.log(`  decipher-tui: downloading ${binaryName}...`);
    const res = await fetch(url, { redirect: "follow" });

    if (!res.ok) {
      // Release might not exist yet (first install before release)
      console.log(
        `  decipher-tui: binary not available yet (${res.status}), using Node.js UI`,
      );
      return;
    }

    const fileStream = createWriteStream(dest);
    await pipeline(res.body, fileStream);
    await chmod(dest, 0o755);
    console.log(`  decipher-tui: installed successfully`);
  } catch (err) {
    console.log(
      `  decipher-tui: download failed (${err.message}), using Node.js UI`,
    );
  }
}

download();
