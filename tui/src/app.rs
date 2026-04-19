//! Application state machine.
//!
//! Manages the lifecycle of the TUI: input mode, chat history,
//! popup state, and communication with the Node.js agent.

use crate::protocol::{ClientMessage, CommandInfo, ServerMessage};

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
}

/// Top-level application state.
pub struct App {
    /// Chat history (scrollable).
    pub history: Vec<ChatEntry>,
    /// Current input buffer (multi-line).
    pub input: String,
    /// Cursor position within the input buffer.
    pub cursor: usize,
    /// Input mode.
    pub mode: InputMode,
    /// Input history for Up/Down cycling.
    pub input_history: Vec<String>,
    /// Current position in input history (-1 = current input).
    pub history_index: Option<usize>,
    /// Saved current input when browsing history.
    pub saved_input: String,
    /// Scroll offset for chat history.
    pub scroll_offset: u16,
    /// Banner info from agent.
    pub banner: Option<BannerInfo>,
    /// Available slash commands (received from agent).
    pub commands: Vec<CommandInfo>,
    /// Current popup filter text (after '/').
    pub popup_filter: String,
    /// Selected popup index.
    pub popup_index: usize,
    /// Whether the agent is currently working.
    pub agent_busy: bool,
    /// Current spinner label (if any).
    pub spinner_label: Option<String>,
    /// Spinner frame counter.
    pub spinner_frame: usize,
    /// When the spinner started (for elapsed time display).
    pub spinner_started: Option<std::time::Instant>,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Last submitted input text (for display after submit).
    pub last_submitted: String,
    /// How many lines above the prompt are "owned" by us (popup).
    /// Used for cursor math during redraw — no position queries needed.
    pub owned_lines_above: u16,
    /// Kill buffer for Emacs-style Ctrl+K/U/W/Y editing.
    pub kill_buffer: String,
    /// Timestamp of last Ctrl+C press (for double-press detection).
    pub last_ctrl_c: Option<std::time::Instant>,
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
            owned_lines_above: 0,
            kill_buffer: String::new(),
            last_ctrl_c: None,
        }
    }

    /// Process a message from the Node.js agent.
    pub fn handle_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Banner {
                version,
                provider,
                model,
                directory,
                api_key_set,
            } => {
                self.banner = Some(BannerInfo {
                    version,
                    provider,
                    model,
                    directory,
                    api_key_set,
                });
            }
            ServerMessage::Mission {
                understood,
                target,
                target_type: _,
                steps,
            } => {
                let mut text = format!("Understood: {understood}");
                if let Some(t) = target {
                    text.push_str(&format!("\nTarget: {t}"));
                }
                if !steps.is_empty() {
                    text.push_str("\n\nPlan:");
                    for (i, step) in steps.iter().enumerate() {
                        text.push_str(&format!("\n  {}. {step}", i + 1));
                    }
                }
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::Mission,
                    text,
                });
                self.agent_busy = true;
            }
            ServerMessage::Clarification { question } => {
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::Clarification,
                    text: question,
                });
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
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::ToolStart,
                    text: format!("{tool} — {reasoning}"),
                });
            }
            ServerMessage::ToolResult {
                tool,
                success,
                summary,
                elapsed_ms,
            } => {
                let icon = if success { "✓" } else { "✗" };
                let secs = elapsed_ms as f64 / 1000.0;
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::ToolResult,
                    text: format!("{icon} {tool} — {summary} ({secs:.1}s)"),
                });
            }
            ServerMessage::AgentMessage { text } => {
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::AgentMessage,
                    text,
                });
            }
            ServerMessage::MissionComplete {
                outcome,
                summary,
                turns,
                elapsed_ms,
                ..
            } => {
                let secs = elapsed_ms as f64 / 1000.0;
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::MissionComplete,
                    text: format!("{outcome} ({secs:.1}s, {turns} turns)\n{summary}"),
                });
                self.agent_busy = false;
                self.spinner_label = None;
                self.spinner_started = None;
            }
            ServerMessage::Error { message } => {
                self.history.push(ChatEntry {
                    kind: ChatEntryKind::Error,
                    text: message,
                });
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
            ServerMessage::AgentMessageDelta { .. } => {
                // Streaming deltas are rendered directly in main.rs,
                // no state update needed here.
            }
            ServerMessage::CommandList { commands } => {
                self.commands = commands;
            }
        }
        // Auto-scroll to bottom on new content
        self.scroll_offset = 0;
    }

    /// Build a ClientMessage for the current input and reset state.
    pub fn submit_input(&mut self) -> Option<ClientMessage> {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return None;
        }

        // Save to history
        self.input_history.push(text.clone());
        self.history_index = None;
        self.last_submitted = text.clone();

        // Add to chat display
        self.history.push(ChatEntry {
            kind: ChatEntryKind::UserInput,
            text: text.clone(),
        });

        // Clear input
        self.input.clear();
        self.cursor = 0;

        // Check for slash command
        if text.starts_with('/') {
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            let name = parts[0].to_string();
            let args = parts.get(1).map(|s| s.to_string());
            return Some(ClientMessage::SlashCommand { name, args });
        }

        Some(ClientMessage::UserInput {
            text,
            images: vec![],
        })
    }

    /// Handle approval response.
    pub fn respond_approval(&mut self, approved: bool) -> ClientMessage {
        self.mode = InputMode::Normal;
        ClientMessage::ApprovalResponse { approved }
    }

    /// Get filtered commands for popup.
    pub fn filtered_commands(&self) -> Vec<&CommandInfo> {
        let filter = self.popup_filter.to_lowercase();
        self.commands
            .iter()
            .filter(|c| {
                if filter.is_empty() {
                    return true;
                }
                // Fuzzy subsequence match
                let name = c.name.to_lowercase();
                let mut fi = filter.chars().peekable();
                for ch in name.chars() {
                    if fi.peek() == Some(&ch) {
                        fi.next();
                    }
                }
                fi.peek().is_none()
            })
            .collect()
    }

    /// Move cursor to the start of the previous word.
    pub fn word_left(&mut self) {
        if self.cursor == 0 { return; }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor;
        // Skip whitespace
        while pos > 0 && bytes[pos - 1] == b' ' { pos -= 1; }
        // Skip word chars
        while pos > 0 && bytes[pos - 1] != b' ' { pos -= 1; }
        self.cursor = pos;
    }

    /// Move cursor to the end of the next word.
    pub fn word_right(&mut self) {
        let len = self.input.len();
        if self.cursor >= len { return; }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor;
        // Skip current word chars
        while pos < len && bytes[pos] != b' ' { pos += 1; }
        // Skip whitespace
        while pos < len && bytes[pos] == b' ' { pos += 1; }
        self.cursor = pos;
    }

    /// Kill from cursor to end of line (Ctrl+K).
    pub fn kill_to_end(&mut self) {
        if self.cursor < self.input.len() {
            self.kill_buffer = self.input[self.cursor..].to_string();
            self.input.truncate(self.cursor);
        }
    }

    /// Kill from cursor to start of line (Ctrl+U).
    pub fn kill_to_start(&mut self) {
        if self.cursor > 0 {
            self.kill_buffer = self.input[..self.cursor].to_string();
            self.input = self.input[self.cursor..].to_string();
            self.cursor = 0;
        }
    }

    /// Kill word backward (Ctrl+W).
    pub fn kill_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let bytes = self.input.as_bytes();
        let mut end = self.cursor;
        // Skip trailing whitespace
        while end > 0 && bytes[end - 1] == b' ' {
            end -= 1;
        }
        // Skip word characters
        let start = end;
        let mut pos = end;
        while pos > 0 && bytes[pos - 1] != b' ' {
            pos -= 1;
        }
        if start == self.cursor && pos == self.cursor {
            return;
        }
        let kill_start = pos;
        self.kill_buffer = self.input[kill_start..self.cursor].to_string();
        self.input = format!("{}{}", &self.input[..kill_start], &self.input[self.cursor..]);
        self.cursor = kill_start;
    }

    /// Yank (paste from kill buffer) (Ctrl+Y).
    pub fn yank(&mut self) {
        if !self.kill_buffer.is_empty() {
            let yanked = self.kill_buffer.clone();
            self.input.insert_str(self.cursor, &yanked);
            self.cursor += yanked.len();
        }
    }

    /// Navigate input history (Up = true, Down = false).
    pub fn navigate_history(&mut self, up: bool) {
        if self.input_history.is_empty() {
            return;
        }

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
                        // Back to current input
                        self.history_index = None;
                        self.input = self.saved_input.clone();
                        self.cursor = self.input.len();
                    }
                }
            }
        }
    }
}
