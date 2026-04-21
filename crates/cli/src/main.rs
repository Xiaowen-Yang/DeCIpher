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
    event::{
        DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste,
        EnableFocusChange, Event, EventStream, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    Terminal, TerminalOptions, Viewport,
};
use tokio::sync::mpsc;

use decipher_protocol::{ClientMessage, ServerMessage};
use decipher_providers::anthropic::AnthropicProvider;
use decipher_runtime::{AgentConfig, AgentLoop};
use decipher_session_store::{load_session, list_sessions, SessionStore};
use decipher_tui::app::{self, App};
use decipher_tui::bottom_pane::{self, BottomPane};

mod config;

/// Tick rate: 32ms ≈ 31.25 FPS — smooth spinner animation matching Codex.
const TICK_RATE: Duration = Duration::from_millis(32);

/// Minimum interval between draws: 120 FPS cap (~8.3ms).
const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_334);

/// Create a fresh ratatui Terminal with an inline viewport of the given height.
fn make_terminal(height: u16) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli_cfg = config::CliConfig::load();

    // Push cursor to the terminal bottom BEFORE raw mode so that
    // Viewport::Inline anchors at the real bottom, not wherever the
    // shell prompt happened to leave the cursor.
    {
        let (_, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        print!("{}", "\n".repeat(rows as usize));
        io::stdout().flush()?;
    }

    // Raw mode + bracketed paste + focus detection. NO alternate screen.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnableBracketedPaste, EnableFocusChange)?;

    // Keyboard enhancement flags for better key detection (Kitty protocol).
    let has_keyboard_enhancement = crossterm::terminal::supports_keyboard_enhancement()
        .unwrap_or(false);
    if has_keyboard_enhancement {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )?;
    }

    let result = run_app(cli_cfg).await;

    let mut stdout = io::stdout();
    if has_keyboard_enhancement {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
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
async fn run_app(cli_cfg: config::CliConfig) -> io::Result<()> {
    // ── Provider ─────────────────────────────────────────────────────────────
    let provider = {
        let mut p = AnthropicProvider::new(&cli_cfg.api_key, &cli_cfg.model);
        if let Some(ref url) = cli_cfg.base_url {
            p = p.with_base_url(url);
        }
        Arc::new(p)
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
    let model = cli_cfg.model.clone();
    let base_url = cli_cfg.base_url.clone();
    let policy_mode = cli_cfg.policy_mode;

    // ── Session store ─────────────────────────────────────────────────────────
    // No Drop impl on SessionStore: session_end is intentionally best-effort —
    // a crash will leave the JSONL without a final record, which is acceptable.
    let decipher_home = config::decipher_home();
    // Session store is created per-mission (when UserInput is submitted) so that
    // mission_goal is known at creation time.
    let mut session_store: Option<SessionStore> = None;
    let mut last_outcome: Option<String> = None;

    // ── TUI state ─────────────────────────────────────────────────────────────
    let mut app = App::new();
    let mut need_redraw = true;
    let mut fps = FrameRateLimiter::new();
    let mut events = EventStream::new();

    // Fixed viewport height — never recreated mid-session (see original comment).
    let viewport_height = bottom_pane::MAX_PANE_HEIGHT;
    let mut terminal = make_terminal(viewport_height)?;
    let actual_width = terminal.size()?.width;
    if actual_width != app.chat.width() {
        app.chat.set_width(actual_width);
    }

    loop {
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
                                // insert_before for scrollback (mirrors previous bridge behavior)
                                let lines = bottom_pane::user_input_lines(&app.last_submitted);
                                let height = lines.len() as u16;
                                terminal.insert_before(height, |buf| {
                                    bottom_pane::render_lines_to_buffer(buf, &lines);
                                })?;
                                fps.mark_drawn();
                                need_redraw = true;

                                match msg {
                                    ClientMessage::UserInput { ref text, .. } => {
                                        if let Some(h) = current_agent_task.take() {
                                            h.abort();
                                        }

                                        let text_trimmed = text.trim();
                                        if text_trimmed.starts_with("/resume") {
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

                                                        let (atx, arx) = mpsc::channel::<bool>(1);
                                                        current_approval_tx = Some(atx);
                                                        let resume_cfg = AgentConfig {
                                                            model: model.clone(),
                                                            api_key: api_key.clone(),
                                                            base_url: base_url.clone(),
                                                            workspace: meta.workspace.clone(),
                                                            mission_goal: meta.mission_goal.clone(),
                                                            policy_mode,
                                                            resume_from: Some(history),
                                                            ..Default::default()
                                                        };
                                                        let tx = event_tx.clone();
                                                        let prov = provider.clone();
                                                        current_agent_task = Some(tokio::spawn(async move {
                                                            let _ = AgentLoop::run(resume_cfg, &*prov, tx, Some(arx)).await;
                                                        }));
                                                    }
                                                },
                                            }
                                        } else {
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
                                            let agent_cfg = build_agent_config(
                                                text.clone(),
                                                api_key.clone(),
                                                model.clone(),
                                                base_url.clone(),
                                                workspace.clone(),
                                                policy_mode,
                                            );
                                            let tx = event_tx.clone();
                                            let prov = provider.clone();
                                            current_agent_task = Some(tokio::spawn(async move {
                                                let _ = AgentLoop::run(agent_cfg, &*prov, tx, Some(arx)).await;
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
                        app.chat.set_width(cols);
                        let vp_h = viewport_height.min(rows);
                        terminal.resize(ratatui::layout::Rect::new(
                            0,
                            rows.saturating_sub(vp_h),
                            cols,
                            vp_h,
                        ))?;
                        need_redraw = true;
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
                    let banner = bottom_pane::banner_lines(version, provider, model, directory, api_key_set);
                    if !banner.is_empty() {
                        let h = banner.len() as u16;
                        terminal.insert_before(h, |buf| {
                            bottom_pane::render_lines_to_buffer(buf, &banner);
                        })?;
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

                // Route through ChatWidget → typed cells → scrollback lines.
                let scrollback_lines = app.chat.handle_server_message(&msg);
                if !scrollback_lines.is_empty() {
                    let h = scrollback_lines.len() as u16;
                    terminal.insert_before(h, |buf| {
                        bottom_pane::render_lines_to_buffer(buf, &scrollback_lines);
                    })?;
                }

                // Record to session JSONL before msg is consumed.
                if let Some(ref ss) = session_store {
                    ss.record(&msg);
                }
                if let ServerMessage::MissionComplete { ref outcome, .. } = msg {
                    last_outcome = Some(outcome.clone());
                }

                fps.mark_drawn();
                app.handle_server_message(msg);
                need_redraw = true;

                // Auto-approve if user pressed 'a' (always) earlier.
                if app.always_approve && app.mode == app::InputMode::ApprovalPending {
                    app.respond_approval(true);
                    if let Some(atx) = &current_approval_tx {
                        let _ = atx.try_send(true);
                    }
                }
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
                    let height = lines.len() as u16;
                    terminal.insert_before(height, |buf| {
                        bottom_pane::render_lines_to_buffer(buf, &lines);
                    })?;
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
                    let agent_cfg = build_agent_config(
                        text.clone(),
                        api_key.clone(),
                        model.clone(),
                        base_url.clone(),
                        workspace.clone(),
                        policy_mode,
                    );
                    let tx = event_tx.clone();
                    let prov = provider.clone();
                    current_agent_task = Some(tokio::spawn(async move {
                        let _ = AgentLoop::run(agent_cfg, &*prov, tx, Some(arx)).await;
                    }));
                }
            }
        }

        if app.should_quit {
            // Abort agent task if running.
            if let Some(h) = current_agent_task.take() {
                h.abort();
            }
            // Flush and close the session JSONL.
            if let Some(ss) = session_store.take() {
                ss.close(last_outcome.clone()).await;
            }
            let lines = bottom_pane::goodbye_lines();
            let height = lines.len() as u16;
            terminal.insert_before(height, |buf| {
                bottom_pane::render_lines_to_buffer(buf, &lines);
            })?;
            break;
        }
    }

    Ok(())
}

/// Build an `AgentConfig` for a single mission run.
fn build_agent_config(
    goal: String,
    api_key: String,
    model: String,
    base_url: Option<String>,
    workspace: String,
    policy_mode: decipher_policy::PolicyMode,
) -> AgentConfig {
    AgentConfig {
        model,
        api_key,
        base_url,
        workspace,
        mission_goal: goal,
        policy_mode,
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
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                KeyAction::Submit(app.respond_approval(true))
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.always_approve = true;
                KeyAction::Submit(app.respond_approval(true))
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                KeyAction::Submit(app.respond_approval(false))
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
