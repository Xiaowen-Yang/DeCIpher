import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  appendHistoryEntry,
  findReverseHistoryMatches,
  loadHistoryEntries,
  toReadlineHistory,
} from "../../lib/history.js";

test("loadHistoryEntries returns empty array when file does not exist", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-history-"));
  const historyPath = join(dir, "history.jsonl");

  const entries = await loadHistoryEntries(historyPath);

  assert.deepEqual(entries, []);
});

test("loadHistoryEntries keeps valid text entries and skips invalid rows", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-history-"));
  const historyPath = join(dir, "history.jsonl");

  await writeFile(
    historyPath,
    [
      '{"text":"first prompt"}',
      "",
      "not json",
      '{"text":42}',
      '{"role":"user","content":"ignored"}',
      '{"text":"second prompt","extra":"discarded"}',
    ].join("\n"),
    "utf8",
  );

  const entries = await loadHistoryEntries(historyPath);

  assert.deepEqual(entries, [
    { text: "first prompt" },
    { text: "second prompt" },
  ]);
});

test("appendHistoryEntry creates the file and stores text-only rows", async () => {
  const dir = await mkdtemp(join(tmpdir(), "decipher-history-"));
  const historyPath = join(dir, "nested", "history.jsonl");

  await appendHistoryEntry(historyPath, {
    text: "first prompt",
    role: "user",
    meta: { ignored: true },
  });
  await appendHistoryEntry(historyPath, "second prompt");

  const raw = await readFile(historyPath, "utf8");
  const entries = await loadHistoryEntries(historyPath);

  assert.deepEqual(
    raw.trim().split("\n").map((line) => JSON.parse(line)),
    [
      { text: "first prompt" },
      { text: "second prompt" },
    ],
  );
  assert.deepEqual(entries, [
    { text: "first prompt" },
    { text: "second prompt" },
  ]);
});

test("toReadlineHistory returns newest-first text values", () => {
  const history = toReadlineHistory([
    { text: "oldest" },
    { text: "middle" },
    { text: "newest" },
  ]);

  assert.deepEqual(history, ["newest", "middle", "oldest"]);
});

test("findReverseHistoryMatches returns newest-first exact-text deduped matches", () => {
  const matches = findReverseHistoryMatches(
    [
      { text: "deploy api" },
      { text: "run tests" },
      { text: "deploy api" },
      { text: "deploy web" },
      { text: "debug worker" },
    ],
    "deploy",
  );

  assert.deepEqual(matches, ["deploy web", "deploy api"]);
});
