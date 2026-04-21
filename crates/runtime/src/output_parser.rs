//! Exec output classification engine (Phase A — Smart Card System).
//!
//! Takes `(cmd, stdout, exit_code)` from a completed exec_command and returns
//! a `ParsedOutput` enum describing what happened.
//!
//! Rules:
//! - Pure synchronous functions — no I/O, no side effects.
//! - No external dependencies beyond `serde`.
//! - The LLM always receives raw `ToolOutput.llm_text`. ParsedOutput is TUI-only.
//! - Parsers run AFTER exec completes. Never modify execution behavior.

use serde::{Deserialize, Serialize};

// ── ParsedOutput enum ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParsedOutput {
    TestSuite(TestSuiteResult),
    DockerBuild(DockerBuildResult),
    DockerRun(DockerRunResult),
    Compose(ComposeResult),
    GitOp(GitResult),
    Lint(LintResult),
    KubePod(KubePodResult),
    KubeLog(KubeLogResult),
    KubeEvent(KubeEventResult),
    Ci(CiResult),
    EnvSetup(EnvSetupResult),
    Migration(MigrationResult),
    Generic,
}

// ── Struct definitions ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestFailure {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestSuiteResult {
    pub runner: String,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub coverage: Option<f32>,
    pub failures: Vec<TestFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DockerBuildResult {
    pub image: String,
    pub steps_total: u32,
    pub steps_done: u32,
    pub stages: Vec<String>,
    pub size_mb: Option<f32>,
    pub cached: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DockerRunResult {
    pub container: String,
    pub ports: Vec<String>,
    pub health: Option<String>,
    pub exit_code: i32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeService {
    pub name: String,
    pub image: String,
    pub port: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeResult {
    pub services: Vec<ComposeService>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GitResult {
    pub op: String,
    pub branch: Option<String>,
    pub files_changed: u32,
    pub additions: u32,
    pub deletions: u32,
    pub conflicts: Vec<String>,
    pub commit_msg: Option<String>,
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LintItem {
    pub location: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LintResult {
    pub tool: String,
    pub warnings: u32,
    pub errors: u32,
    pub items: Vec<LintItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PodStatus {
    pub name: String,
    pub ready: String,
    pub status: String,
    pub restarts: u32,
    pub age: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KubePodResult {
    pub resource: String,
    pub pods: Vec<PodStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KubeLogResult {
    pub pod: String,
    pub lines_total: u32,
    pub errors: u32,
    pub warnings: u32,
    pub highlights: Vec<String>,
    pub root_cause: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KubeEventEntry {
    pub kind: String,
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KubeEventResult {
    pub namespace: String,
    pub events: Vec<KubeEventEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CiStage {
    pub name: String,
    pub status: String,
    pub elapsed: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CiResult {
    pub pipeline_id: Option<String>,
    pub stages: Vec<CiStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvSetupResult {
    pub manager: String,
    pub packages: u32,
    pub vulnerabilities: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrationResult {
    pub name: Option<String>,
    pub applied: u32,
    pub error: Option<String>,
}

// ── Public dispatcher ──────────────────────────────────────────────────────────

/// Parse exec_command output into a structured `ParsedOutput`.
///
/// Tries parsers in priority order; returns `Generic` if none match.
/// Pure and synchronous — call after exec completes.
pub fn parse_output(cmd: &str, stdout: &str, exit_code: i32) -> ParsedOutput {
    try_parse_test_suite(cmd, stdout, exit_code)
        .or_else(|| try_parse_docker_build(cmd, stdout, exit_code))
        .or_else(|| try_parse_docker_run(cmd, stdout, exit_code))
        .or_else(|| try_parse_compose(cmd, stdout, exit_code))
        .or_else(|| try_parse_git(cmd, stdout, exit_code))
        .or_else(|| try_parse_lint(cmd, stdout, exit_code))
        .or_else(|| try_parse_kube_pod(cmd, stdout, exit_code))
        .or_else(|| try_parse_kube_log(cmd, stdout, exit_code))
        .or_else(|| try_parse_kube_event(cmd, stdout, exit_code))
        .or_else(|| try_parse_ci(cmd, stdout, exit_code))
        .or_else(|| try_parse_env_setup(cmd, stdout, exit_code))
        .or_else(|| try_parse_migration(cmd, stdout, exit_code))
        .unwrap_or(ParsedOutput::Generic)
}

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Extract the last whitespace-delimited number before `keyword` in `s`.
/// Returns None if no such number exists.
fn extract_num_before(s: &str, keyword: &str) -> Option<u32> {
    let pos = s.find(keyword)?;
    let before = s[..pos].trim();
    before.split_whitespace().last()?.parse::<u32>().ok()
}

/// Extract a percentage value from a line containing "XX.X%".
fn parse_percent(line: &str) -> Option<f32> {
    let pct_pos = line.find('%')?;
    let before = line[..pct_pos].trim();
    before.split_whitespace().last()?.parse::<f32>().ok()
}

// ── TestSuite ──────────────────────────────────────────────────────────────────

fn try_parse_test_suite(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    let runner = detect_test_runner(cmd)?;

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut coverage: Option<f32> = None;
    let mut failures: Vec<TestFailure> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();

        // cargo test: "test result: ok. 78 passed; 0 failed; 2 ignored; ..."
        if line.starts_with("test result:") {
            for part in line.split(';') {
                let part = part.trim();
                if let Some(n) = extract_num_before(part, "passed") { passed = n; }
                if let Some(n) = extract_num_before(part, "failed") { failed = n; }
                if let Some(n) = extract_num_before(part, "ignored") { skipped = n; }
            }
        }

        // jest: "Tests: 5 failed, 78 passed, 83 total"
        if line.starts_with("Tests:") {
            for part in line.split(',') {
                let part = part.trim();
                if let Some(n) = extract_num_before(part, "passed") { passed = n; }
                if let Some(n) = extract_num_before(part, "failed") { failed = n; }
                if let Some(n) = extract_num_before(part, "skipped") { skipped += n; }
                if let Some(n) = extract_num_before(part, "todo") { skipped += n; }
            }
        }

        // pytest: "===== 2 failed, 78 passed in 5.2s ====="
        if line.starts_with('=') && line.ends_with('=') && line.contains("passed") {
            let stripped = line.trim_matches(|c: char| c == '=' || c == ' ');
            for part in stripped.split(',') {
                let part = part.trim();
                if let Some(n) = extract_num_before(part, "passed") { passed = n; }
                if let Some(n) = extract_num_before(part, "failed") { failed = n; }
                if let Some(n) = extract_num_before(part, "skipped") { skipped = n; }
            }
        }

        // go test: "ok  github.com/foo/bar  0.312s"
        if line.starts_with("ok ") { passed += 1; }
        if line == "FAIL" { failed += 1; }

        // Coverage: "coverage: 78.5% of statements"
        if line.contains("coverage:") && line.contains('%') {
            if let Some(cov) = parse_percent(line) {
                coverage = Some(cov);
            }
        }

        // Cargo test inline failure: "test tests::foo::bar ... FAILED"
        if line.starts_with("test ") && line.ends_with("... FAILED") {
            let name = line["test ".len()..].trim_end_matches("... FAILED").trim().to_string();
            if !name.is_empty() && failures.len() < 5 {
                failures.push(TestFailure { name, message: String::new() });
            }
        }
    }

    Some(ParsedOutput::TestSuite(TestSuiteResult {
        runner: runner.to_string(),
        passed,
        failed,
        skipped,
        coverage,
        failures,
    }))
}

fn detect_test_runner(cmd: &str) -> Option<&'static str> {
    if cmd.contains("cargo test") { Some("cargo test") }
    else if cmd.contains("npx jest") || (cmd.contains("jest") && !cmd.contains("npm test")) { Some("jest") }
    else if cmd.contains("pytest") { Some("pytest") }
    else if cmd.contains("go test") { Some("go test") }
    else if cmd.contains("vitest") { Some("vitest") }
    else if cmd.contains("npm test") { Some("npm test") }
    else { None }
}

// ── DockerBuild ────────────────────────────────────────────────────────────────

fn try_parse_docker_build(cmd: &str, stdout: &str, exit_code: i32) -> Option<ParsedOutput> {
    if !cmd.contains("docker build") { return None; }

    let mut steps_total = 0u32;
    let mut steps_done = 0u32;
    let mut stages: Vec<String> = Vec::new();
    let mut size_mb: Option<f32> = None;
    let mut cached = false;
    let mut error: Option<String> = None;

    // Extract image name from "-t name" flag
    let image = extract_flag_value(cmd, "-t")
        .or_else(|| extract_flag_value(cmd, "--tag"))
        .unwrap_or_default();

    for line in stdout.lines() {
        let line = line.trim();

        // Classic format: "Step 9/12 : RUN npm install"
        if line.starts_with("Step ") && line.contains('/') && line.contains(':') {
            if let Some(step_part) = line.strip_prefix("Step ").and_then(|s| s.split(':').next()) {
                if let Some((n_str, total_str)) = step_part.trim().split_once('/') {
                    if let (Ok(n), Ok(t)) = (n_str.trim().parse::<u32>(), total_str.trim().parse::<u32>()) {
                        steps_done = n;
                        steps_total = t;
                    }
                }
            }
        }

        // BuildKit stage: "#3 [builder 1/5] FROM node:22"
        if line.starts_with('#') {
            if let Some(stage) = extract_buildkit_stage(line) {
                if !stages.contains(&stage) { stages.push(stage); }
            }
        }

        // CACHED
        if line.contains("CACHED") { cached = true; }

        // Size in MB
        if line.contains("MB") {
            if let Some(mb) = extract_size_mb(line) { size_mb = Some(mb); }
        }

        // Error (exit_code != 0)
        if exit_code != 0 && error.is_none() {
            if line.starts_with("ERROR") || line.contains("failed to solve") || line.contains("error:") {
                error = Some(line.chars().take(200).collect());
            }
        }
    }

    Some(ParsedOutput::DockerBuild(DockerBuildResult {
        image,
        steps_total,
        steps_done,
        stages,
        size_mb,
        cached,
        error,
    }))
}

fn extract_flag_value(cmd: &str, flag: &str) -> Option<String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let pos = parts.iter().position(|&p| p == flag)?;
    parts.get(pos + 1).map(|s| s.to_string())
}

fn extract_buildkit_stage(line: &str) -> Option<String> {
    // "#3 [builder 1/5] FROM node:22" → "builder"
    let start = line.find('[')? + 1;
    let end = line.find(']')?;
    if end <= start { return None; }
    let inner = &line[start..end];
    let stage = inner.split_whitespace().next()?.to_string();
    // Skip numeric-only tokens (raw step numbers)
    if stage.parse::<u32>().is_ok() { return None; }
    Some(stage)
}

fn extract_size_mb(line: &str) -> Option<f32> {
    let pos = line.find("MB")?;
    let before = line[..pos].trim();
    before.split_whitespace().last()?.parse::<f32>().ok()
}

// ── DockerRun ──────────────────────────────────────────────────────────────────

fn try_parse_docker_run(cmd: &str, stdout: &str, exit_code: i32) -> Option<ParsedOutput> {
    if !cmd.contains("docker run") { return None; }

    let mut container = String::new();
    let mut ports: Vec<String> = Vec::new();
    let mut health: Option<String> = None;
    let mut error: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();

        // Container ID: 64-char hex on first meaningful line
        if container.is_empty() && line.len() >= 12
            && line[..12.min(line.len())].chars().all(|c| c.is_ascii_hexdigit())
        {
            container = line[..12.min(line.len())].to_string();
        }

        // Port mapping: "0.0.0.0:3000->3000/tcp"
        if line.contains("->") && (line.contains("/tcp") || line.contains("/udp")) {
            ports.push(line.to_string());
        }

        // Health
        if line.contains("healthy") || line.contains("starting") || line.contains("unhealthy") {
            health = Some(line.chars().take(60).collect());
        }

        // Error
        if exit_code != 0 && error.is_none()
            && (line.starts_with("Error") || line.starts_with("docker:"))
        {
            error = Some(line.chars().take(200).collect());
        }
    }

    // Require at least one signal of docker run output
    if container.is_empty() && ports.is_empty() && health.is_none() && error.is_none() {
        return None;
    }

    Some(ParsedOutput::DockerRun(DockerRunResult {
        container,
        ports,
        health,
        exit_code,
        error,
    }))
}

// ── Compose ────────────────────────────────────────────────────────────────────

fn try_parse_compose(cmd: &str, stdout: &str, exit_code: i32) -> Option<ParsedOutput> {
    if !cmd.contains("docker compose") && !cmd.contains("docker-compose") { return None; }

    let mut services: Vec<ComposeService> = Vec::new();

    // Parse `docker compose ps` table
    // "NAME     IMAGE    COMMAND   SERVICE   CREATED   STATUS   PORTS"
    let mut in_ps = false;
    for line in stdout.lines() {
        let line = line.trim();

        if line.starts_with("NAME") && line.contains("IMAGE") && line.contains("STATUS") {
            in_ps = true;
            continue;
        }
        if in_ps && !line.is_empty() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let status = if parts.iter().any(|&p| p == "Up" || p.starts_with("Up")) {
                    "up"
                } else {
                    "exit"
                };
                services.push(ComposeService {
                    name: parts[0].to_string(),
                    image: parts[1].to_string(),
                    port: parts.get(6).map(|p| p.to_string()),
                    status: status.to_string(),
                });
            }
        }
    }

    // Parse compose up output lines
    // "Container project-web-1  Created" / "Container project-db-1  Started"
    if services.is_empty() {
        for line in stdout.lines() {
            let line = line.trim();
            let status = if line.ends_with("Created") || line.ends_with("Started") || line.ends_with("Running") {
                if exit_code == 0 { "up" } else { "error" }
            } else if line.ends_with("Error") || line.ends_with("error") {
                "error"
            } else {
                continue;
            };

            let name = line.split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            if !name.is_empty() && services.len() < 20 {
                services.push(ComposeService {
                    name,
                    image: String::new(),
                    port: None,
                    status: status.to_string(),
                });
            }
        }
    }

    if services.is_empty() { return None; }

    Some(ParsedOutput::Compose(ComposeResult { services }))
}

// ── Git ────────────────────────────────────────────────────────────────────────

fn try_parse_git(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    let op = detect_git_op(cmd)?;

    let mut branch: Option<String> = None;
    let mut files_changed = 0u32;
    let mut additions = 0u32;
    let mut deletions = 0u32;
    let mut conflicts: Vec<String> = Vec::new();
    let mut commit_msg: Option<String> = None;
    let mut remote_url: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();

        // "On branch main"
        if line.starts_with("On branch ") {
            branch = Some(line["On branch ".len()..].to_string());
        }

        // "[main abc1234] feat: fix bug"
        if line.starts_with('[') && line.contains(']') {
            // Extract branch from "[main ...]"
            if let Some(inner) = line.strip_prefix('[') {
                if let Some(b) = inner.split_whitespace().next() {
                    if branch.is_none() { branch = Some(b.to_string()); }
                }
            }
            // Extract commit message after "]"
            if let Some(msg) = line.splitn(2, ']').nth(1) {
                let msg = msg.trim();
                if !msg.is_empty() && commit_msg.is_none() {
                    commit_msg = Some(msg.chars().take(80).collect());
                }
            }
        }

        // "3 files changed, 42 insertions(+), 18 deletions(-)"
        if line.contains("file") && line.contains("changed") {
            for part in line.split(',') {
                let part = part.trim();
                if part.contains("file") { if let Some(n) = extract_num_before(part, "file") { files_changed = n; } }
                if part.contains("insertion") { if let Some(n) = extract_num_before(part, "insertion") { additions = n; } }
                if part.contains("deletion") { if let Some(n) = extract_num_before(part, "deletion") { deletions = n; } }
            }
        }

        // CONFLICT lines
        if line.starts_with("CONFLICT") {
            let path = line.split(':').nth(1).unwrap_or("").trim().to_string();
            if !path.is_empty() && conflicts.len() < 10 { conflicts.push(path); }
        }
        if line.starts_with("both modified:") || line.starts_with("both added:") {
            if let Some(path) = line.splitn(2, ':').nth(1) {
                conflicts.push(path.trim().to_string());
            }
        }

        // Remote URL
        if line.starts_with("remote:") && (line.contains("https://") || line.contains("git@")) {
            remote_url = Some(line["remote:".len()..].trim().to_string());
        }
    }

    Some(ParsedOutput::GitOp(GitResult {
        op: op.to_string(),
        branch,
        files_changed,
        additions,
        deletions,
        conflicts,
        commit_msg,
        remote_url,
    }))
}

fn detect_git_op(cmd: &str) -> Option<&'static str> {
    if cmd.contains("git commit") { Some("commit") }
    else if cmd.contains("git push") { Some("push") }
    else if cmd.contains("git merge") { Some("merge") }
    else if cmd.contains("git pull") { Some("pull") }
    else if cmd.contains("git rebase") { Some("rebase") }
    else if cmd.contains("git status") { Some("status") }
    else if cmd.contains("git diff") { Some("diff") }
    else if cmd.contains("git log") { Some("log") }
    else { None }
}

// ── Lint ───────────────────────────────────────────────────────────────────────

fn try_parse_lint(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    let tool = detect_lint_tool(cmd)?;

    let mut warnings = 0u32;
    let mut errors = 0u32;
    let mut items: Vec<LintItem> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        match tool {
            "cargo clippy" => {
                // "warning: unused variable `x`"
                if line.starts_with("warning:") && !line.contains("warning(s) emitted") {
                    warnings += 1;
                    if items.len() < 5 {
                        items.push(LintItem { location: String::new(), message: line.chars().take(120).collect() });
                    }
                }
                // "warning: N warnings emitted" — authoritative count
                if line.starts_with("warning:") && line.contains("warning") && line.contains("emitted") {
                    if let Some(n) = extract_num_before(line, "warning") { warnings = n; }
                }
                if line.starts_with("error[") || (line.starts_with("error:") && !line.contains("aborting")) {
                    errors += 1;
                }
            }
            "eslint" => {
                // "src/app.js:12:5: error: 'x' is not defined"
                if line.contains(": error ") || line.contains(": Error ") {
                    errors += 1;
                    if items.len() < 5 {
                        let loc = line.splitn(3, ':').take(2).collect::<Vec<_>>().join(":");
                        items.push(LintItem { location: loc, message: line.chars().take(120).collect() });
                    }
                }
                if line.contains(": warning ") { warnings += 1; }
                // Summary: "✖ 3 problems (2 errors, 1 warning)"
                if line.contains("problems") {
                    if let Some(n) = extract_num_before(line, "error") { errors = n; }
                    if let Some(n) = extract_num_before(line, "warning") { warnings = n; }
                }
            }
            "ruff" => {
                // "src/app.py:12:5: E501 Line too long"
                let parts: Vec<&str> = line.splitn(3, ':').collect();
                if parts.len() >= 2 && parts[0].ends_with(".py") {
                    errors += 1;
                    if items.len() < 5 {
                        items.push(LintItem {
                            location: parts[..2].join(":"),
                            message: line.chars().take(120).collect(),
                        });
                    }
                }
                // "Found 5 errors."
                if line.starts_with("Found ") && line.contains("error") {
                    if let Some(n) = extract_num_before(line, "error") { errors = n; }
                }
            }
            "prettier" => {
                if line.contains("[warn]") { warnings += 1; }
            }
            "tsc" => {
                // "src/app.ts(12,5): error TS2322: ..."
                if line.contains("error TS") {
                    errors += 1;
                    if items.len() < 5 {
                        let loc = line.split(':').next().unwrap_or("").to_string();
                        items.push(LintItem { location: loc, message: line.chars().take(120).collect() });
                    }
                }
            }
            _ => {}
        }
    }

    Some(ParsedOutput::Lint(LintResult {
        tool: tool.to_string(),
        warnings,
        errors,
        items,
    }))
}

fn detect_lint_tool(cmd: &str) -> Option<&'static str> {
    if cmd.contains("cargo clippy") { Some("cargo clippy") }
    else if cmd.contains("eslint") { Some("eslint") }
    else if cmd.contains("ruff") { Some("ruff") }
    else if cmd.contains("prettier") && (cmd.contains("--check") || cmd.contains("--write")) { Some("prettier") }
    else if cmd.contains("tsc") && cmd.contains("--noEmit") { Some("tsc") }
    else { None }
}

// ── KubePod ────────────────────────────────────────────────────────────────────

fn try_parse_kube_pod(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    let is_pods = cmd.contains("kubectl") && (cmd.contains("get pod") || cmd.contains("rollout"));
    if !is_pods { return None; }

    let resource = if cmd.contains("rollout") { "rollout" } else { "pods" };
    let mut pods: Vec<PodStatus> = Vec::new();
    let mut in_table = false;

    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("NAME") && line.contains("READY") && line.contains("STATUS") {
            in_table = true;
            continue;
        }
        if in_table && !line.is_empty() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                pods.push(PodStatus {
                    name: parts[0].to_string(),
                    ready: parts[1].to_string(),
                    status: parts[2].to_string(),
                    restarts: parts[3].parse::<u32>().unwrap_or(0),
                    age: parts.get(4).unwrap_or(&"").to_string(),
                });
            }
        }
    }

    let has_rollout_info = stdout.contains("deployment") || stdout.contains("rolled out");
    if pods.is_empty() && !has_rollout_info { return None; }

    Some(ParsedOutput::KubePod(KubePodResult {
        resource: resource.to_string(),
        pods,
    }))
}

// ── KubeLog ────────────────────────────────────────────────────────────────────

fn try_parse_kube_log(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    if !cmd.contains("kubectl logs") && !cmd.contains("kubectl_logs") { return None; }

    let pod = cmd.split_whitespace()
        .skip_while(|&p| p != "logs")
        .nth(1)
        .unwrap_or("")
        .to_string();

    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut highlights: Vec<String> = Vec::new();
    let mut root_cause: Option<String> = None;
    let lines_total = stdout.lines().count() as u32;

    for line in stdout.lines() {
        let upper = line.to_uppercase();
        let is_error = upper.contains("ERROR") || upper.contains("EXCEPTION")
            || upper.contains("FATAL") || upper.contains("PANIC");
        let is_warn = !is_error && upper.contains("WARN");

        if is_error {
            errors += 1;
            if root_cause.is_none() { root_cause = Some(line.chars().take(200).collect()); }
            if highlights.len() < 5 { highlights.push(line.chars().take(150).collect()); }
        } else if is_warn {
            warnings += 1;
            if highlights.len() < 5 { highlights.push(line.chars().take(150).collect()); }
        }
    }

    Some(ParsedOutput::KubeLog(KubeLogResult {
        pod,
        lines_total,
        errors,
        warnings,
        highlights,
        root_cause,
    }))
}

// ── KubeEvent ─────────────────────────────────────────────────────────────────

fn try_parse_kube_event(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    let is_events = cmd.contains("kubectl") && (cmd.contains("get events") || cmd.contains("describe"));
    if !is_events { return None; }

    let namespace = cmd.split_whitespace()
        .skip_while(|&p| p != "-n" && p != "--namespace")
        .nth(1)
        .unwrap_or("default")
        .to_string();

    let mut events: Vec<KubeEventEntry> = Vec::new();
    let mut in_events = false;

    for line in stdout.lines() {
        let line = line.trim();

        if line == "Events:" || (line.starts_with("LAST SEEN") && line.contains("REASON")) {
            in_events = true;
            continue;
        }

        if in_events && !line.is_empty() {
            // Use split_whitespace to handle variable column spacing
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 3 {
                let kind = cols.get(1).unwrap_or(&"").to_string();
                let reason = cols.get(2).unwrap_or(&"").to_string();
                // Message is everything from col 4 onwards (after TYPE REASON OBJECT)
                let message = if cols.len() > 4 {
                    cols[4..].join(" ")
                } else {
                    cols.last().unwrap_or(&"").to_string()
                };
                if !kind.is_empty() && events.len() < 20 {
                    events.push(KubeEventEntry { kind, reason, message });
                }
            }
        }
    }

    if events.is_empty() { return None; }

    Some(ParsedOutput::KubeEvent(KubeEventResult { namespace, events }))
}

// ── CI ─────────────────────────────────────────────────────────────────────────

fn try_parse_ci(cmd: &str, stdout: &str, _exit_code: i32) -> Option<ParsedOutput> {
    let is_ci = cmd.contains("gh run") || cmd.contains("gh workflow");
    if !is_ci { return None; }

    let mut pipeline_id: Option<String> = None;
    let mut stages: Vec<CiStage> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();

        // "Run #1234" or "✓ Run  completed"
        if line.starts_with("Run #") {
            pipeline_id = line["Run #".len()..].split_whitespace().next().map(|s| s.to_string());
        }

        // Stage status lines: "✓ build  (45s)" "✗ test" "● deploy"
        let (status, rest) = if line.starts_with('✓') || line.starts_with('\u{2713}') {
            ("success", &line[3..])
        } else if line.starts_with('✗') || line.starts_with('\u{2717}') {
            ("failure", &line[3..])
        } else if line.starts_with('●') || line.starts_with('\u{25cf}') || line.contains("in_progress") {
            ("in_progress", line)
        } else {
            continue;
        };

        let (name, elapsed) = if let Some(paren) = rest.rfind('(') {
            let elapsed_str = rest[paren + 1..].trim_end_matches(')').trim().to_string();
            (rest[..paren].trim().to_string(), Some(elapsed_str))
        } else {
            (rest.trim().to_string(), None)
        };

        if !name.is_empty() {
            stages.push(CiStage { name, status: status.to_string(), elapsed, detail: None });
        }
    }

    if stages.is_empty() { return None; }

    Some(ParsedOutput::Ci(CiResult { pipeline_id, stages }))
}

// ── EnvSetup ───────────────────────────────────────────────────────────────────

fn try_parse_env_setup(cmd: &str, stdout: &str, exit_code: i32) -> Option<ParsedOutput> {
    let manager = detect_env_manager(cmd)?;

    let mut packages = 0u32;
    let mut vulnerabilities = 0u32;
    let mut error: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();

        // npm: "added 847 packages"
        if line.starts_with("added ") && line.contains("package") {
            if let Some(n) = extract_num_before(line, "package") { packages = n; }
        }

        // npm: "found 3 vulnerabilities"
        if line.starts_with("found ") && line.contains("vulnerabilit") {
            if let Some(n) = extract_num_before(line, "vulnerabilit") { vulnerabilities = n; }
        }

        // pip: "Successfully installed pkg1 pkg2 ..."
        if line.starts_with("Successfully installed") {
            packages = line.split_whitespace().count().saturating_sub(2) as u32;
        }

        // cargo: count "Compiling X vN" lines
        if line.starts_with("Compiling ") { packages += 1; }

        // pnpm: "Progress: resolved N, reused N, downloaded N, added N, done"
        if line.starts_with("Progress:") && line.contains("added") {
            if let Some(n) = extract_num_before(line.rsplit("added").next().unwrap_or(""), ",") {
                packages = n;
            }
        }

        // Error
        if exit_code != 0 && error.is_none()
            && (line.starts_with("npm ERR!") || line.starts_with("error:") || line.starts_with("ERROR"))
        {
            error = Some(line.chars().take(200).collect());
        }
    }

    Some(ParsedOutput::EnvSetup(EnvSetupResult {
        manager: manager.to_string(),
        packages,
        vulnerabilities,
        error,
    }))
}

