//! DeCIpher CLI — entry point.
//!
//! Wires the Rust-native AgentLoop into the ratatui TUI.  The previous
//! Node.js subprocess model (agent-bridge) is replaced by an in-process
//! tokio task that communicates with the event loop via mpsc channels.
//!
//! Key design choices:
//! - ratatui inline viewport: buffer-diffed viewport at bottom, permanent scrollback above
//! - crossterm `EventStream` for truly async event reading
//! - 32ms tick rate (~30 FPS) for smooth spinner animation
//! - Frame rate limiter (120 FPS cap) to prevent redundant redraws
//! - No manual cursor tracking — structurally impossible cursor bugs
//! - All server messages routed through ChatWidget for typed cells
//! - AgentLoop runs in a spawned tokio task; approval is a bidirectional channel

use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{
        DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste,
        EnableFocusChange, Event, EventStream, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    Terminal, TerminalOptions, Viewport,
};
use tokio::sync::mpsc;

use decipher_protocol::{ClientMessage, ServerMessage};
use decipher_providers::anthropic::AnthropicProvider;
use decipher_providers::openai::OpenAiProvider;
use decipher_providers::Provider;
use decipher_mcp::{McpConfig, McpClient};
use decipher_runtime::{AgentConfig, AgentLoop, HookConfig, load_skills, load_instructions, generate_template};
use decipher_session_store::{load_session, list_sessions, MemoryStore, SessionStore};
use decipher_tui::app::{self, App};
use decipher_tui::bottom_pane::{self, BottomPane};

mod config;
mod insert_history;

/// Tick rate: 32ms ≈ 31.25 FPS — smooth spinner animation matching Codex.
const TICK_RATE: Duration = Duration::from_millis(32);

/// Minimum interval between draws: 120 FPS cap (~8.3ms).
const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_334);

/// Create a ratatui Terminal with a Fixed viewport anchored at the given position.
///
/// `Viewport::Fixed` writes cells at absolute cursor positions and NEVER calls
/// `scroll_up()`, so `terminal.draw()` cannot push viewport content into the
/// terminal's scrollback buffer. Ghost lines from the spinner are structurally
/// impossible with this approach.
fn make_terminal(height: u16, cols: u16, vp_y: u16) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, vp_y, cols, height)),
        },
    )
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli_cfg = config::CliConfig::load();

    // ── Non-interactive exec mode ─────────────────────────────────────────────
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.first().map(|s| s.as_str()) == Some("exec") {
        let exit_code = run_exec_mode(&raw_args[1..], cli_cfg).await;
        std::process::exit(exit_code);
    }

    // Capture terminal size before raw mode. With Viewport::Fixed we don't
    // need the initial newline scroll that Viewport::Inline required, but we
    // keep it so the bottom rows are clear when the viewport is first painted.
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    print!("{}", "\n".repeat(rows as usize));
    io::stdout().flush()?;

    // Raw mode + bracketed paste + focus detection. NO alternate screen.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnableBracketedPaste, EnableFocusChange)?;

    // Keyboard enhancement (Kitty protocol) is skipped to avoid the 200-2000ms
    // detection timeout on non-Kitty terminals (Terminal.app, iTerm2).
    // The TUI works correctly without it — only losing some key disambiguation.

    let result = run_app(cli_cfg, cols, rows).await;

    let mut stdout = io::stdout();
    disable_raw_mode()?;
    execute!(stdout, DisableBracketedPaste, DisableFocusChange)?;
    println!();

    result
}

/// Simple frame rate limiter — tracks last draw time, skips if too soon.
struct FrameRateLimiter {
    last_draw: Option<Instant>,
}

impl FrameRateLimiter {
    fn new() -> Self {
        Self { last_draw: None }
    }

    fn should_draw(&mut self) -> bool {
        let now = Instant::now();
        match self.last_draw {
            Some(last) if now.duration_since(last) < MIN_FRAME_INTERVAL => false,
            _ => {
                self.last_draw = Some(now);
                true
            }
        }
    }

    fn mark_drawn(&mut self) {
        self.last_draw = Some(Instant::now());
    }
}

