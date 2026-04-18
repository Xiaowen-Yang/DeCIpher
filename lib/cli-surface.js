import { getConfigDir, getHistoryPath, getSessionPath } from "./config.js";

const SLASH_COMMANDS = [
  "/help",
  "/model",
  "/setting",
  "/status",
  "/resume",
  "/plan",
  "/review",
  "/transcript",
  "/log",
  "/artifacts",
  "/demo",
  "/doctor",
  "/agents",
  "/quit",
];

export function buildCliStatusSnapshot(config, sessionState, persisted = null) {
  return {
    cwd: process.cwd(),
    target: sessionState.currentTarget?.path ?? persisted?.target_path ?? null,
    approval_state: sessionState.approved ?? false,
    approval_policy: sessionState.approvalPolicy ?? config.approval_policy ?? "on-request",
    model: config.model,
    provider: config.provider,
    base_url: config.base_url,
    retry_budget: config.max_iterations,
    last_verification_result: sessionState.lastVerificationResult ?? persisted?.last_verification_state ?? null,
    temp_workspace_path: sessionState.lastRunResult?.workspace ?? persisted?.workspace_path ?? null,
    writable_roots: [process.cwd(), "/tmp"],
    persistence: {
      config_dir: getConfigDir(),
      history_path: getHistoryPath(),
      session_path: getSessionPath(),
    },
    network: "blocked/unknown",
  };
}

export function buildCliReviewSnapshot(sessionState, persisted = null) {
  const patch = sessionState.lastRunResult?.patch ?? persisted?.patch ?? null;
  const writtenBack = sessionState.lastRunResult?.writtenBack ?? persisted?.written_back ?? [];
  const classification = sessionState.lastRunResult?.classification?.classification ?? persisted?.classification?.classification ?? null;
  const confidence = sessionState.lastRunResult?.classification?.confidence ?? persisted?.classification?.confidence ?? null;

  return {
    target: sessionState.currentTarget?.path ?? persisted?.target_path ?? null,
    classification,
    confidence,
    last_state: sessionState.lastVerificationResult ?? persisted?.last_verification_state ?? null,
    workspace: sessionState.lastRunResult?.workspace ?? persisted?.workspace_path ?? null,
    would_write_back: writtenBack,
    patch_preview: patch ? patch.split("\n").slice(0, 12).join("\n") : null,
  };
}

export function suggestCliSlashCommand(cmd) {
  const normalized = `/${cmd}`;
  if (normalized === "/settings") return "/setting";
  const exact = SLASH_COMMANDS.find((item) => item === normalized);
  if (exact) return exact;
  return SLASH_COMMANDS.find((item) => item.startsWith(normalized)) ?? null;
}
