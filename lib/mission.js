function normalizeText(input) {
  return String(input ?? "").trim();
}

function lower(input) {
  return normalizeText(input).toLowerCase();
}

function buildMission({
  type,
  goal,
  domain,
  stop_boundary,
  requires_clarification = false,
  clarification_question = null,
  previous_type = null,
}) {
  return {
    type,
    goal,
    domain,
    stop_boundary,
    requires_clarification,
    clarification_question,
    previous_type,
  };
}

/**
 * Regex-based mission parser — LEGACY FALLBACK.
 *
 * The primary interactive path uses analyzeMission() from mission-analyzer.js
 * which delegates to the LLM for intent understanding. This function remains
 * as a deterministic fallback for:
 *   - unit tests that need predictable output without API calls
 *   - non-interactive / demo mode where the target type is already known
 *   - graceful degradation when the API is unavailable
 *
 * Do not extend the regex patterns here — improve prompts/analyze.md instead.
 */
export function parseMission(input) {
  const goal = normalizeText(input);
  const text = lower(input);

  if (!goal) {
    return buildMission({
      type: "clarify",
      goal,
      domain: "unknown",
      stop_boundary: "clarified",
      requires_clarification: true,
      clarification_question: "What do you want DeCIpher to do?",
    });
  }

  if (/\b(keep tuning|tuning|optimi[sz]e|rerun and tune)\b/.test(text)) {
    return buildMission({
      type: "benchmark_tune",
      goal,
      domain: "benchmark",
      stop_boundary: "user_stop",
    });
  }

  if (
    /\b(build|create|make)\b/.test(text) &&
    /\b(start|run|launch)\b/.test(text) &&
    /\b(container|image|docker)\b/.test(text)
  ) {
    return buildMission({
      type: "build_start",
      goal,
      domain: "container",
      stop_boundary: "container_running",
    });
  }

  if (
    /\b(hpl|benchmark)\b/.test(text) &&
    /\b(run|execute|start)\b/.test(text)
  ) {
    return buildMission({
      type: "benchmark_run",
      goal,
      domain: "benchmark",
      stop_boundary: "benchmark_completed",
    });
  }

  if (
    /\b(build|create|make)\b/.test(text) &&
    /\b(container|image|docker)\b/.test(text)
  ) {
    return buildMission({
      type: "build",
      goal,
      domain: "container",
      stop_boundary: "image_built",
    });
  }

  if (
    /\b(fix|repair|debug|resolve)\b/.test(text) &&
    /\b(ci|pipeline|docker|build|deploy|failure|error)\b/.test(text)
  ) {
    return buildMission({
      type: "repair",
      goal,
      domain: /\bci|pipeline\b/.test(text) ? "ci" : "deployment",
      stop_boundary: "issue_resolved",
    });
  }

  // Generic fix/repair/debug without explicit domain keyword
  if (/\b(fix|repair|debug|resolve)\b/.test(text)) {
    return buildMission({
      type: "repair",
      goal,
      domain: "deployment",
      stop_boundary: "issue_resolved",
    });
  }

  if (
    /\b(generate|create|scaffold|write|set up|setup|init|initialise|initialize)\b/.test(
      text,
    ) &&
    /\b(dockerfile|docker file|compose|ci|workflow|config|yaml|yml|makefile|github action)\b/.test(
      text,
    )
  ) {
    return buildMission({
      type: "generate",
      goal,
      domain: /\bci|workflow|github action\b/.test(text) ? "ci" : "container",
      stop_boundary: "files_generated",
    });
  }

  // "run/start/launch this container" — user wants the container running
  if (
    /\b(run|start|launch|execute)\b/.test(text) &&
    /\b(container|image|docker|this|it|scenario)\b/.test(text)
  ) {
    return buildMission({
      type: "build_start",
      goal,
      domain: "container",
      stop_boundary: "container_running",
    });
  }

  // Input contains a quoted or absolute path with any action verb → repair
  if (
    /["'\/~]/.test(goal) &&
    /\b(run|start|fix|build|deploy|check|test|launch|debug)\b/.test(text)
  ) {
    return buildMission({
      type: "repair",
      goal,
      domain: "deployment",
      stop_boundary: "issue_resolved",
    });
  }

  return buildMission({
    type: "clarify",
    goal,
    domain: "unknown",
    stop_boundary: "clarified",
    requires_clarification: true,
    clarification_question: "What do you want DeCIpher to do exactly?",
  });
}

export function updateMissionFromUserInput(currentMission, input) {
  const nextMission = parseMission(input);
  if (currentMission?.type && nextMission.type !== currentMission.type) {
    return {
      ...nextMission,
      previous_type: currentMission.type,
    };
  }
  return nextMission;
}