/// Run the main event loop.
///
/// Owns the terminal and all channel state. The AgentLoop runs as a
/// background tokio task; this loop receives `ServerMessage` events from
/// it and sends approval decisions back via a separate channel.
async fn run_app(cli_cfg: config::CliConfig, initial_cols: u16, initial_rows: u16) -> io::Result<()> {
    // ── Provider ─────────────────────────────────────────────────────────────
    let provider: Arc<dyn Provider> = match cli_cfg.provider_type {
        config::ProviderType::OpenAi => {
            let base = cli_cfg.base_url.as_deref().unwrap_or("https://api.openai.com");
            Arc::new(OpenAiProvider::new(&cli_cfg.api_key, &cli_cfg.model, base))
        }
        config::ProviderType::Anthropic => {
            let mut p = AnthropicProvider::new(&cli_cfg.api_key, &cli_cfg.model);
            if let Some(ref url) = cli_cfg.base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
    };

    // ── Channels ─────────────────────────────────────────────────────────────
    // event_tx  → agent task → event_rx (ServerMessage stream)
    // approval_tx → main loop → approval_rx (bool, per agent run)
    let (event_tx, mut event_rx) = mpsc::channel::<ServerMessage>(64);
    let mut current_approval_tx: Option<mpsc::Sender<bool>> = None;
    let mut current_agent_task: Option<tokio::task::JoinHandle<()>> = None;

    // Extract config fields used repeatedly in the loop.
    let workspace = cli_cfg.workspace.clone();
    let api_key = cli_cfg.api_key.clone();
    let mut model = cli_cfg.model.clone();
    let base_url = cli_cfg.base_url.clone();
    let policy_mode = cli_cfg.policy_mode;
    let mut provider_type = cli_cfg.provider_type;
    let mut provider = provider;

    // ── MCP client initialization (lazy — deferred to first mission) ─────────
    let decipher_home = config::decipher_home();
    let mut mcp_tools: Vec<decipher_mcp::McpTool> = Vec::new();
    let mut mcp_clients_arc: Option<std::sync::Arc<Vec<std::sync::Arc<tokio::sync::Mutex<McpClient>>>>> = None;
    let mut mcp_initialized = false;

    // ── Session store ─────────────────────────────────────────────────────────
    // No Drop impl on SessionStore: session_end is intentionally best-effort —
    // a crash will leave the JSONL without a final record, which is acceptable.
    // Session store is created per-mission (when UserInput is submitted) so that
    // mission_goal is known at creation time.
    let mut session_store: Option<SessionStore> = None;
    let mut last_outcome: Option<String> = None;

    // ── Plan mode state ────────────────────────────────────────────────────────
    // When Some, we received a PLAN MissionComplete and are waiting for yes/no.
    let plan_mode_flag = cli_cfg.plan_mode_flag;
    let mut pending_plan_execution: Option<AgentConfig> = None;

    // ── TUI state ─────────────────────────────────────────────────────────────
    let mut app = App::new();
    let mut need_redraw = true;
    let mut fps = FrameRateLimiter::new();
    let mut events = EventStream::new();

    // Fixed viewport height. The terminal is recreated on resize (cheap —
    // just recreates the ratatui buffer) with the updated position.
    let viewport_height = bottom_pane::MAX_PANE_HEIGHT;
    // vp_y: dynamic 0-based row of viewport top. Shifts down as history is
    // inserted via Reverse Index. Adjusted on resize via cursor delta.
    let mut vp_y = initial_rows.saturating_sub(viewport_height);
    let mut screen_rows = initial_rows;
    // Accumulated scrollback lines — re-emitted on resize to restore content
    // after clearing ghost artifacts from terminal reflow.
    let mut scrollback_history: Vec<ratatui::text::Line<'static>> = Vec::new();
    // Clear the viewport area so no stale shell content shows through.
    let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
    let mut terminal = make_terminal(viewport_height, initial_cols, vp_y)?;

    if initial_cols != app.chat.width() {
        app.chat.set_width(initial_cols);
    }

    // pending_resize: coalesces rapid resize events — only the last per tick is applied.
    let mut pending_resize: Option<(u16, u16)> = None;

    // ── Startup banner ───────────────────────────────────────────────────────
    // Render the banner immediately at TUI startup, not via AgentLoop event.
    // Uses Codex-style Reverse Index + DECSTBM to insert content above the
    // viewport and push the viewport down. The terminal's native scrollback
    // owns this content — immune to resize artifacts.
    {
        let version = env!("CARGO_PKG_VERSION");
        let provider = match cli_cfg.provider_type {
            config::ProviderType::Anthropic => {
                if cli_cfg.base_url.is_some() { "anthropic (custom)" } else { "anthropic" }
            }
            config::ProviderType::OpenAi => "openai-compat",
        };
        let api_key_set = !cli_cfg.api_key.is_empty();
        let directory = std::env::current_dir()
            .map(|p| {
                if let Ok(home) = std::env::var("HOME") {
                    if let Ok(rel) = p.strip_prefix(&home) {
                        return format!("~/{}", rel.display());
                    }
                }
                p.display().to_string()
            })
            .unwrap_or_else(|_| ".".into());

        let mut stdout = io::stdout();
        let _ = decipher_tui::render::set_terminal_title(
            &mut stdout,
            &format!("DeCIpher \u{2014} {}", model),
        );

        let instr_files = load_instructions(&decipher_home, std::path::Path::new(&workspace));
        let instr_display = instr_files.loaded_paths_display();
        let banner = bottom_pane::banner_lines(version, provider, &model, &directory, api_key_set, instr_display.as_deref());
        if !banner.is_empty() {
            scrollback_history.extend(banner.iter().cloned());
            let shift = insert_history::insert_history_lines(&banner, vp_y, screen_rows, initial_cols)?;
            if shift > 0 {
                vp_y += shift;
                let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
                terminal = make_terminal(viewport_height, initial_cols, vp_y)?;
            }
        }

        // Also set banner info in app state for status bar display.
        app.banner = Some(app::BannerInfo {
            version: version.to_string(),
            provider: provider.to_string(),
            model: model.clone(),
            directory: directory.clone(),
            api_key_set,
        });
    }

    loop {
        // ── Apply debounced resize ───────────────────────────────────────────
        // Terminal reflow during drag-resize moves viewport content to
        // unpredictable positions, creating ghost lines.  The fix:
        //   1. Clear the ENTIRE visible screen (kills all ghosts)
        //   2. Anchor viewport at the bottom
        //   3. Re-emit accumulated scrollback history above the viewport
        // Content already in the terminal's native scrollback buffer (scroll
        // up to see) is untouched by ClearType::FromCursorDown.
        if let Some((cols, rows)) = pending_resize.take() {
            app.chat.set_width(cols);
            let vp_h = viewport_height.min(rows);
            screen_rows = rows;

            // 1. Clear entire visible screen — eliminates all ghost artifacts.
            let _ = execute!(io::stdout(), cursor::MoveTo(0, 0), Clear(ClearType::FromCursorDown));

            // 2. Anchor viewport at bottom of new screen.
            vp_y = rows.saturating_sub(vp_h);
            terminal = make_terminal(vp_h, cols, vp_y)?;

            // 3. Rebuild scrollback from cells at the new width, then re-emit.
            scrollback_history = app.chat.rebuild_scrollback(cols);
            if !scrollback_history.is_empty() {
                let shift = insert_history::insert_history_lines(
                    &scrollback_history, vp_y, screen_rows, cols,
                )?;
                if shift > 0 {
                    vp_y += shift;
                    let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
                    terminal = make_terminal(vp_h, cols, vp_y)?;
                }
            }

            need_redraw = true;
        }

        // ── Draw ─────────────────────────────────────────────────────────────
        if need_redraw && fps.should_draw() {
            if app.mode == app::InputMode::Pager {
                let mut stdout = io::stdout();
                decipher_tui::pager::render_pager(&mut stdout, &mut app)?;
            } else {
                terminal.draw(|frame| {
                    let area = frame.area();
                    frame.render_widget(BottomPane::new(&app), area);
                    if let Some((x, y)) = BottomPane::new(&app).cursor_position(area) {
                        frame.set_cursor_position((x, y));
                    }
                })?;
            }
            need_redraw = false;
        }

        tokio::select! {
            biased;

            // ── Terminal events ───────────────────────────────────────────────
            Some(result) = events.next() => {
                match result {
                    Ok(Event::Key(key)) => {
                        if key.kind != KeyEventKind::Press { continue; }
                        let action = handle_key(&mut app, key);
                        match action {
                            KeyAction::Redraw => { need_redraw = true; }
                            KeyAction::Submit(msg) => {
                                // Push submitted user input into scrollback.
                                let lines = bottom_pane::user_input_lines(&app.last_submitted);
                                scrollback_history.extend(lines.iter().cloned());
                                let shift = insert_history::insert_history_lines(&lines, vp_y, screen_rows, app.chat.width())?;
                                if shift > 0 {
                                    vp_y += shift;
                                    let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
                                    terminal = make_terminal(viewport_height, app.chat.width(), vp_y)?;
                                }
                                fps.mark_drawn();
                                need_redraw = true;

                                match msg {
                                    ClientMessage::UserInput { ref text, .. } => {
                                        if let Some(h) = current_agent_task.take() {
                                            h.abort();
                                        }

                                        let text_trimmed = text.trim();
                                        if text_trimmed == "/mcp"
                                            || text_trimmed.starts_with("/mcp ")
                                        {
                                            // ── /mcp ──────────────────────────────────────────
                                            let cfg = McpConfig::load(&decipher_home);
                                            let text = if cfg.servers.is_empty() {
                                                "No MCP servers configured. Add servers to ~/.decipher/mcp.json".to_string()
                                            } else {
                                                let mut lines = format!("Configured MCP servers ({}):\n", cfg.servers.len());
                                                for s in &cfg.servers {
                                                    let tool_count = mcp_tools.iter()
                                                        .filter(|t| t.server_name == s.name)
                                                        .count();
                                                    lines.push_str(&format!(
                                                        "  {} — {} {} tool{}\n",
                                                        s.name, s.command,
                                                        tool_count,
                                                        if tool_count == 1 { "" } else { "s" }
                                                    ));
                                                }
                                                if !mcp_tools.is_empty() {
                                                    lines.push_str("\nAvailable tools:\n");
                                                    for t in &mcp_tools {
                                                        lines.push_str(&format!("  [{}] {}", t.server_name, t.name));
                                                        if !t.description.is_empty() {
                                                            lines.push_str(&format!(" — {}", t.description));
                                                        }
                                                        lines.push('\n');
                                                    }
                                                }
                                                lines.trim_end().to_string()
                                            };
                                            let _ = event_tx.try_send(ServerMessage::AgentMessage { text });
                                        } else if text_trimmed == "/hooks"
                                            || text_trimmed.starts_with("/hooks ")
                                        {
                                            // ── /hooks ────────────────────────────────────────
                                            let hc = HookConfig::load(&decipher_home);
                                            let total = hc.pre_tool_use.len()
                                                + hc.post_tool_use.len()
                                                + hc.session_start.len()
                                                + hc.session_end.len();
                                            let text = if total == 0 {
                                                "No hooks configured. Add hooks.json to ~/.decipher/".to_string()
                                            } else {
                                                let mut lines = String::from("Configured hooks:\n");
                                                if !hc.pre_tool_use.is_empty() {
                                                    lines.push_str(&format!("  PreToolUse ({}):\n", hc.pre_tool_use.len()));
                                                    for h in &hc.pre_tool_use { lines.push_str(&format!("    {}\n", h.command)); }
                                                }
                                                if !hc.post_tool_use.is_empty() {
                                                    lines.push_str(&format!("  PostToolUse ({}):\n", hc.post_tool_use.len()));
                                                    for h in &hc.post_tool_use { lines.push_str(&format!("    {}\n", h.command)); }
                                                }
                                                if !hc.session_start.is_empty() {
                                                    lines.push_str(&format!("  SessionStart ({}):\n", hc.session_start.len()));
                                                    for h in &hc.session_start { lines.push_str(&format!("    {}\n", h.command)); }
                                                }
                                                if !hc.session_end.is_empty() {
                                                    lines.push_str(&format!("  SessionEnd ({}):\n", hc.session_end.len()));
                                                    for h in &hc.session_end { lines.push_str(&format!("    {}\n", h.command)); }
                                                }
                                                lines.trim_end().to_string()
                                            };
                                            let _ = event_tx.try_send(ServerMessage::AgentMessage { text });
                                        } else if text_trimmed == "/skills"
                                            || text_trimmed.starts_with("/skills ")
                                        {
                                            // ── /skills ───────────────────────────────────────
                                            let loaded = load_skills(
                                                &decipher_home,
                                                std::path::Path::new(&workspace),
                                            );
                                            let text = if loaded.is_empty() {
                                                "No skills loaded. Add SKILL.md files to ~/.decipher/skills/<name>/ or .decipher/skills/<name>/.".to_string()
                                            } else {
                                                let mut lines = format!("Loaded skills ({}):\n", loaded.len());
                                                for s in &loaded {
                                                    if s.description.is_empty() {
                                                        lines.push_str(&format!("  {}\n", s.name));
                                                    } else {
                                                        lines.push_str(&format!("  {} — {}\n", s.name, s.description));
                                                    }
                                                }
                                                lines.trim_end().to_string()
                                            };
                                            let _ = event_tx.try_send(ServerMessage::AgentMessage { text });
                                        } else if text_trimmed == "/memory"
                                            || text_trimmed.starts_with("/memory ")
                                        {
                                            // ── /memory [list|add <text>|clear] ──────────────
                                            let sub = text_trimmed["/memory".len()..].trim();
                                            match MemoryStore::new(&decipher_home, &workspace) {
                                                Err(e) => {
                                                    let _ = event_tx.try_send(ServerMessage::Error {
                                                        message: format!("memory: {e}"),
                                                    });
                                                }
                                                Ok(mem_store) => {
                                                    if sub.is_empty() || sub == "list" {
                                                        match mem_store.list() {
                                                            Ok(entries) if entries.is_empty() => {
                                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                                    text: "No memories stored. Use /memory add <text> to add one.".into(),
                                                                });
                                                            }
                                                            Ok(entries) => {
                                                                let mut lines = format!("Stored memories ({}):\n", entries.len());
                                                                for e in &entries {
                                                                    let short = &e.id[..e.id.len().min(8)];
                                                                    lines.push_str(&format!("  [{short}] {}\n", e.content));
                                                                }
                                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                                    text: lines.trim_end().to_string(),
                                                                });
                                                            }
                                                            Err(e) => {
                                                                let _ = event_tx.try_send(ServerMessage::Error {
                                                                    message: format!("memory list: {e}"),
                                                                });
                                                            }
                                                        }
                                                    } else if let Some(content) = sub.strip_prefix("add ") {
                                                        match mem_store.add(content.trim()) {
                                                            Ok(id) => {
                                                                let short = &id[..id.len().min(8)];
                                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                                    text: format!("Memory saved [{short}]: {}", content.trim()),
                                                                });
                                                            }
                                                            Err(e) => {
                                                                let _ = event_tx.try_send(ServerMessage::Error {
                                                                    message: format!("memory add: {e}"),
                                                                });
                                                            }
                                                        }
                                                    } else if sub == "clear" {
                                                        match mem_store.clear() {
                                                            Ok(()) => {
                                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                                    text: "All memories cleared.".into(),
                                                                });
                                                            }
                                                            Err(e) => {
                                                                let _ = event_tx.try_send(ServerMessage::Error {
                                                                    message: format!("memory clear: {e}"),
                                                                });
                                                            }
                                                        }
                                                    } else {
                                                        let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                            text: "Usage: /memory [list|add <text>|clear]".into(),
                                                        });
                                                    }
                                                }
                                            }
                                        } else if text_trimmed == "/sessions"
                                            || text_trimmed.starts_with("/sessions ")
                                        {
                                            // ── /sessions ─────────────────────────────────────
                                            match list_sessions(&decipher_home).await {
                                                Ok(sessions) if sessions.is_empty() => {
                                                    let _ = event_tx.try_send(
                                                        ServerMessage::AgentMessage {
                                                            text: "No sessions recorded yet."
                                                                .into(),
                                                        },
                                                    );
                                                }
                                                Ok(sessions) => {
                                                    let mut lines =
                                                        String::from("Recorded sessions:\n");
                                                    for s in &sessions {
                                                        let short_id = &s.thread_id
                                                            [..s.thread_id.len().min(8)];
                                                        let goal: String = s
                                                            .mission_goal
                                                            .chars()
                                                            .take(50)
                                                            .collect();
                                                        let goal = if s.mission_goal.len() > 50 {
                                                            format!("{goal}…")
                                                        } else {
                                                            goal
                                                        };
                                                        let outcome = s
                                                            .outcome
                                                            .as_deref()
                                                            .unwrap_or("—");
                                                        let date = s
                                                            .started_at
                                                            .format("%Y-%m-%d %H:%M")
                                                            .to_string();
                                                        lines.push_str(&format!(
                                                            "[{short_id}] {goal} — {outcome} — {date}\n"
                                                        ));
                                                    }
                                                    let _ = event_tx.try_send(
                                                        ServerMessage::AgentMessage {
                                                            text: lines.trim_end().to_string(),
                                                        },
                                                    );
                                                }
                                                Err(e) => {
                                                    let _ = event_tx.try_send(
                                                        ServerMessage::Error {
                                                            message: format!("sessions: {e}"),
                                                        },
                                                    );
                                                }
                                            }
                                        } else if text_trimmed == "/model"
                                            || text_trimmed.starts_with("/model ")
                                        {
                                            // ── /model [name] ────────────────────────────────
                                            let arg = text_trimmed["/model".len()..].trim();
                                            if arg.is_empty() {
                                                let ptype = match provider_type {
                                                    config::ProviderType::Anthropic => "anthropic",
                                                    config::ProviderType::OpenAi => "openai-compat",
                                                };
                                                let mut tags = vec![ptype.to_string()];
                                                if decipher_providers::model_quirks::is_reasoning_model(&model) {
                                                    tags.push("reasoning".to_string());
                                                }
                                                if decipher_providers::model_quirks::supports_thinking_mode(&model) {
                                                    tags.push("thinking".to_string());
                                                }
                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                    text: format!("Current model: {} ({})", model, tags.join(", ")),
                                                });
                                            } else {
                                                model = arg.to_string();
                                                // Re-detect provider type from new model.
                                                provider_type = config::auto_detect_provider(
                                                    base_url.as_deref(), &model,
                                                );
                                                provider = match provider_type {
                                                    config::ProviderType::OpenAi => {
                                                        let base = base_url.as_deref().unwrap_or("https://api.openai.com");
                                                        Arc::new(OpenAiProvider::new(&api_key, &model, base))
                                                    }
                                                    config::ProviderType::Anthropic => {
                                                        let mut p = AnthropicProvider::new(&api_key, &model);
                                                        if let Some(ref url) = base_url {
                                                            p = p.with_base_url(url);
                                                        }
                                                        Arc::new(p)
                                                    }
                                                };
                                                // Update banner.
                                                if let Some(ref mut b) = app.banner {
                                                    b.model = model.clone();
                                                }
                                                let ptype = match provider_type {
                                                    config::ProviderType::Anthropic => "anthropic",
                                                    config::ProviderType::OpenAi => "openai-compat",
                                                };
                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                    text: format!("Switched to model: {} ({})", model, ptype),
                                                });
                                                // Update terminal title.
                                                let mut stdout = io::stdout();
                                                let _ = decipher_tui::render::set_terminal_title(
                                                    &mut stdout,
                                                    &format!("DeCIpher \u{2014} {}", model),
                                                );
                                            }
                                        } else if text_trimmed == "/export"
                                            || text_trimmed.starts_with("/export ")
                                        {
                                            // ── /export [path] ───────────────────────────────
                                            let arg = text_trimmed["/export".len()..].trim();
                                            let out_path = if arg.is_empty() {
                                                let ts = std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .unwrap_or_default()
                                                    .as_secs();
                                                format!("decipher-session-{ts}.md")
                                            } else {
                                                arg.to_string()
                                            };
                                            // Generate markdown from transcript lines.
                                            let lines = app.chat.transcript_lines(120);
                                            let mut md = String::from("# DeCIpher Session Export\n\n");
                                            for line in &lines {
                                                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                                                md.push_str(&text);
                                                md.push('\n');
                                            }
                                            match std::fs::write(&out_path, &md) {
                                                Ok(()) => {
                                                    let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                        text: format!("Session exported to {out_path} ({} lines)", lines.len()),
                                                    });
                                                }
                                                Err(e) => {
                                                    let _ = event_tx.try_send(ServerMessage::Error {
                                                        message: format!("export: {e}"),
                                                    });
                                                }
                                            }
                                        } else if text_trimmed == "/init" {
                                            // ── /init ─────────────────────────────────────────
                                            let text = match generate_template(std::path::Path::new(&workspace)) {
                                                Ok(path) => format!("Created {}", path.display()),
                                                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                                                    format!("DECIPHER.md already exists at {}/DECIPHER.md", workspace)
                                                }
                                                Err(e) => format!("Failed to create DECIPHER.md: {}", e),
                                            };
                                            let _ = event_tx.try_send(ServerMessage::AgentMessage { text });
                                        } else if text_trimmed.starts_with("/resume") {
                                            // ── /resume [thread_id] ─────────────────────────
                                            let arg = text_trimmed["/resume".len()..].trim();
                                            let tid_opt: Option<String> = if arg.is_empty() {
                                                match list_sessions(&decipher_home).await {
                                                    Ok(list) => list.into_iter().next().map(|e| e.thread_id),
                                                    Err(_) => None,
                                                }
                                            } else {
                                                Some(arg.to_string())
                                            };

                                            match tid_opt {
                                                None => {
                                                    let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                        text: "error: no sessions found to resume".into(),
                                                    });
                                                }
                                                Some(tid) => match load_session(&decipher_home, &tid).await {
                                                    Err(e) => {
                                                        let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                            text: format!("error: could not resume {tid}: {e}"),
                                                        });
                                                    }
                                                    Ok((meta, history)) => {
                                                        // Close previous session if any.
                                                        if let Some(ss) = session_store.take() {
                                                            let prev = last_outcome.take();
                                                            tokio::spawn(async move { ss.close(prev).await; });
                                                        }
                                                        session_store = SessionStore::new(
                                                            &decipher_home, &model,
                                                            &meta.workspace, &meta.mission_goal,
                                                        ).await.ok();

                                                        // Lazy MCP init for resume path.
                                                        if !mcp_initialized {
                                                            let (mt, mc) = init_mcp_clients(&decipher_home).await;
                                                            mcp_tools = mt;
                                                            mcp_clients_arc = mc;
                                                            mcp_initialized = true;
                                                        }
                                                        let (atx, arx) = mpsc::channel::<bool>(1);
                                                        current_approval_tx = Some(atx);
                                                        let mut resume_cfg = build_agent_config(
                                                            meta.mission_goal.clone(),
                                                            api_key.clone(),
                                                            model.clone(),
                                                            base_url.clone(),
                                                            meta.workspace.clone(),
                                                            policy_mode,
                                                            &decipher_home,
                                                            None,
                                                            mcp_tools.clone(),
                                                            mcp_clients_arc.clone(),
                                                        );
                                                        resume_cfg.resume_from = Some(history);
                                                        let tx = event_tx.clone();
                                                        let prov = provider.clone();
                                                        current_agent_task = Some(tokio::spawn(async move {
                                                            if let Err(e) = AgentLoop::run(resume_cfg, &*prov, tx.clone(), Some(arx)).await {
                                                                let _ = tx.send(ServerMessage::Error { message: format!("Agent error: {e}") }).await;
                                                            }
                                                        }));
                                                    }
                                                },
                                            }
                                        } else if let Some(exec_cfg) = pending_plan_execution.take() {
                                            // ── Plan approval: yes/no ─────────────────────────
                                            let answer = text_trimmed.to_lowercase();
                                            if answer == "yes" || answer == "y" {
                                                let (atx, arx) = mpsc::channel::<bool>(1);
                                                current_approval_tx = Some(atx);
                                                let tx = event_tx.clone();
                                                let prov = provider.clone();
                                                current_agent_task = Some(tokio::spawn(async move {
                                                    if let Err(e) = AgentLoop::run(exec_cfg, &*prov, tx.clone(), Some(arx)).await {
                                                        let _ = tx.send(ServerMessage::Error { message: format!("Agent error: {e}") }).await;
                                                    }
                                                }));
                                            } else {
                                                let _ = event_tx.try_send(ServerMessage::AgentMessage {
                                                    text: "Plan cancelled.".into(),
                                                });
                                            }
                                        } else {
                                            // ── Lazy MCP init (once) ─────────────────────────
                                            if !mcp_initialized {
                                                let (mt, mc) = init_mcp_clients(&decipher_home).await;
                                                mcp_tools = mt;
                                                mcp_clients_arc = mc;
                                                mcp_initialized = true;
                                            }
                                            // ── Normal mission ───────────────────────────────
                                            // Close previous session; open one for this mission.
                                            if let Some(ss) = session_store.take() {
                                                let prev = last_outcome.take();
                                                tokio::spawn(async move { ss.close(prev).await; });
                                            }
                                            session_store = SessionStore::new(
                                                &decipher_home, &model, &workspace, text,
                                            ).await.ok();

                                            let (atx, arx) = mpsc::channel::<bool>(1);
                                            current_approval_tx = Some(atx);
                                            let mut agent_cfg = build_agent_config(
                                                text.clone(),
                                                api_key.clone(),
                                                model.clone(),
                                                base_url.clone(),
                                                workspace.clone(),
                                                policy_mode,
                                                &decipher_home,
                                                None,
                                                mcp_tools.clone(),
                                                mcp_clients_arc.clone(),
                                            );
                                            // Apply plan mode flag if set.
                                            if plan_mode_flag {
                                                agent_cfg.plan_mode = true;
                                            }
                                            let tx = event_tx.clone();
                                            let prov = provider.clone();
                                            current_agent_task = Some(tokio::spawn(async move {
                                                if let Err(e) = AgentLoop::run(agent_cfg, &*prov, tx.clone(), Some(arx)).await {
                                                    let _ = tx.send(ServerMessage::Error { message: format!("Agent error: {e}") }).await;
                                                }
                                            }));
                                        }
                                    }
                                    ClientMessage::ApprovalResponse { approved } => {
                                        if let Some(atx) = &current_approval_tx {
                                            let _ = atx.try_send(approved);
                                        }
                                    }
                                    ClientMessage::Interrupt => {
                                        if let Some(h) = current_agent_task.take() {
                                            h.abort();
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            KeyAction::None => {}
                        }
                    }
                    Ok(Event::Paste(text)) => {
                        app.input.insert_str(app.cursor, &text);
                        app.cursor += text.len();
                        need_redraw = true;
                    }
                    Ok(Event::Resize(cols, rows)) => {
                        // RESIZE-1: debounce — only the last resize per tick is applied.
                        // Avoids saturating the terminal with rapid resize processing
                        // when the user drags the window border quickly.
                        pending_resize = Some((cols, rows));
                    }
                    Ok(Event::FocusGained) => { app.terminal_focused = true; }
                    Ok(Event::FocusLost) => { app.terminal_focused = false; }
                    Ok(_) => {}
                    Err(_) => { break; }
                }
            }

            // ── Agent events (replaces bridge.rx.recv()) ─────────────────────
            Some(msg) = event_rx.recv() => {
                // Banner: render above scrollback + set terminal title.
                if let ServerMessage::Banner { ref version, ref provider, ref model, ref directory, api_key_set } = msg {
                    let mut stdout = io::stdout();
                    decipher_tui::render::set_terminal_title(&mut stdout, &format!("DeCIpher \u{2014} {model}"))?;
                    let banner = bottom_pane::banner_lines(version, provider, model, directory, api_key_set, None);
                    if !banner.is_empty() {
                        scrollback_history.extend(banner.iter().cloned());
                        let shift = insert_history::insert_history_lines(&banner, vp_y, screen_rows, app.chat.width())?;
                        if shift > 0 {
                            vp_y += shift;
                            let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
                            terminal = make_terminal(viewport_height, app.chat.width(), vp_y)?;
                        }
                    }
                }

                // Desktop notification on mission complete when unfocused.
                if let ServerMessage::MissionComplete { ref outcome, ref summary, .. } = msg {
                    if !app.terminal_focused {
                        let mut stdout = io::stdout();
                        let _ = decipher_tui::render::send_notification(
                            &mut stdout,
                            &format!("DeCIpher: {outcome} \u{2014} {summary}"),
                        );
                    }
                }

                // Early auto-approve: when always_approve is set, skip the transcript card
                // entirely and respond immediately without entering ApprovalPending mode.
                if app.always_approve {
                    if let ServerMessage::ApprovalRequest { .. } = &msg {
                        if let Some(atx) = &current_approval_tx {
                            let _ = atx.try_send(true);
                        }
                        need_redraw = true;
                        continue;
                    }
                }

                // Route through ChatWidget → typed cells → scrollback lines.
                let scrollback_lines = app.chat.handle_server_message(&msg);
                if !scrollback_lines.is_empty() {
                    scrollback_history.extend(scrollback_lines.iter().cloned());
                    let shift = insert_history::insert_history_lines(&scrollback_lines, vp_y, screen_rows, app.chat.width())?;
                    if shift > 0 {
                        vp_y += shift;
                        let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
                        terminal = make_terminal(viewport_height, app.chat.width(), vp_y)?;
                    }
                }

                // Record to session JSONL before msg is consumed.
                if let Some(ref ss) = session_store {
                    ss.record(&msg);
                }
                if let ServerMessage::MissionComplete { ref outcome, .. } = msg {
                    last_outcome = Some(outcome.clone());
                }

                // Handle PLAN outcome: save the execution config, prompt yes/no.
                if let ServerMessage::MissionComplete { ref outcome, ref summary, .. } = msg {
                    if outcome == "PLAN" && plan_mode_flag {
                        // Build the execution config (plan_mode = false this time).
                        let mut exec_cfg = build_agent_config(
                            app.last_submitted.clone(),
                            api_key.clone(),
                            model.clone(),
                            base_url.clone(),
                            workspace.clone(),
                            policy_mode,
                            &decipher_home,
                            None,
                            mcp_tools.clone(),
                            mcp_clients_arc.clone(),
                        );
                        // 5B: Inject the approved plan into the execution agent's context
                        // so it starts execution with full knowledge of the plan steps.
                        exec_cfg.memory_context = Some(format!(
                            "## Approved Plan\n{summary}\n\nFollow this plan. Execute each step in order."
                        ));
                        pending_plan_execution = Some(exec_cfg);
                        // Show the plan as an AgentMessage.
                        let plan_display = format!(
                            "{summary}\n\nPlan ready. Type 'yes' to execute, 'no' to cancel."
                        );
                        let _ = event_tx.try_send(ServerMessage::AgentMessage {
                            text: plan_display,
                        });
                        need_redraw = true;
                        // Don't forward the raw MissionComplete to TUI cells.
                        continue;
                    }
                }

                fps.mark_drawn();
                app.handle_server_message(msg);
                need_redraw = true;

            }

            // ── Tick — spinner animation ──────────────────────────────────────
            _ = tokio::time::sleep(TICK_RATE) => {
                if app.spinner_label.is_some() || app.chat.is_streaming() {
                    app.spinner_frame += 1;
                    need_redraw = true;
                }
            }
        }

        // Dispatch queued message when agent becomes idle.
        if !app.agent_busy {
            if let Some(queued) = app.queued_message.take() {
                if let ClientMessage::UserInput { ref text, .. } = queued {
                    let lines = bottom_pane::user_input_lines(&app.last_submitted);
                    scrollback_history.extend(lines.iter().cloned());
                    let shift = insert_history::insert_history_lines(&lines, vp_y, screen_rows, app.chat.width())?;
                    if shift > 0 {
                        vp_y += shift;
                        let _ = execute!(io::stdout(), cursor::MoveTo(0, vp_y), Clear(ClearType::FromCursorDown));
                        terminal = make_terminal(viewport_height, app.chat.width(), vp_y)?;
                    }
                    fps.mark_drawn();
                    need_redraw = true;

                    if let Some(h) = current_agent_task.take() {
                        h.abort();
                    }
                    // Close previous session; open one for this queued mission.
                    if let Some(ss) = session_store.take() {
                        let prev = last_outcome.take();
                        tokio::spawn(async move { ss.close(prev).await; });
                    }
                    session_store = SessionStore::new(
                        &decipher_home, &model, &workspace, text,
                    ).await.ok();

                    let (atx, arx) = mpsc::channel::<bool>(1);
                    current_approval_tx = Some(atx);
                    let mut agent_cfg = build_agent_config(
                        text.clone(),
                        api_key.clone(),
                        model.clone(),
                        base_url.clone(),
                        workspace.clone(),
                        policy_mode,
                        &decipher_home,
                        None,
                        mcp_tools.clone(),
                        mcp_clients_arc.clone(),
                    );
                    // 5C: propagate plan_mode to queued message dispatch.
                    if plan_mode_flag {
                        agent_cfg.plan_mode = true;
                    }
                    let tx = event_tx.clone();
                    let prov = provider.clone();
                    current_agent_task = Some(tokio::spawn(async move {
                        if let Err(e) = AgentLoop::run(agent_cfg, &*prov, tx.clone(), Some(arx)).await {
                            let _ = tx.send(ServerMessage::Error { message: format!("Agent error: {e}") }).await;
                        }
                    }));
                }
            }
        }

        if app.should_quit {
            // Abort agent task if running.
            if let Some(h) = current_agent_task.take() {
                h.abort();
            }
            // Gracefully shut down MCP server processes.
            if let Some(ref clients) = mcp_clients_arc {
                for client_arc in clients.as_ref() {
                    let mut client = client_arc.lock().await;
                    client.shutdown().await;
                }
            }
            // Flush and close the session JSONL.
            if let Some(ss) = session_store.take() {
                ss.close(last_outcome.clone()).await;
            }
            let lines = bottom_pane::goodbye_lines();
            let _ = insert_history::insert_history_lines(&lines, vp_y, screen_rows, app.chat.width());
            break;
        }
    }

    Ok(())
}