fn detect_env_manager(cmd: &str) -> Option<&'static str> {
    if cmd.contains("npm ci") { Some("npm ci") }
    else if cmd.contains("npm install") { Some("npm install") }
    else if cmd.contains("pnpm install") { Some("pnpm install") }
    else if cmd.contains("yarn install") || (cmd.contains("yarn") && !cmd.contains("cargo")) { Some("yarn") }
    else if cmd.contains("pip install") { Some("pip install") }
    else if cmd.contains("cargo build") { Some("cargo build") }
    else { None }
}

// ── Migration ─────────────────────────────────────────────────────────────────

fn try_parse_migration(cmd: &str, stdout: &str, exit_code: i32) -> Option<ParsedOutput> {
    let is_migration = cmd.contains("prisma migrate")
        || cmd.contains("diesel migration")
        || cmd.contains("alembic")
        || cmd.contains("db:migrate")
        || cmd.contains("rails db:migrate")
        || cmd.contains("migrate deploy");

    if !is_migration { return None; }

    let mut name: Option<String> = None;
    let mut applied = 0u32;
    let mut error: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();

        // "Applying migration `20230101_init`..."
        if line.to_lowercase().starts_with("applying") {
            applied += 1;
            name = extract_backtick_or_quoted(line).or(name);
        }

        // "Running migration X"
        if line.to_lowercase().starts_with("running migration") { applied += 1; }

        // "The following migration(s) have been applied:"
        if line.to_lowercase().contains("migration") && line.to_lowercase().contains("applied") {
            applied = applied.max(1);
        }

        // Error
        if exit_code != 0 && error.is_none()
            && (line.starts_with("Error") || line.contains("failed") || line.starts_with("error:"))
        {
            error = Some(line.chars().take(200).collect());
        }
    }

    Some(ParsedOutput::Migration(MigrationResult { name, applied, error }))
}

