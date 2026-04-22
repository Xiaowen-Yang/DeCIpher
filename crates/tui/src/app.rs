//! Application state machine.
//!
//! Manages the lifecycle of the TUI: input mode, chat history,
//! popup state, and communication with the Node.js agent.

use decipher_protocol::{ClientMessage, CommandInfo, ImageData, ServerMessage};
use crate::cell::AgentMessageCell;
use crate::chat::ChatWidget;
use crate::terminal_detect::TerminalCaps;

/// Agent processing phase — shown in the live activity bar.
///
/// Labels match the visual spec vocabulary exactly.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentPhase {
    Idle,
    /// Mission just received — LLM is parsing intent and building plan.
    Planning,
    /// LLM is reasoning between tool calls.
    Thinking,
    /// Generic tool execution (fallback).
    Executing,
    /// write_file / apply_patch in progress.
    ApplyingEdits,
    /// exec_command / test runner in progress.
    RunningChecks,
    /// Post-tool LLM review pass.
    Verifying,
    /// Waiting for user approval before proceeding.
    WaitingForApproval,

    // ── Phase D: context-aware exec phases ─────────────────────────────────
    /// docker build in progress.
    BuildingImage,
    /// docker run / docker compose up in progress.
    StartingContainer,
    /// kubectl apply / rollout in progress.
    Deploying,
    /// kubectl logs / streaming logs.
    TailingLogs,
    /// kubectl get pods / watch pods.
    WatchingPods,
    /// npm ci / pip install / cargo build.
    InstallingDeps,
    /// cargo clippy / eslint / ruff.
    Linting,
    /// cargo test / npm test / pytest / go test.
    RunningTests,
    /// git commit / push / merge.
    GitOp,
    /// prisma migrate / diesel migration / alembic.
    Migrating,
}

impl AgentPhase {
    /// Display label as shown in the live activity bar.
    /// Must match `docs/plans/2026-04-21-interaction-surface-visual-spec.md`.
    pub fn label(&self) -> &str {
        match self {
            Self::Idle => "",
            Self::Planning => "Understanding mission",
            Self::Thinking => "Working",
            Self::Executing => "Working",
            Self::ApplyingEdits => "Applying edits",
            Self::RunningChecks => "Running checks",
            Self::Verifying => "Reviewing changes",
            Self::WaitingForApproval => "Waiting for approval",
            Self::BuildingImage => "Building image",
            Self::StartingContainer => "Starting container",
            Self::Deploying => "Deploying",
            Self::TailingLogs => "Tailing logs",
            Self::WatchingPods => "Watching pods",
            Self::InstallingDeps => "Installing deps",
            Self::Linting => "Linting",
            Self::RunningTests => "Running tests",
            Self::GitOp => "Git operation",
            Self::Migrating => "Migrating",
        }
    }

    /// True when this phase should display the animated spinner.
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Detect a specific phase from an exec_command's `cmd` string.
    /// Returns None if no specific phase matches (caller falls back to RunningChecks).
    pub fn from_exec_cmd(cmd: &str) -> Option<Self> {
        if cmd.contains("docker build") { return Some(Self::BuildingImage); }
        if cmd.contains("docker run") || cmd.contains("docker compose up") || cmd.contains("docker-compose up") {
            return Some(Self::StartingContainer);
        }
        if cmd.contains("cargo test") || cmd.contains("npm test") || cmd.contains("pytest")
            || cmd.contains("go test") || cmd.contains("npx jest") || cmd.contains("vitest") {
            return Some(Self::RunningTests);
        }
        if cmd.contains("cargo clippy") || cmd.contains("eslint") || cmd.contains("ruff")
            || (cmd.contains("prettier") && cmd.contains("--check")) {
            return Some(Self::Linting);
        }
        if cmd.contains("npm ci") || cmd.contains("npm install") || cmd.contains("pip install")
            || cmd.contains("pnpm install") || cmd.contains("yarn install") || cmd.contains("cargo build") {
            return Some(Self::InstallingDeps);
        }
        if cmd.contains("git commit") || cmd.contains("git push") || cmd.contains("git merge")
            || cmd.contains("git pull") || cmd.contains("git rebase") {
            return Some(Self::GitOp);
        }
        if cmd.contains("kubectl apply") || cmd.contains("kubectl rollout") || cmd.contains("kubectl create") {
            return Some(Self::Deploying);
        }
        if cmd.contains("kubectl logs") { return Some(Self::TailingLogs); }
        if cmd.contains("kubectl get pod") || cmd.contains("kubectl watch") {
            return Some(Self::WatchingPods);
        }
        if cmd.contains("prisma migrate") || cmd.contains("diesel migration") || cmd.contains("alembic")
            || cmd.contains("db:migrate") {
            return Some(Self::Migrating);
        }
        None
    }
}

