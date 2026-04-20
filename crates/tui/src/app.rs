//! Application state machine.
//!
//! Manages the lifecycle of the TUI: input mode, chat history,
//! popup state, and communication with the Node.js agent.

use decipher_protocol::{ClientMessage, CommandInfo, ServerMessage};
use crate::streaming::StreamState;
use crate::terminal_detect::TerminalCaps;

/// A single entry in the chat history.
#[derive(Debug, Clone)]
pub struct ChatEntry {
    pub kind: ChatEntryKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatEntryKind {
    UserInput,
    Mission,
    Clarification,
    ToolStart,
    ToolResult,
    AgentMessage,
    MissionComplete,
    Error,
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
    pub history: Vec<ChatEntry>,
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
    pub should_quit: bool,
    pub last_submitted: String,
    pub kill_buffer: String,
    pub last_ctrl_c: Option<std::time::Instant>,
    /// Streaming pipeline state (newline-gated buffering + adaptive chunking).
    pub stream: StreamState,
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
    /// Whether to auto-approve all actions for this session.
    pub always_approve: bool,
    /// Cumulative token usage for this session.
    pub total_tokens: u64,
    /// Tokens from most recent API call.
    pub last_tokens: u64,
    /// Terminal capabilities (detected once at startup).
    pub terminal_caps: TerminalCaps,
    /// Session log entries (JSONL recording).
    pub session_log: Vec<String>,
    /// Whether session logging is enabled (via DECIPHER_TUI_RECORD_SESSION env).
    pub session_logging: bool,
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
        Self {
            history: Vec::new(),
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
            spinner_started: None,
            should_quit: false,
            last_submitted: String::new(),
            kill_buffer: String::new(),
            last_ctrl_c: None,
            stream: StreamState::new(),
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
            always_approve: false,
            total_tokens: 0,
            last_tokens: 0,
            terminal_caps: crate::terminal_detect::detect(),
            session_log: Vec::new(),
            session_logging: std::env::var("DECIPHER_TUI_RECORD_SESSION").is_ok(),
        }
    }

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
            ServerMessage::Mission { understood, target, target_type: _, steps } => {
                let mut text = format!("Understood: {understood}");
                if let Some(t) = target { text.push_str(&format!("\nTarget: {t}")); }
                if !steps.is_empty() {
                    text.push_str("\n\nPlan:");
                    for (i, step) in steps.iter().enumerate() {
                        text.push_str(&format!("\n  {}. {step}", i + 1));
                    }
                }
                self.history.push(ChatEntry { kind: ChatEntryKind::Mission, text });
                self.agent_busy = true;
            }
            ServerMessage::Clarification { question } => {
                self.history.push(ChatEntry { kind: ChatEntryKind::Clarification, text: question });
                self.agent_busy = false;
            }
            ServerMessage::ApprovalRequest { .. } => {
                self.mode = InputMode::ApprovalPending;
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::AgentMessage,
                    text: "DeCIpher needs permission to proceed. [Y/n]".into(),
                });
            }
            ServerMessage::ToolStart { tool, reasoning } => {
                self.history.push(ChatEntry { kind: ChatEntryKind::ToolStart, text: format!("{tool} — {reasoning}") });
            }
            ServerMessage::ToolResult { tool, success, summary, elapsed_ms } => {
                let icon = if success { "✓" } else { "✗" };
                let secs = elapsed_ms as f64 / 1000.0;
                self.history.push(ChatEntry { kind: ChatEntryKind::ToolResult, text: format!("{icon} {tool} — {summary} ({secs:.1}s)") });
            }
            ServerMessage::AgentMessage { text } => {
                self.history.push(ChatEntry { kind: ChatEntryKind::AgentMessage, text });
            }
            ServerMessage::MissionComplete { outcome, summary, turns, elapsed_ms, .. } => {
                let secs = elapsed_ms as f64 / 1000.0;
                self.history.push(ChatEntry { kind: ChatEntryKind::MissionComplete, text: format!("{outcome} ({secs:.1}s, {turns} turns)\n{summary}") });
                self.agent_busy = false;
                self.spinner_label = None;
                self.spinner_started = None;
            }
            ServerMessage::Error { message } => {
                self.history.push(ChatEntry { kind: ChatEntryKind::Error, text: message });
                self.agent_busy = false;
                self.spinner_label = None;
                self.spinner_started = None;
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
            ServerMessage::AgentMessageDelta { .. } => {}
            ServerMessage::CommandList { commands } => { self.commands = commands; }
            ServerMessage::TokenUsage { total_tokens, .. } => {
                self.last_tokens = total_tokens;
                self.total_tokens += total_tokens;
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
        self.history.push(ChatEntry { kind: ChatEntryKind::UserInput, text: text.clone() });
        self.input.clear();
        self.cursor = 0;

        // Handle local slash commands
        if text == "/clear" {
            self.history.clear();
            return None; // Handled locally, no message to server
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
        Some(ClientMessage::UserInput { text, images: vec![] })
    }

    /// Copy the last agent response to the clipboard.
    fn copy_last_response(&self) {
        let last = self.history.iter().rev()
            .find(|e| e.kind == ChatEntryKind::AgentMessage);
        if let Some(entry) = last {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(&entry.text);
            }
        }
    }

    pub fn respond_approval(&mut self, approved: bool) -> ClientMessage {
        self.mode = InputMode::Normal;
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
        // No match found
        self.search_match_index = None;
    }

    /// Accept current search result and return to Normal mode.
    pub fn accept_history_search(&mut self) {
        self.mode = InputMode::Normal;
        self.search_query.clear();
        // Keep the current input (the matched result)
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