/// Non-interactive exec mode: run a mission headlessly, print results, return exit code.
///
/// Exit codes: 0=PASS, 1=FAIL, 2=PARTIAL, 3=error.
async fn run_exec_mode(args: &[String], cli_cfg: config::CliConfig) -> i32 {
    // ── Parse arguments ───────────────────────────────────────────────────────
    // Usage: exec <task> [--output-format text|json] [--quiet]
    let mut task = String::new();
    let mut output_format = "text".to_string();
    let mut quiet = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output-format" => {
                i += 1;
                if i < args.len() {
                    output_format = args[i].clone();
                }
            }
            "--quiet" => {
                quiet = true;
            }
            other if !other.starts_with("--") => {
                if task.is_empty() {
                    task = other.to_string();
                } else {
                    task.push(' ');
                    task.push_str(other);
                }
            }
            _ => {}
        }
        i += 1;
    }

    if task.is_empty() {
        eprintln!("decipher exec: task text is required");
        eprintln!("Usage: decipher exec <task> [--output-format text|json] [--quiet]");
        return 3;
    }

    // ── Provider ──────────────────────────────────────────────────────────────
    let provider: Arc<dyn Provider> = match cli_cfg.provider_type {
        config::ProviderType::OpenAi => {
            let base = cli_cfg.base_url.as_deref().unwrap_or("https://api.openai.com");
            Arc::new(OpenAiProvider::new(&cli_cfg.api_key, &cli_cfg.model, base))
        }
        config::ProviderType::Anthropic => {
            let mut p = AnthropicProvider::new(&cli_cfg.api_key, &cli_cfg.model);
            if let Some(ref url) = cli_cfg.base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
    };

    // ── Run agent ─────────────────────────────────────────────────────────────
    let (event_tx, mut event_rx) = mpsc::channel::<ServerMessage>(128);

    let decipher_home = config::decipher_home();
    let (exec_mcp_tools, exec_mcp_clients) = init_mcp_clients(&decipher_home).await;
    let agent_cfg = build_agent_config(
        task.clone(),
        cli_cfg.api_key.clone(),
        cli_cfg.model.clone(),
        cli_cfg.base_url.clone(),
        cli_cfg.workspace.clone(),
        cli_cfg.policy_mode,
        &decipher_home,
        None,
        exec_mcp_tools,
        exec_mcp_clients,
    );

    let prov = provider.clone();
    let tx = event_tx.clone();
    let _agent_task = tokio::spawn(async move {
        if let Err(e) = AgentLoop::run(agent_cfg, &*prov, tx.clone(), None).await {
            let _ = tx.send(ServerMessage::Error { message: format!("Agent error: {e}") }).await;
        }
    });

    let exec_start = Instant::now();
    let mut outcome = "FAIL".to_string();
    let mut summary = String::new();
    let mut turns = 0u32;

    // Drain events until MissionComplete.
    while let Some(msg) = event_rx.recv().await {
        if !quiet {
            match &msg {
                ServerMessage::AgentStatus { phase, turn, .. } => {
                    eprintln!("[turn {turn}] {phase}");
                }
                ServerMessage::ToolStart { tool, .. } => {
                    eprintln!("  → {tool}");
                }
                ServerMessage::ToolResult { tool, success, summary: s, .. } => {
                    let status = if *success { "ok" } else { "fail" };
                    eprintln!("  ← {tool} [{status}] {s}");
                }
                ServerMessage::AgentMessage { text } => {
                    eprintln!("  {text}");
                }
                _ => {}
            }
        }
        if let ServerMessage::MissionComplete { outcome: o, summary: s, turns: t, .. } = msg {
            outcome = o;
            summary = s;
            turns = t;
            break;
        }
    }

    let elapsed_ms = exec_start.elapsed().as_millis() as u64;

    // ── Output ────────────────────────────────────────────────────────────────
    if output_format == "json" {
        println!(
            r#"{{"outcome":"{outcome}","summary":{summary_json},"turns":{turns},"elapsed_ms":{elapsed_ms}}}"#,
            summary_json = serde_json::Value::String(summary.clone()),
        );
    } else {
        println!("outcome: {outcome}");
        println!("summary: {summary}");
    }

    // ── Exit code ─────────────────────────────────────────────────────────────
    match outcome.as_str() {
        "PASS" => 0,
        "PARTIAL" => 2,
        _ => 1,
    }
}