/// Current input mode of the TUI.
#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    CommandPopup,
    ApprovalPending,
    HistorySearch,
    Pager,
    FileSearch,
}

/// Top-level application state.
pub struct App {
    /// ChatWidget manages typed cell history (replaces old Vec<ChatEntry>).
    pub chat: ChatWidget,
    pub input: String,
    pub cursor: usize,
    pub mode: InputMode,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub saved_input: String,
    pub scroll_offset: u16,
    pub banner: Option<BannerInfo>,
    pub commands: Vec<CommandInfo>,
    pub popup_filter: String,
    pub popup_index: usize,
    pub agent_busy: bool,
    pub spinner_label: Option<String>,
    pub spinner_frame: usize,
    pub spinner_started: Option<std::time::Instant>,
    /// Current agent phase for status indicator display.
    pub agent_phase: AgentPhase,
    /// Detail text for the status indicator (e.g., tool name, file path).
    pub agent_phase_detail: Option<String>,
    /// Current turn number in the agent loop.
    pub agent_turn: u32,
    /// Maximum turns allowed.
    pub agent_max_turns: u32,
    /// Mission start time (for elapsed display).
    pub mission_started: Option<std::time::Instant>,
    /// Approval action detail (tool name + reasoning) for viewport display.
    pub pending_approval_action: Option<String>,
    /// Selected option in the approval popup (0=approve, 1=always, 2=deny).
    pub approval_index: usize,
    pub should_quit: bool,
    pub last_submitted: String,
    pub kill_buffer: String,
    pub last_ctrl_c: Option<std::time::Instant>,
    /// Whether the terminal window is currently focused.
    pub terminal_focused: bool,
    /// Ctrl+R history search query.
    pub search_query: String,
    /// Index into input_history for current search match.
    pub search_match_index: Option<usize>,
    /// Saved input before entering history search.
    pub search_saved_input: String,
    /// Queued message to submit when agent finishes (Tab while busy).
    pub queued_message: Option<ClientMessage>,
    /// Whether to show the shortcut overlay.
    pub show_shortcuts: bool,
    /// Pager scroll offset (for Ctrl+T transcript view).
    pub pager_scroll: usize,
    /// File search query (@ popup).
    pub file_search_query: String,
    /// File search results.
    pub file_search_results: Vec<crate::file_search::FileResult>,
    /// File search selection index.
    pub file_search_index: usize,
    /// Position of @ in input (for replacement).
    pub file_search_at_pos: usize,
    /// Images staged for the next submission (via Ctrl+V paste).
    pub pending_images: Vec<ImageData>,
    /// Running total of images pasted this session (used for [Image #N] numbering).
    pub session_image_count: usize,
    /// Whether to auto-approve all actions for this session.
    pub always_approve: bool,
    /// Cumulative token usage for this session.
    pub total_tokens: u64,
    /// Tokens from most recent API call.
    pub last_tokens: u64,
    /// Prompt tokens from most recent API call (= current context window usage).
    pub context_tokens: u64,
    /// Model context window size (from token_usage messages).
    pub context_window: u64,
    /// Terminal capabilities (detected once at startup).
    pub terminal_caps: TerminalCaps,
    /// Session log entries (JSONL recording).
    pub session_log: Vec<String>,
    /// Whether session logging is enabled (via DECIPHER_TUI_RECORD_SESSION env).
    pub session_logging: bool,
    /// Cached pager transcript lines (plain strings for crossterm rendering).
    pub pager_cache: Vec<String>,
    /// Cache key when pager_cache was last built: (committed_len, revision).
    pub pager_cache_key: (usize, u64),
    /// Terminal width when pager_cache was last built.
    pub pager_cache_width: u16,
}

