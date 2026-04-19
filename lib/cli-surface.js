import {
  getConfigDir,
  getHistoryPath,
  getSessionPath,
  maskConfig,
  MODEL_CONFIG_KEYS,
  SETTING_CONFIG_KEYS,
} from "./config.js";

export const SLASH_COMMANDS = [
  { name: "/help", description: "Show help and commands" },
  { name: "/model", description: "Show or change model" },
  { name: "/setting", description: "Show or update configuration" },
  { name: "/status", description: "Show current session status" },
  { name: "/resume", description: "Resume last executor session" },
  { name: "/plan", description: "Show current mission plan" },
  { name: "/review", description: "Review current repair state and patch" },
  { name: "/transcript", description: "Show last executor transcript" },
  { name: "/log", description: "Alias for transcript" },
  { name: "/artifacts", description: "Show saved artifacts and workspace" },
  { name: "/demo", description: "Run a demo scenario" },
  { name: "/doctor", description: "Check environment" },
  { name: "/agents", description: "List available agents and skills" },
  { name: "/quit", description: "Exit" },
];

function pickConfigView(config, keys) {
  const masked = maskConfig(config);
  return Object.fromEntries(keys.map((key) => [key, masked[key] ?? null]));
}

export function buildCliModelView(config) {
  return pickConfigView(config, MODEL_CONFIG_KEYS);
}

export function buildCliSettingsView(config, sessionState, persisted = null) {
  return {
    ...pickConfigView(config, SETTING_CONFIG_KEYS),
    execution_visibility: {
      writable_roots: [process.cwd(), "/tmp"],
      network: "blocked/unknown",
      temp_workspace_path:
        sessionState.lastRunResult?.workspace ??
        persisted?.workspace_path ??
        null,
      persistence: {
        config_dir: getConfigDir(),
        history_path: getHistoryPath(),
        session_path: getSessionPath(),
      },
    },
  };
}