/// Build an `AgentConfig` for a single mission run.
#[allow(clippy::too_many_arguments)]
fn build_agent_config(
    goal: String,
    api_key: String,
    model: String,
    base_url: Option<String>,
    workspace: String,
    policy_mode: decipher_policy::PolicyMode,
    decipher_home: &std::path::Path,
    memory_context: Option<String>,
    mcp_tools: Vec<decipher_mcp::McpTool>,
    mcp_clients: Option<std::sync::Arc<Vec<std::sync::Arc<tokio::sync::Mutex<McpClient>>>>>,
) -> AgentConfig {
    let skills = load_skills(decipher_home, std::path::Path::new(&workspace));
    let instructions = load_instructions(decipher_home, std::path::Path::new(&workspace));
    let git_context = decipher_runtime::collect_git_context(std::path::Path::new(&workspace));
    let hook_config = HookConfig::load(decipher_home);
    // Load memory context if not explicitly provided.
    let memory_context = memory_context.or_else(|| {
        MemoryStore::new(decipher_home, &workspace)
            .ok()
            .and_then(|m| m.load_all_for_injection().ok())
            .filter(|s| !s.is_empty())
    });
    AgentConfig {
        model,
        api_key,
        base_url,
        workspace,
        mission_goal: goal,
        policy_mode,
        skills,
        instructions,
        memory_context,
        hook_config,
        mcp_tools,
        mcp_clients,
        git_context,
        ..Default::default()
    }
}