#[derive(Debug, Clone)]
pub struct BannerInfo {
    pub version: String,
    pub provider: String,
    pub model: String,
    pub directory: String,
    pub api_key_set: bool,
}

impl App {
    pub fn new() -> Self {
        let (_, cols) = crossterm::terminal::size().unwrap_or((24, 80));
        Self {
            chat: ChatWidget::new(cols),
            input: String::new(),
            cursor: 0,
            mode: InputMode::Normal,
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            scroll_offset: 0,
            banner: None,
            commands: Vec::new(),
            popup_filter: String::new(),
            popup_index: 0,
            agent_busy: false,
            spinner_label: None,
            spinner_frame: 0,
            agent_phase: AgentPhase::Idle,
            agent_phase_detail: None,
            agent_turn: 0,
            agent_max_turns: 20,
            mission_started: None,
            pending_approval_action: None,
            approval_index: 0,
            spinner_started: None,
            should_quit: false,
            last_submitted: String::new(),
            kill_buffer: String::new(),
            last_ctrl_c: None,
            terminal_focused: true,
            search_query: String::new(),
            search_match_index: None,
            search_saved_input: String::new(),
            queued_message: None,
            show_shortcuts: false,
            pager_scroll: 0,
            file_search_query: String::new(),
            file_search_results: Vec::new(),
            file_search_index: 0,
            file_search_at_pos: 0,
            pending_images: Vec::new(),
            session_image_count: 0,
            always_approve: false,
            total_tokens: 0,
            last_tokens: 0,
            context_tokens: 0,
            context_window: 0,
            terminal_caps: crate::terminal_detect::detect(),
            session_log: Vec::new(),
            session_logging: std::env::var("DECIPHER_TUI_RECORD_SESSION").is_ok(),
            pager_cache: Vec::new(),
            pager_cache_key: (0, 0),
            pager_cache_width: 0,
        }
    }