export function buildCliStatusSnapshot(config, sessionState, persisted = null) {
  const summary = persisted?.summary ?? null;
  return {
    cwd: process.cwd(),
    target: sessionState.currentTarget?.path ?? persisted?.target_path ?? null,
    mission_type:
      sessionState.currentMission?.type ??
      persisted?.mission?.type ??
      summary?.mission_type ??
      null,
    mission_goal:
      sessionState.currentMission?.goal ??
      persisted?.mission_summary ??
      persisted?.mission?.goal ??
      summary?.mission_summary ??
      null,
    mission_stop_boundary:
      sessionState.currentMission?.stop_boundary ??
      persisted?.mission?.stop_boundary ??
      summary?.mission_stop_boundary ??
      null,
    plan_step_ids:
      sessionState.currentPlan?.steps?.map((step) => step.id) ??
      summary?.plan_step_ids ??
      [],
    requires_clarification:
      sessionState.currentPlan?.requires_clarification ??
      summary?.requires_clarification ??
      false,
    clarification_question:
      sessionState.currentPlan?.clarification_question ??
      summary?.clarification_question ??
      null,
    approval_state: sessionState.approved ?? false,
    approval_policy:
      sessionState.approvalPolicy ?? config.approval_policy ?? "on-request",
    model: config.model,
    provider: config.provider,
    base_url: config.base_url,
    retry_budget: config.max_iterations,
    last_verification_result:
      sessionState.lastVerificationResult ??
      persisted?.last_verification_state ??
      null,
    resumable: persisted?.resumable ?? false,
    stop_reason: persisted?.stop_reason ?? null,
    temp_workspace_path:
      sessionState.lastRunResult?.workspace ??
      persisted?.workspace_path ??
      null,
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
  const writtenBack =
    sessionState.lastRunResult?.writtenBack ?? persisted?.written_back ?? [];
  const classification =
    sessionState.lastRunResult?.classification?.classification ??
    persisted?.classification?.classification ??
    null;
  const confidence =
    sessionState.lastRunResult?.classification?.confidence ??
    persisted?.classification?.confidence ??
    null;

  return {
    target: sessionState.currentTarget?.path ?? persisted?.target_path ?? null,
    mission_goal:
      sessionState.currentMission?.goal ??
      persisted?.mission_summary ??
      persisted?.mission?.goal ??
      persisted?.summary?.mission_summary ??
      null,
    requires_clarification:
      sessionState.currentPlan?.requires_clarification ??
      persisted?.plan?.requires_clarification ??
      persisted?.summary?.requires_clarification ??
      false,
    clarification_question:
      sessionState.currentPlan?.clarification_question ??
      persisted?.plan?.clarification_question ??
      persisted?.summary?.clarification_question ??
      null,
    classification,
    confidence,
    last_state:
      sessionState.lastVerificationResult ??
      persisted?.last_verification_state ??
      null,
    workspace:
      sessionState.lastRunResult?.workspace ??
      persisted?.workspace_path ??
      null,
    would_write_back: writtenBack,
    patch_preview: patch ? patch.split("\n").slice(0, 12).join("\n") : null,
  };
}

export function buildCliTranscriptView(sessionState, persisted = null) {
  const transcript =
    sessionState.lastRunResult?.transcript ?? persisted?.transcript ?? null;

  return {
    target: sessionState.currentTarget?.path ?? persisted?.target_path ?? null,
    mission_goal:
      sessionState.currentMission?.goal ??
      persisted?.mission_summary ??
      persisted?.mission?.goal ??
      persisted?.summary?.mission_summary ??
      null,
    last_state:
      sessionState.lastVerificationResult ??
      persisted?.last_verification_state ??
      null,
    transcript_path:
      sessionState.lastRunResult?.transcriptPath ??
      persisted?.transcript_path ??
      persisted?.artifact_refs?.transcript_path ??
      null,
    transcript,
  };
}

export function buildCliArtifactsView(sessionState, persisted = null) {
  const patch = sessionState.lastRunResult?.patch ?? persisted?.patch ?? null;

  return {
    target: sessionState.currentTarget?.path ?? persisted?.target_path ?? null,
    mission_goal:
      sessionState.currentMission?.goal ??
      persisted?.mission_summary ??
      persisted?.mission?.goal ??
      persisted?.summary?.mission_summary ??
      null,
    workspace:
      sessionState.lastRunResult?.workspace ??
      persisted?.workspace_path ??
      persisted?.artifact_refs?.workspace_path ??
      null,
    written_back:
      sessionState.lastRunResult?.writtenBack ?? persisted?.written_back ?? [],
    last_state:
      sessionState.lastVerificationResult ??
      persisted?.last_verification_state ??
      null,
    artifact_refs:
      sessionState.lastRunResult?.artifactRefs ??
      persisted?.artifact_refs ??
      null,
    preserved_artifacts:
      sessionState.lastRunResult?.preservedArtifacts ??
      persisted?.preserved_artifacts ??
      persisted?.artifact_refs?.preserved_artifacts ??
      null,
    patch_preview: patch ? patch.split("\n").slice(0, 8).join("\n") : null,
  };
}

export function buildCliPlanView(sessionState, persisted = null) {
  const missionPlan = sessionState.currentPlan ?? persisted?.plan ?? null;
  const missionGoal =
    sessionState.currentMission?.goal ??
    persisted?.mission_summary ??
    persisted?.mission?.goal ??
    persisted?.summary?.mission_summary ??
    null;

  const lines = [];
  if (missionGoal) {
    lines.push(`  Mission: ${missionGoal}`);
  }

  if (missionPlan?.requires_clarification) {
    lines.push(
      `  Clarification needed: ${missionPlan.clarification_question ?? "What do you want DeCIpher to do exactly?"}`,
    );
    return lines.join("\n");
  }

  if (missionPlan?.steps?.length) {
    if (lines.length > 0) {
      lines.push("");
    }
    lines.push(
      ...missionPlan.steps.map(
        (step) => `  ${step.done ? "●" : "○"} ${step.label}`,
      ),
    );
    return lines.join("\n");
  }

  const target = sessionState.currentTarget?.path ?? persisted?.target_path;
  const lastState =
    sessionState.lastVerificationResult ?? persisted?.last_verification_state;
  const patch = sessionState.lastRunResult?.patch ?? persisted?.patch;
  const writtenBack =
    sessionState.lastRunResult?.writtenBack ?? persisted?.written_back ?? [];

  const steps = [
    { label: "Resolve target", done: Boolean(target) },
    {
      label: "Reproduce failure",
      done: Boolean(lastState && lastState !== "PASS"),
    },
    {
      label: "Classify root cause",
      done: Boolean(
        sessionState.lastRunResult?.classification?.classification ??
        persisted?.classification?.classification,
      ),
    },
    { label: "Propose patch", done: Boolean(patch) },
    { label: "Verify fix", done: lastState === "PASS" },
    { label: "Write back repaired files", done: writtenBack.length > 0 },
  ];

  if (lines.length > 0) {
    lines.push("");
  }
  lines.push(
    ...steps.map((step) => `  ${step.done ? "●" : "○"} ${step.label}`),
  );
  return lines.join("\n");
}

function looksConversationalInput(input = "") {
  const text = String(input ?? "")
    .trim()
    .toLowerCase();
  if (!text) {
    return false;
  }

  if (/^(hi|hello|hey|thanks|thank you|yo|你好|您好|嗨|谢谢)\b/.test(text)) {
    return true;
  }

  if (/\?$/.test(text)) {
    return true;
  }

  return /^(what|why|how|can|could|would|should|is|are|do|does|did)\b/.test(
    text,
  );
}

export function decideCliInteraction({ route, input = "" }) {
  if (route?.mode === "clarify") {
    return {
      mode: "clarify",
      question: route.question ?? "What do you want DeCIpher to do exactly?",
    };
  }

  if (route?.mode === "execute_target") {
    return {
      mode: "execute_target",
      action: route.action ?? null,
      inferred: route.inferred ?? false,
    };
  }

  if (looksConversationalInput(input)) {
    return { mode: "conversation" };
  }

  return {
    mode: "clarify",
    question:
      "What should DeCIpher build, run, repair, or generate for this mission?",
  };
}

export function suggestCliSlashCommand(cmd) {
  const normalized = `/${cmd}`;
  if (normalized === "/settings") return "/setting";
  const exact = SLASH_COMMANDS.find((item) => item.name === normalized);
  if (exact) return exact.name;
  return (
    SLASH_COMMANDS.find((item) => item.name.startsWith(normalized))?.name ??
    null
  );
}
