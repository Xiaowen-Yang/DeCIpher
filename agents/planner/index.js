export function createMissionPlan(mission) {
  if (!mission || mission.requires_clarification) {
    return {
      mission,
      selected_subsystem: "clarification",
      requires_clarification: true,
      clarification_question:
        mission?.clarification_question ??
        "What do you want DeCIpher to do exactly?",
      steps: [],
    };
  }

  if (mission.type === "repair") {
    return {
      mission,
      selected_subsystem: "repair",
      requires_clarification: false,
      steps: [
        { id: "inspect_target", label: "Inspect target and context" },
        { id: "reproduce_failure", label: "Reproduce the failure" },
        { id: "classify_failure", label: "Classify root cause" },
        { id: "apply_repair", label: "Apply repair" },
        { id: "verify_repair", label: "Verify repair" },
      ],
    };
  }

  if (mission.type === "build_start") {
    return {
      mission,
      selected_subsystem: "generation_or_repair",
      requires_clarification: false,
      steps: [
        { id: "inspect_target", label: "Inspect target and environment" },
        {
          id: "generate_or_repair_assets",
          label: "Generate or repair required assets",
        },
        { id: "build_container", label: "Build the container" },
        { id: "start_container", label: "Start the container" },
        {
          id: "verify_container_running",
          label: "Verify the container is running",
        },
      ],
    };
  }

  if (mission.type === "build") {
    return {
      mission,
      selected_subsystem: "generation_or_repair",
      requires_clarification: false,
      steps: [
        { id: "inspect_target", label: "Inspect target and environment" },
        {
          id: "generate_or_repair_assets",
          label: "Generate or repair required assets",
        },
        { id: "build_container", label: "Build the container" },
        { id: "verify_image_built", label: "Verify the image was built" },
      ],
    };
  }

  if (mission.type === "benchmark_run") {
    return {
      mission,
      selected_subsystem: "generation_or_repair",
      requires_clarification: false,
      steps: [
        { id: "inspect_target", label: "Inspect target and environment" },
        {
          id: "generate_or_repair_assets",
          label: "Generate or repair required assets",
        },
        { id: "build_container", label: "Build the container" },
        { id: "start_container", label: "Start the container" },
        { id: "run_benchmark", label: "Run the benchmark" },
        { id: "collect_benchmark_result", label: "Collect benchmark results" },
      ],
    };
  }

  if (mission.type === "generate") {
    return {
      mission,
      selected_subsystem: "generation",
      requires_clarification: false,
      steps: [
        {
          id: "inspect_target",
          label: "Inspect target directory and existing files",
        },
        { id: "generate_files", label: "Generate required files" },
        {
          id: "verify_generated",
          label: "Verify generated files are executable",
        },
      ],
    };
  }

  if (mission.type === "greenfield") {
    return {
      mission,
      selected_subsystem: "generation",
      requires_clarification: false,
      steps: [
        {
          id: "understand_goal",
          label: "Understand user goal and requirements",
        },
        {
          id: "generate_all_files",
          label: "Generate all required files from scratch",
        },
        {
          id: "build_and_test",
          label: "Build and test the generated artifacts",
        },
        {
          id: "debug_and_iterate",
          label: "Debug failures and iterate until working",
        },
        { id: "verify_outcome", label: "Verify the user's stated goal is met" },
      ],
    };
  }

  if (mission.type === "clone_and_run") {
    return {
      mission,
      selected_subsystem: "generation",
      requires_clarification: false,
      steps: [
        { id: "clone_repo", label: "Clone the Git repository" },
        { id: "read_readme", label: "Read README and understand the project" },
        {
          id: "check_docker_config",
          label: "Check for existing Dockerfile or docker-compose.yml",
        },
        {
          id: "build_image",
          label: "Build the Docker image (generate Dockerfile if needed)",
        },
        { id: "run_container", label: "Run the container" },
        { id: "verify_running", label: "Verify the container is running" },
      ],
    };
  }

  if (mission.type === "benchmark_tune") {
    return {
      mission,
      selected_subsystem: "generation_or_repair",
      requires_clarification: false,
      steps: [
        { id: "inspect_target", label: "Inspect target and environment" },
        {
          id: "generate_or_repair_assets",
          label: "Generate or repair required assets",
        },
        { id: "run_benchmark", label: "Run benchmark iteration" },
        { id: "collect_benchmark_result", label: "Collect benchmark results" },
        { id: "adjust_parameters", label: "Adjust tuning parameters" },
      ],
    };
  }

  return {
    mission,
    selected_subsystem: "clarification",
    requires_clarification: true,
    clarification_question: "What do you want DeCIpher to do exactly?",
    steps: [],
  };
}

/**
 * Infer a default action from the resolved target type.
 * Used when the mission plan is unclear but the target is known.
 */
function defaultActionForTarget(target) {
  switch (target.type) {
    case "dockerfile":
      return "docker_build";
    case "logfile":
      return "triage_only";
    default:
      return "fix";
  }
}

export function selectMissionRoute(plan, target) {
  // No target at all — must clarify before we can do anything.
  if (!target) {
    return {
      mode: "clarify",
      question:
        "Which directory, scenario, Dockerfile, or log should DeCIpher use for this mission?",
    };
  }

  // Generation subsystem takes priority over target-type routing.
  if (plan?.selected_subsystem === "generation") {
    return { mode: "execute_target", action: "generate" };
  }

  // When the mission is unclear but a target IS resolved, default to executing
  // based on target type rather than blocking with a clarification gate.
  // The agent will announce what it understood and proceed.
  if (!plan || plan.requires_clarification) {
    return {
      mode: "execute_target",
      action: defaultActionForTarget(target),
      inferred: true, // flag so the CLI can announce it
    };
  }

  // Benchmark subsystems get explicit routing so the agent loop receives the
  // correct synthetic mission goal (e.g. "Run the benchmark to completion").
  if (plan.mission?.type === "benchmark_run") {
    return { mode: "execute_target", action: "benchmark_run" };
  }

  if (plan.mission?.type === "benchmark_tune") {
    return { mode: "execute_target", action: "benchmark_run" };
  }

  if (target.type === "dockerfile") {
    return { mode: "execute_target", action: "docker_build" };
  }

  if (target.type === "logfile") {
    return { mode: "execute_target", action: "triage_only" };
  }

  return { mode: "execute_target", action: "fix" };
}