    /// Update App state from a server message (spinner, mode, agent_busy, etc.).
    ///
    /// Cell creation and scrollback rendering are handled by ChatWidget —
    /// this method only manages the UI state machine.
    pub fn handle_server_message(&mut self, msg: ServerMessage) {
        // Session logging
        if self.session_logging {
            if let Ok(json) = serde_json::to_string(&msg) {
                self.session_log.push(json);
            }
        }
        match msg {
            ServerMessage::Banner { version, provider, model, directory, api_key_set } => {
                self.banner = Some(BannerInfo { version, provider, model, directory, api_key_set });
            }
            ServerMessage::Mission { .. } => {
                self.agent_busy = true;
                self.agent_phase = AgentPhase::Planning;
                self.agent_phase_detail = None;
                self.agent_turn = 0;
                self.mission_started = Some(std::time::Instant::now());
            }
            ServerMessage::Clarification { .. } => {
                self.agent_busy = false;
                self.agent_phase = AgentPhase::Idle;
            }
            ServerMessage::ApprovalRequest { ref action, .. } => {
                self.mode = InputMode::ApprovalPending;
                self.approval_index = 0; // Default to "Approve"
                self.agent_phase = AgentPhase::WaitingForApproval;
                self.pending_approval_action = action.as_ref().map(|a| {
                    if let Some(ref reason) = a.reasoning {
                        format!("{} — {}", a.tool, reason.chars().take(60).collect::<String>())
                    } else {
                        a.tool.clone()
                    }
                });
            }
            ServerMessage::ToolStart { ref tool, ref args, .. } => {
                self.agent_busy = true;
                self.agent_phase = match tool.as_str() {
                    "write_file" | "apply_patch" => AgentPhase::ApplyingEdits,
                    "exec_command" => {
                        let cmd = args
                            .as_ref()
                            .and_then(|v| v.get("cmd"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        AgentPhase::from_exec_cmd(cmd).unwrap_or(AgentPhase::RunningChecks)
                    }
                    "kubectl_exec" | "kubectl_apply" | "kubectl_delete" => AgentPhase::RunningChecks,
                    "read_file" | "list_files" | "search" | "grep_search" | "file_search" => AgentPhase::Thinking,
                    _ => AgentPhase::Executing,
                };
                self.agent_phase_detail = Some(tool.clone());
                if self.spinner_label.is_none() {
                    self.spinner_started = Some(std::time::Instant::now());
                }
                self.spinner_label = Some(format!("→ {tool}"));
            }
            ServerMessage::ToolResult { .. } => {
                self.agent_phase = AgentPhase::Thinking;
                self.agent_phase_detail = None;
            }
            ServerMessage::AgentMessage { .. } => {
                self.agent_busy = true;
            }
            ServerMessage::AgentMessageDelta { .. } => {
                self.agent_busy = true;
                if self.spinner_label.is_none() {
                    self.spinner_started = Some(std::time::Instant::now());
                    self.spinner_label = Some("Working".to_string());
                }
                if matches!(self.agent_phase, AgentPhase::Idle | AgentPhase::Thinking | AgentPhase::Planning) {
                    self.agent_phase = AgentPhase::Thinking;
                }
            }
            ServerMessage::ExecOutputDelta { ref delta } => {
                // Show latest output line in the activity bar detail.
                let trimmed = delta.trim();
                if !trimmed.is_empty() {
                    self.agent_phase_detail = Some(
                        trimmed.chars().take(60).collect(),
                    );
                }
            }
            ServerMessage::AgentStatus { turn, max_turns, tool_name, phase, .. } => {
                self.agent_turn = turn;
                self.agent_max_turns = max_turns;
                if let Some(name) = tool_name {
                    self.agent_phase_detail = Some(name);
                }
                // AgentStatus arriving means the agent loop IS running —
                // activate busy/spinner state if not already active.
                if !self.agent_busy {
                    self.agent_busy = true;
                    self.agent_phase = AgentPhase::Thinking;
                    self.mission_started = Some(std::time::Instant::now());
                }
                if self.spinner_label.is_none() {
                    self.spinner_started = Some(std::time::Instant::now());
                }
                self.spinner_label = Some(phase);
            }
            ServerMessage::MissionComplete { .. } => {
                self.agent_busy = false;
                self.spinner_label = None;
                self.spinner_started = None;
                self.agent_phase = AgentPhase::Idle;
                self.agent_phase_detail = None;
                self.mission_started = None;
            }
            ServerMessage::Error { .. } => {
                self.agent_busy = false;
                self.spinner_label = None;
                self.spinner_started = None;
                self.agent_phase = AgentPhase::Idle;
                self.agent_phase_detail = None;
            }
            ServerMessage::Spinner { label, done } => {
                if done {
                    self.spinner_label = None;
                    self.spinner_started = None;
                } else {
                    if self.spinner_label.is_none() {
                        self.spinner_started = Some(std::time::Instant::now());
                    }
                    self.spinner_label = Some(label);
                }
            }
            ServerMessage::CommandList { commands } => { self.commands = commands; }
            ServerMessage::ToolCall { .. } => {}
            ServerMessage::ToolCallResult { .. } => {}
            ServerMessage::FilesModified { .. } => {}  // handled in chat.rs
            ServerMessage::TokenUsage { prompt_tokens, completion_tokens, total_tokens, context_window } => {
                self.last_tokens = total_tokens;
                self.total_tokens += total_tokens; // accumulate across turns
                self.context_tokens = prompt_tokens;
                if let Some(cw) = context_window {
                    self.context_window = cw;
                }
                let _ = completion_tokens;
            }
            ServerMessage::SubagentStart { .. } => {
                // Subagent events are rendered in chat.rs; no state change needed here.
            }
            ServerMessage::SubagentComplete { .. } => {
                // Subagent events are rendered in chat.rs; no state change needed here.
            }
        }
        self.scroll_offset = 0;
    }

    pub fn submit_input(&mut self) -> Option<ClientMessage> {
        let text = self.input.trim().to_string();
        if text.is_empty() { return None; }
        self.input_history.push(text.clone());
        self.history_index = None;
        self.last_submitted = text.clone();
        let images = std::mem::take(&mut self.pending_images);
        let image_refs = images.iter()
            .map(|img| img.path.clone().unwrap_or_else(|| img.mime.clone()))
            .collect();

        // Push UserCell to ChatWidget for pager transcript
        self.chat.committed_cells.push(Box::new(
            crate::cell::UserCell::new(text.clone(), image_refs)
        ));

        self.input.clear();
        self.cursor = 0;

        // Handle local slash commands
        if text == "/clear" {
            self.chat.committed_cells.clear();
            return None;
        }
        if text == "/copy" {
            self.copy_last_response();
            return None;
        }

        if text.starts_with('/') {
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            let name = parts[0].to_string();
            let args = parts.get(1).map(|s| s.to_string());
            return Some(ClientMessage::SlashCommand { name, args });
        }
        Some(ClientMessage::UserInput { text, images })
    }

    /// Copy the last agent response to the clipboard.
    fn copy_last_response(&self) {
        let last = self.chat.committed_cells.iter().rev()
            .find_map(|cell| cell.as_any().downcast_ref::<AgentMessageCell>());
        if let Some(agent_cell) = last {
            // Use raw text directly — no need to re-extract from rendered lines.
            let text = agent_cell.raw_text.clone();
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(&text);
            }
        }
    }

    pub fn respond_approval(&mut self, approved: bool) -> ClientMessage {
        self.mode = InputMode::Normal;
        self.pending_approval_action = None;
        self.chat.resolve_active_approval(approved);
        ClientMessage::ApprovalResponse { approved }
    }

    pub fn filtered_commands(&self) -> Vec<&CommandInfo> {
        let filter = self.popup_filter.to_lowercase();
        self.commands.iter().filter(|c| {
            if filter.is_empty() { return true; }
            let name = c.name.to_lowercase();
            let mut fi = filter.chars().peekable();
            for ch in name.chars() {
                if fi.peek() == Some(&ch) { fi.next(); }
            }
            fi.peek().is_none()
        }).collect()
    }

    pub fn word_left(&mut self) {
        if self.cursor == 0 { return; }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor;
        while pos > 0 && bytes[pos - 1] == b' ' { pos -= 1; }
        while pos > 0 && bytes[pos - 1] != b' ' { pos -= 1; }
        self.cursor = pos;
    }

    pub fn word_right(&mut self) {
        let len = self.input.len();
        if self.cursor >= len { return; }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor;
        while pos < len && bytes[pos] != b' ' { pos += 1; }
        while pos < len && bytes[pos] == b' ' { pos += 1; }
        self.cursor = pos;
    }

    pub fn kill_to_end(&mut self) {
        if self.cursor < self.input.len() {
            self.kill_buffer = self.input[self.cursor..].to_string();
            self.input.truncate(self.cursor);
        }
    }

    pub fn kill_to_start(&mut self) {
        if self.cursor > 0 {
            self.kill_buffer = self.input[..self.cursor].to_string();
            self.input = self.input[self.cursor..].to_string();
            self.cursor = 0;
        }
    }

    pub fn kill_word_backward(&mut self) {
        if self.cursor == 0 { return; }
        let bytes = self.input.as_bytes();
        let mut end = self.cursor;
        while end > 0 && bytes[end - 1] == b' ' { end -= 1; }
        let mut pos = end;
        while pos > 0 && bytes[pos - 1] != b' ' { pos -= 1; }
        if end == self.cursor && pos == self.cursor { return; }
        self.kill_buffer = self.input[pos..self.cursor].to_string();
        self.input = format!("{}{}", &self.input[..pos], &self.input[self.cursor..]);
        self.cursor = pos;
    }

    pub fn kill_word_forward(&mut self) {
        let len = self.input.len();
        if self.cursor >= len { return; }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor;
        while pos < len && bytes[pos] == b' ' { pos += 1; }
        while pos < len && bytes[pos] != b' ' { pos += 1; }
        if pos == self.cursor { return; }
        self.kill_buffer = self.input[self.cursor..pos].to_string();
        self.input = format!("{}{}", &self.input[..self.cursor], &self.input[pos..]);
    }

    pub fn yank(&mut self) {
        if !self.kill_buffer.is_empty() {
            let yanked = self.kill_buffer.clone();
            self.input.insert_str(self.cursor, &yanked);
            self.cursor += yanked.len();
        }
    }

    /// Enter Ctrl+R history search mode.
    pub fn enter_history_search(&mut self) {
        self.search_saved_input = self.input.clone();
        self.search_query.clear();
        self.search_match_index = None;
        self.mode = InputMode::HistorySearch;
    }

    /// Search for the next older match in history.
    pub fn search_history_older(&mut self) {
        let start = self.search_match_index.map(|i| i.wrapping_sub(1)).unwrap_or(self.input_history.len().wrapping_sub(1));
        self.search_match_from(start, true);
    }

    /// Search for the next newer match in history.
    pub fn search_history_newer(&mut self) {
        let start = self.search_match_index.map(|i| i + 1).unwrap_or(0);
        self.search_match_from(start, false);
    }

    fn search_match_from(&mut self, start: usize, reverse: bool) {
        if self.input_history.is_empty() || self.search_query.is_empty() { return; }
        let len = self.input_history.len();
        let query = self.search_query.to_lowercase();
        for step in 0..len {
            let idx = if reverse {
                (start.wrapping_sub(step)) % len
            } else {
                (start + step) % len
            };
            if idx >= len { continue; }
            if self.input_history[idx].to_lowercase().contains(&query) {
                self.search_match_index = Some(idx);
                self.input = self.input_history[idx].clone();
                self.cursor = self.input.len();
                return;
            }
        }
        self.search_match_index = None;
    }

    /// Accept current search result and return to Normal mode.
    pub fn accept_history_search(&mut self) {
        self.mode = InputMode::Normal;
        self.search_query.clear();
    }

    /// Cancel search, restore original input.
    pub fn cancel_history_search(&mut self) {
        self.mode = InputMode::Normal;
        self.input = self.search_saved_input.clone();
        self.cursor = self.input.len();
        self.search_query.clear();
        self.search_match_index = None;
    }

    pub fn navigate_history(&mut self, up: bool) {
        if self.input_history.is_empty() { return; }
        match self.history_index {
            None => {
                if up {
                    self.saved_input = self.input.clone();
                    self.history_index = Some(self.input_history.len() - 1);
                    self.input = self.input_history.last().cloned().unwrap_or_default();
                    self.cursor = self.input.len();
                }
            }
            Some(idx) => {
                if up && idx > 0 {
                    self.history_index = Some(idx - 1);
                    self.input = self.input_history[idx - 1].clone();
                    self.cursor = self.input.len();
                } else if !up {
                    if idx + 1 < self.input_history.len() {
                        self.history_index = Some(idx + 1);
                        self.input = self.input_history[idx + 1].clone();
                        self.cursor = self.input.len();
                    } else {
                        self.history_index = None;
                        self.input = self.saved_input.clone();
                        self.cursor = self.input.len();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_input_preserves_image_refs_in_user_cell_and_message() {
        let mut app = App::new();
        app.input = "Please inspect this screenshot [Image #1]".to_string();
        app.cursor = app.input.len();
        app.pending_images.push(ImageData {
            data: "ignored-base64".to_string(),
            path: Some("/tmp/decipher-clipboard/test.png".to_string()),
            mime: "image/png".to_string(),
        });

        let msg = app.submit_input().expect("user input message");

        match msg {
            ClientMessage::UserInput { text, images } => {
                assert_eq!(text, "Please inspect this screenshot [Image #1]");
                assert_eq!(images.len(), 1);
            }
            other => panic!("expected user_input, got {other:?}"),
        }

        let user_cell = app.chat.committed_cells
            .last()
            .and_then(|cell| cell.as_any().downcast_ref::<crate::cell::UserCell>())
            .expect("user cell");
        assert_eq!(user_cell.images.len(), 1);
    }
}