enum KeyAction {
    None,
    Redraw,
    Submit(ClientMessage),
}

fn handle_key(app: &mut App, key: KeyEvent) -> KeyAction {
    // Ctrl+C: interrupt agent or quit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if app.agent_busy {
            app.agent_busy = false;
            app.spinner_label = None;
            app.chat.finalize_active_cell_as_failed();
            return KeyAction::Submit(ClientMessage::Interrupt);
        }
        let now = Instant::now();
        if let Some(last) = app.last_ctrl_c {
            if now.duration_since(last) < Duration::from_secs(1) {
                app.should_quit = true;
                return KeyAction::Redraw;
            }
        }
        app.last_ctrl_c = Some(now);
        if !app.input.is_empty() || !app.pending_images.is_empty() {
            app.input.clear();
            app.cursor = 0;
            app.pending_images.clear();
            return KeyAction::Redraw;
        }
        app.should_quit = true;
        return KeyAction::Redraw;
    }

    // Ctrl+D: quit on empty, forward-delete otherwise
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
        if app.input.is_empty() && app.mode == app::InputMode::Normal {
            app.should_quit = true;
            return KeyAction::Redraw;
        }
        if app.cursor < app.input.len() {
            app.input.remove(app.cursor);
            return KeyAction::Redraw;
        }
        return KeyAction::None;
    }

    // Ctrl+Z: suspend (Unix only)
    #[cfg(unix)]
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('z') {
        unsafe {
            libc::raise(libc::SIGTSTP);
        }
        return KeyAction::Redraw;
    }

    match app.mode {
        app::InputMode::ApprovalPending => match key.code {
            // Direct key shortcuts (still work for fast users).
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                KeyAction::Submit(app.respond_approval(true))
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.always_approve = true;
                KeyAction::Submit(app.respond_approval(true))
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                KeyAction::Submit(app.respond_approval(false))
            }
            // Arrow-key navigation in popup.
            KeyCode::Up => {
                if app.approval_index > 0 { app.approval_index -= 1; }
                KeyAction::Redraw
            }
            KeyCode::Down => {
                if app.approval_index < 2 { app.approval_index += 1; }
                KeyAction::Redraw
            }
            // Enter confirms the selected option.
            KeyCode::Enter => {
                match app.approval_index {
                    0 => KeyAction::Submit(app.respond_approval(true)),
                    1 => { app.always_approve = true; KeyAction::Submit(app.respond_approval(true)) }
                    _ => KeyAction::Submit(app.respond_approval(false)),
                }
            }
            _ => KeyAction::None,
        },

        app::InputMode::HistorySearch => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('r') => { app.search_history_older(); return KeyAction::Redraw; }
                    KeyCode::Char('s') => { app.search_history_newer(); return KeyAction::Redraw; }
                    KeyCode::Char('c') => { app.cancel_history_search(); return KeyAction::Redraw; }
                    KeyCode::Char('u') => { app.search_query.clear(); app.search_match_index = None; return KeyAction::Redraw; }
                    _ => {}
                }
            }
            match key.code {
                KeyCode::Enter => { app.accept_history_search(); KeyAction::Redraw }
                KeyCode::Esc => { app.cancel_history_search(); KeyAction::Redraw }
                KeyCode::Up => { app.search_history_older(); KeyAction::Redraw }
                KeyCode::Down => { app.search_history_newer(); KeyAction::Redraw }
                KeyCode::Backspace => {
                    app.search_query.pop();
                    if !app.search_query.is_empty() { app.search_history_older(); }
                    else { app.search_match_index = None; app.input = app.search_saved_input.clone(); app.cursor = app.input.len(); }
                    KeyAction::Redraw
                }
                KeyCode::Char(c) => {
                    app.search_query.push(c);
                    app.search_match_index = None;
                    app.search_history_older();
                    KeyAction::Redraw
                }
                _ => KeyAction::None,
            }
        }

        app::InputMode::Pager => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.mode = app::InputMode::Normal;
                app.pager_scroll = 0;
                KeyAction::Redraw
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.pager_scroll = app.pager_scroll.saturating_add(1);
                KeyAction::Redraw
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.pager_scroll = app.pager_scroll.saturating_sub(1);
                KeyAction::Redraw
            }
            KeyCode::Char('d') => {
                app.pager_scroll = app.pager_scroll.saturating_add(20);
                KeyAction::Redraw
            }
            KeyCode::Char('u') => {
                app.pager_scroll = app.pager_scroll.saturating_sub(20);
                KeyAction::Redraw
            }
            KeyCode::Char('g') | KeyCode::Home => {
                app.pager_scroll = 0;
                KeyAction::Redraw
            }
            KeyCode::Char('G') | KeyCode::End => {
                app.pager_scroll = usize::MAX;
                KeyAction::Redraw
            }
            KeyCode::PageDown => {
                let h = crossterm::terminal::size().map(|(_, h)| h as usize).unwrap_or(40);
                app.pager_scroll = app.pager_scroll.saturating_add(h.saturating_sub(2));
                KeyAction::Redraw
            }
            KeyCode::PageUp => {
                let h = crossterm::terminal::size().map(|(_, h)| h as usize).unwrap_or(40);
                app.pager_scroll = app.pager_scroll.saturating_sub(h.saturating_sub(2));
                KeyAction::Redraw
            }
            _ => KeyAction::None,
        },

        app::InputMode::FileSearch => match key.code {
            KeyCode::Esc => {
                let end = app.file_search_at_pos + 1 + app.file_search_query.len();
                let end = end.min(app.input.len());
                app.input.replace_range(app.file_search_at_pos..end, "");
                app.cursor = app.file_search_at_pos;
                app.mode = app::InputMode::Normal;
                app.file_search_results.clear();
                KeyAction::Redraw
            }
            KeyCode::Up => {
                if app.file_search_index > 0 { app.file_search_index -= 1; }
                KeyAction::Redraw
            }
            KeyCode::Down => {
                let max = app.file_search_results.len().saturating_sub(1);
                if app.file_search_index < max { app.file_search_index += 1; }
                KeyAction::Redraw
            }
            KeyCode::Enter | KeyCode::Tab => {
                if let Some(result) = app.file_search_results.get(app.file_search_index) {
                    let path = result.path.clone();
                    let end = app.file_search_at_pos + 1 + app.file_search_query.len();
                    let end = end.min(app.input.len());
                    app.input.replace_range(app.file_search_at_pos..end, &path);
                    app.cursor = app.file_search_at_pos + path.len();
                } else {
                    app.cursor = app.file_search_at_pos + 1 + app.file_search_query.len();
                }
                app.mode = app::InputMode::Normal;
                app.file_search_results.clear();
                KeyAction::Redraw
            }
            KeyCode::Backspace => {
                if app.file_search_query.is_empty() {
                    if app.file_search_at_pos < app.input.len() {
                        app.input.remove(app.file_search_at_pos);
                        app.cursor = app.file_search_at_pos;
                    }
                    app.mode = app::InputMode::Normal;
                    app.file_search_results.clear();
                } else {
                    app.file_search_query.pop();
                    let end = app.file_search_at_pos + 1 + app.file_search_query.len() + 1;
                    let end = end.min(app.input.len());
                    let new_text = format!("@{}", app.file_search_query);
                    app.input.replace_range(app.file_search_at_pos..end, &new_text);
                    app.cursor = app.file_search_at_pos + new_text.len();
                    let cwd = std::env::current_dir().unwrap_or_default();
                    app.file_search_results = decipher_tui::file_search::search_files(&cwd, &app.file_search_query);
                    app.file_search_index = 0;
                }
                KeyAction::Redraw
            }
            KeyCode::Char(c) => {
                app.file_search_query.push(c);
                let at_end = app.file_search_at_pos + 1 + app.file_search_query.len() - 1;
                let at_end = at_end.min(app.input.len());
                let new_text = format!("@{}", app.file_search_query);
                app.input.replace_range(app.file_search_at_pos..at_end, &new_text);
                app.cursor = app.file_search_at_pos + new_text.len();
                let cwd = std::env::current_dir().unwrap_or_default();
                app.file_search_results = decipher_tui::file_search::search_files(&cwd, &app.file_search_query);
                app.file_search_index = 0;
                KeyAction::Redraw
            }
            _ => KeyAction::None,
        },

        app::InputMode::CommandPopup => match key.code {
            KeyCode::Esc => { app.mode = app::InputMode::Normal; app.input.clear(); app.cursor = 0; KeyAction::Redraw }
            KeyCode::Up => { if app.popup_index > 0 { app.popup_index -= 1; } KeyAction::Redraw }
            KeyCode::Down => { let max = app.filtered_commands().len().saturating_sub(1); if app.popup_index < max { app.popup_index += 1; } KeyAction::Redraw }
            KeyCode::Enter | KeyCode::Tab => {
                let filtered = app.filtered_commands();
                if let Some(cmd) = filtered.get(app.popup_index) { app.input = cmd.name.clone(); app.cursor = app.input.len(); }
                app.mode = app::InputMode::Normal; app.popup_filter.clear(); app.popup_index = 0; KeyAction::Redraw
            }
            KeyCode::Char(c) => { app.popup_filter.push(c); app.popup_index = 0; app.input = format!("/{}", app.popup_filter); app.cursor = app.input.len(); KeyAction::Redraw }
            KeyCode::Backspace => {
                app.popup_filter.pop();
                if app.popup_filter.is_empty() { app.mode = app::InputMode::Normal; app.input.clear(); app.cursor = 0; }
                else { app.input = format!("/{}", app.popup_filter); app.cursor = app.input.len(); }
                KeyAction::Redraw
            }
            _ => KeyAction::None,
        },

        app::InputMode::Normal => {
            // Emacs editing keys (Ctrl+)
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('a') => { app.cursor = 0; return KeyAction::Redraw; }
                    KeyCode::Char('e') => { app.cursor = app.input.len(); return KeyAction::Redraw; }
                    KeyCode::Char('b') => { if app.cursor > 0 { app.cursor -= 1; } return KeyAction::Redraw; }
                    KeyCode::Char('f') => { if app.cursor < app.input.len() { app.cursor += 1; } return KeyAction::Redraw; }
                    KeyCode::Char('p') => { app.navigate_history(true); return KeyAction::Redraw; }
                    KeyCode::Char('n') => { app.navigate_history(false); return KeyAction::Redraw; }
                    KeyCode::Char('h') => { if app.cursor > 0 { app.cursor -= 1; app.input.remove(app.cursor); } return KeyAction::Redraw; }
                    KeyCode::Char('j') | KeyCode::Char('m') => { app.input.insert(app.cursor, '\n'); app.cursor += 1; return KeyAction::Redraw; }
                    KeyCode::Char('k') => { app.kill_to_end(); return KeyAction::Redraw; }
                    KeyCode::Char('u') => { app.kill_to_start(); return KeyAction::Redraw; }
                    KeyCode::Char('w') => { app.kill_word_backward(); return KeyAction::Redraw; }
                    KeyCode::Char('y') => { app.yank(); return KeyAction::Redraw; }
                    KeyCode::Char('r') => { app.enter_history_search(); return KeyAction::Redraw; }
                    KeyCode::Char('t') => { app.mode = app::InputMode::Pager; app.pager_scroll = 0; return KeyAction::Redraw; }
                    KeyCode::Char('x') => {
                        if let Some(result) = open_editor(&app.input) {
                            app.input = result;
                            app.cursor = app.input.len();
                        }
                        return KeyAction::Redraw;
                    }
                    KeyCode::Char('v') => {
                        if let Some(img) = decipher_clipboard::paste_image() {
                            app.session_image_count += 1;
                            let token = format!("[Image #{}]", app.session_image_count);
                            app.input.insert_str(app.cursor, &token);
                            app.cursor += token.len();
                            app.pending_images.push(img);
                            return KeyAction::Redraw;
                        }
                        return KeyAction::None;
                    }
                    _ => {}
                }
            }

            // Alt+ keybindings
            if key.modifiers.contains(KeyModifiers::ALT) {
                match key.code {
                    KeyCode::Left | KeyCode::Char('b') => { app.word_left(); return KeyAction::Redraw; }
                    KeyCode::Right | KeyCode::Char('f') => { app.word_right(); return KeyAction::Redraw; }
                    KeyCode::Char('d') | KeyCode::Delete => { app.kill_word_forward(); return KeyAction::Redraw; }
                    KeyCode::Backspace => { app.kill_word_backward(); return KeyAction::Redraw; }
                    _ => {}
                }
            }

            // Ctrl+Left/Right word navigation
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Left => { app.word_left(); return KeyAction::Redraw; }
                    KeyCode::Right => { app.word_right(); return KeyAction::Redraw; }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    app.input.insert(app.cursor, '\n'); app.cursor += 1; KeyAction::Redraw
                }
                KeyCode::Enter => {
                    if let Some(msg) = app.submit_input() { KeyAction::Submit(msg) } else { KeyAction::None }
                }
                KeyCode::Char('/') if app.input.is_empty() => {
                    app.mode = app::InputMode::CommandPopup; app.popup_filter.clear(); app.popup_index = 0;
                    app.input = "/".into(); app.cursor = 1; KeyAction::Redraw
                }
                KeyCode::Char('?') if app.input.is_empty() => {
                    app.show_shortcuts = !app.show_shortcuts; KeyAction::Redraw
                }
                KeyCode::Char('@') => {
                    app.file_search_at_pos = app.cursor;
                    app.input.insert(app.cursor, '@');
                    app.cursor += 1;
                    app.file_search_query.clear();
                    app.file_search_index = 0;
                    let cwd = std::env::current_dir().unwrap_or_default();
                    app.file_search_results = decipher_tui::file_search::search_files(&cwd, "");
                    app.mode = app::InputMode::FileSearch;
                    KeyAction::Redraw
                }
                KeyCode::Tab => {
                    if app.agent_busy {
                        if let Some(msg) = app.submit_input() {
                            app.queued_message = Some(msg);
                        }
                        KeyAction::Redraw
                    } else {
                        if let Some(msg) = app.submit_input() { KeyAction::Submit(msg) } else { KeyAction::None }
                    }
                }
                KeyCode::Char(c) => { app.show_shortcuts = false; app.input.insert(app.cursor, c); app.cursor += 1; KeyAction::Redraw }
                KeyCode::Backspace => { if app.cursor > 0 { app.cursor -= 1; app.input.remove(app.cursor); } KeyAction::Redraw }
                KeyCode::Delete => { if app.cursor < app.input.len() { app.input.remove(app.cursor); } KeyAction::Redraw }
                KeyCode::Left => { if app.cursor > 0 { app.cursor -= 1; } KeyAction::Redraw }
                KeyCode::Right => { if app.cursor < app.input.len() { app.cursor += 1; } KeyAction::Redraw }
                KeyCode::Home => { app.cursor = 0; KeyAction::Redraw }
                KeyCode::End => { app.cursor = app.input.len(); KeyAction::Redraw }
                KeyCode::Up => { app.navigate_history(true); KeyAction::Redraw }
                KeyCode::Down => { app.navigate_history(false); KeyAction::Redraw }
                _ => KeyAction::None,
            }
        }
    }
}