fn extract_backtick_or_quoted(s: &str) -> Option<String> {
    if let (Some(a), Some(b)) = (s.find('`'), s.rfind('`')) {
        if a < b { return Some(s[a + 1..b].to_string()); }
    }
    if let (Some(a), Some(b)) = (s.find('"'), s.rfind('"')) {
        if a < b { return Some(s[a + 1..b].to_string()); }
    }
    None
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── TestSuite ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_cargo_test_success() {
        let stdout = "running 78 tests\ntest result: ok. 78 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.05s";
        let result = try_parse_test_suite("cargo test -p decipher-tui", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::TestSuite(r)) if r.passed == 78 && r.failed == 0 && r.skipped == 2));
        assert_eq!(result.unwrap().as_test_suite().unwrap().runner, "cargo test");
    }

    #[test]
    fn parse_cargo_test_failure() {
        let stdout = "test foo::bar ... FAILED\ntest foo::baz ... ok\ntest result: FAILED. 1 passed; 1 failed; 0 ignored";
        let result = try_parse_test_suite("cargo test", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::TestSuite(r)) if r.failed == 1 && r.passed == 1));
        assert_eq!(result.unwrap().as_test_suite().unwrap().failures.len(), 1);
    }

    #[test]
    fn parse_jest_success() {
        let stdout = "Tests: 78 passed, 78 total\nTest Suites: 5 passed, 5 total";
        let result = try_parse_test_suite("npx jest --coverage", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::TestSuite(r)) if r.passed == 78 && r.runner == "jest"));
    }

    #[test]
    fn parse_pytest_failure() {
        let stdout = "collected 80 items\n===== 2 failed, 78 passed in 5.21s =====";
        let result = try_parse_test_suite("pytest tests/", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::TestSuite(r)) if r.failed == 2 && r.passed == 78));
    }

    // ── DockerBuild ────────────────────────────────────────────────────────────

    #[test]
    fn parse_docker_build_success() {
        let stdout = "Step 1/12 : FROM node:22\nStep 12/12 : CMD [\"node\", \"server.js\"]\nSuccessfully tagged myapp:latest\n";
        let result = try_parse_docker_build("docker build -t myapp:latest .", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::DockerBuild(r)) if r.steps_total == 12 && r.image == "myapp:latest"));
    }

    #[test]
    fn parse_docker_build_failure() {
        let stdout = "Step 1/12 : FROM node:22\nStep 9/12 : RUN npm test\nnpm ERR! Test failed.\nERROR: failed to solve: process npm test returned exit code 1";
        let result = try_parse_docker_build("docker build -t myapp .", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::DockerBuild(r)) if r.error.is_some()));
    }

    // ── DockerRun ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_docker_run_detached() {
        let stdout = "abc1234567890def";
        let result = try_parse_docker_run("docker run -d -p 3000:3000 myapp", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::DockerRun(r)) if !r.container.is_empty()));
    }

    #[test]
    fn parse_docker_run_error() {
        let stdout = "Error response from daemon: No such image: badimage:latest";
        let result = try_parse_docker_run("docker run badimage:latest", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::DockerRun(r)) if r.error.is_some()));
    }

    // ── Compose ────────────────────────────────────────────────────────────────

    #[test]
    fn parse_compose_up() {
        let stdout = "Container myapp-web-1  Created\nContainer myapp-db-1  Started";
        let result = try_parse_compose("docker compose up -d", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::Compose(r)) if r.services.len() == 2));
    }

    #[test]
    fn parse_compose_ps() {
        let stdout = "NAME         IMAGE         COMMAND   SERVICE   CREATED   STATUS    PORTS\nmyapp-web-1  nginx:latest  nginx     web       1m        Up 1m     80/tcp";
        let result = try_parse_compose("docker compose ps", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::Compose(r)) if !r.services.is_empty()));
    }

    // ── Git ────────────────────────────────────────────────────────────────────

    #[test]
    fn parse_git_commit_success() {
        let stdout = "[main abc1234] feat: fix Docker build\n 3 files changed, 42 insertions(+), 18 deletions(-)";
        let result = try_parse_git("git commit -m 'feat: fix Docker build'", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::GitOp(r)) if r.files_changed == 3 && r.additions == 42 && r.deletions == 18));
        let g = result.unwrap();
        let git = g.as_git_op().unwrap();
        assert_eq!(git.op, "commit");
        assert!(git.commit_msg.as_deref().unwrap_or("").contains("fix Docker build"));
    }

    #[test]
    fn parse_git_merge_conflict() {
        let stdout = "Auto-merging src/main.rs\nCONFLICT (content): Merge conflict in src/main.rs\nAutomatic merge failed; fix conflicts and then commit the result.";
        let result = try_parse_git("git merge feature-branch", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::GitOp(r)) if !r.conflicts.is_empty()));
    }

    // ── Lint ───────────────────────────────────────────────────────────────────

    #[test]
    fn parse_clippy_no_warnings() {
        let stdout = "    Checking decipher-tui v0.1.0\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.23s";
        let result = try_parse_lint("cargo clippy --all-targets", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::Lint(r)) if r.warnings == 0 && r.errors == 0));
    }

    #[test]
    fn parse_clippy_with_warnings() {
        let stdout = "warning: unused variable `x`\n  --> src/main.rs:10:5\nwarning: 1 warnings emitted";
        let result = try_parse_lint("cargo clippy", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::Lint(r)) if r.warnings >= 1));
    }

    // ── KubePod ────────────────────────────────────────────────────────────────

    #[test]
    fn parse_kubectl_get_pods() {
        let stdout = "NAME                     READY   STATUS    RESTARTS   AGE\nweb-6d9f4-xk2p8          1/1     Running   0          5m\ndb-5c8b7-j9q3r           1/1     Running   2          10m";
        let result = try_parse_kube_pod("kubectl get pods", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::KubePod(r)) if r.pods.len() == 2 && r.pods[1].restarts == 2));
    }

    #[test]
    fn parse_kubectl_get_pods_crashloop() {
        let stdout = "NAME             READY   STATUS             RESTARTS   AGE\nweb-abc-xyz      0/1     CrashLoopBackOff   5          3m";
        let result = try_parse_kube_pod("kubectl get pods -n production", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::KubePod(r)) if r.pods[0].status == "CrashLoopBackOff" && r.pods[0].restarts == 5));
    }

    // ── KubeLog ────────────────────────────────────────────────────────────────

    #[test]
    fn parse_kubectl_logs_with_errors() {
        let stdout = "2024-01-01T10:00:00Z INFO server started\n2024-01-01T10:01:00Z ERROR connection refused to database\n2024-01-01T10:01:01Z FATAL cannot continue without DB";
        let result = try_parse_kube_log("kubectl logs web-abc-xyz", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::KubeLog(r)) if r.errors >= 2 && r.root_cause.is_some()));
    }

    #[test]
    fn parse_kubectl_logs_clean() {
        let stdout = "2024-01-01T10:00:00Z INFO starting server\n2024-01-01T10:00:01Z INFO listening on :8080";
        let result = try_parse_kube_log("kubectl logs my-pod -n default", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::KubeLog(r)) if r.errors == 0 && r.lines_total == 2));
    }

    // ── KubeEvent ─────────────────────────────────────────────────────────────

    #[test]
    fn parse_kubectl_get_events() {
        let stdout = "LAST SEEN   TYPE      REASON     OBJECT            MESSAGE\n5m          Normal    Pulled     pod/web-xyz       Pulled image\n3m          Warning   BackOff    pod/web-xyz       Back-off restarting failed container";
        let result = try_parse_kube_event("kubectl get events", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::KubeEvent(r)) if r.events.len() == 2));
    }

    #[test]
    fn parse_kubectl_describe_events() {
        let stdout = "Name: web-pod\nEvents:\n  5m    Normal  Scheduled  pod/web  Successfully assigned\n  4m    Warning OOMKilled   pod/web  Out of memory";
        let result = try_parse_kube_event("kubectl describe pod web-pod", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::KubeEvent(r)) if !r.events.is_empty()));
    }

    // ── CI ─────────────────────────────────────────────────────────────────────

    #[test]
    fn parse_gh_run_success() {
        let stdout = "Run #1234\n✓ build  (45s)\n✓ test   (120s)\n✓ deploy (30s)";
        let result = try_parse_ci("gh run view 1234", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::Ci(r)) if r.stages.len() == 3 && r.pipeline_id.as_deref() == Some("1234")));
    }

    #[test]
    fn parse_gh_run_failure() {
        let stdout = "Run #5678\n✓ build  (40s)\n✗ test\n○ deploy";
        let result = try_parse_ci("gh run watch 5678", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::Ci(r)) if r.stages.iter().any(|s| s.status == "failure")));
    }

    // ── EnvSetup ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_npm_ci_success() {
        let stdout = "added 847 packages, and audited 848 packages in 12s\nfound 0 vulnerabilities";
        let result = try_parse_env_setup("npm ci", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::EnvSetup(r)) if r.packages == 847 && r.vulnerabilities == 0));
    }

    #[test]
    fn parse_npm_install_with_vuln() {
        let stdout = "added 120 packages from 80 contributors\nfound 3 vulnerabilities (1 high, 2 moderate)";
        let result = try_parse_env_setup("npm install", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::EnvSetup(r)) if r.packages == 120 && r.vulnerabilities == 3));
    }

    // ── Migration ─────────────────────────────────────────────────────────────

    #[test]
    fn parse_prisma_migrate_success() {
        let stdout = "Applying migration `20230101_000001_init`...\nThe following migration(s) have been applied:\n  migrations/20230101_000001_init";
        let result = try_parse_migration("prisma migrate deploy", stdout, 0);
        assert!(matches!(&result, Some(ParsedOutput::Migration(r)) if r.applied >= 1 && r.name.is_some()));
    }

    #[test]
    fn parse_migration_error() {
        let stdout = "Applying migration `20230202_add_users`...\nError: migration failed: column already exists";
        let result = try_parse_migration("prisma migrate deploy", stdout, 1);
        assert!(matches!(&result, Some(ParsedOutput::Migration(r)) if r.error.is_some()));
    }

    // ── Generic fallback ──────────────────────────────────────────────────────

    #[test]
    fn unknown_cmd_returns_generic() {
        let result = parse_output("ls -la", "total 42\n-rw-r--r-- 1 user group 100 Jan 1 file.txt", 0);
        assert!(matches!(result, ParsedOutput::Generic));
    }

    #[test]
    fn dispatcher_priority_test_suite_before_env_setup() {
        // "cargo build" before "cargo test" should be EnvSetup, not TestSuite
        let stdout = "   Compiling myapp v0.1.0\n    Finished `release` profile target(s) in 3.5s";
        let result = parse_output("cargo build --release", stdout, 0);
        assert!(matches!(result, ParsedOutput::EnvSetup(_)), "Expected EnvSetup, got {result:?}");
    }
}

// ── Downcast helpers for test assertions ──────────────────────────────────────

impl ParsedOutput {
    pub fn as_test_suite(&self) -> Option<&TestSuiteResult> {
        if let Self::TestSuite(r) = self { Some(r) } else { None }
    }
    pub fn as_git_op(&self) -> Option<&GitResult> {
        if let Self::GitOp(r) = self { Some(r) } else { None }
    }
}