/// Open $EDITOR with the given text, return the edited result.
fn open_editor(initial_text: &str) -> Option<String> {
    use std::fs;
    use std::process::Command;

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("decipher-edit.tmp");
    if fs::write(&tmp_path, initial_text).is_err() {
        return None;
    }

    let _ = crossterm::terminal::disable_raw_mode();

    let status = Command::new(&editor)
        .arg(&tmp_path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    let _ = crossterm::terminal::enable_raw_mode();

    match status {
        Ok(s) if s.success() => {
            let result = fs::read_to_string(&tmp_path).ok()?;
            let _ = fs::remove_file(&tmp_path);
            let trimmed = result.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        }
        _ => {
            let _ = fs::remove_file(&tmp_path);
            None
        }
    }
}

/// Initialize MCP clients from `~/.decipher/mcp.json`.
///
/// Returns the list of discovered tools and an Arc-wrapped client list.
/// Servers that fail to connect are silently skipped.
async fn init_mcp_clients(
    decipher_home: &std::path::Path,
) -> (
    Vec<decipher_mcp::McpTool>,
    Option<std::sync::Arc<Vec<std::sync::Arc<tokio::sync::Mutex<McpClient>>>>>,
) {
    let cfg = McpConfig::load(decipher_home);
    if cfg.servers.is_empty() {
        return (Vec::new(), None);
    }

    let mut all_tools: Vec<decipher_mcp::McpTool> = Vec::new();
    let mut clients: Vec<std::sync::Arc<tokio::sync::Mutex<McpClient>>> = Vec::new();

    for server_cfg in &cfg.servers {
        match McpClient::connect(server_cfg).await {
            Ok(mut client) => {
                let tools = client.list_tools().await.unwrap_or_default();
                all_tools.extend(tools);
                clients.push(std::sync::Arc::new(tokio::sync::Mutex::new(client)));
            }
            Err(e) => {
                eprintln!("[mcp] failed to connect to '{}': {e}", server_cfg.name);
            }
        }
    }

    if clients.is_empty() {
        (all_tools, None)
    } else {
        (all_tools, Some(std::sync::Arc::new(clients)))
    }
}
