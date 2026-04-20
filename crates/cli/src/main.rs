//! DeCIpher CLI — entry point.
//!
//! Spawns the Node.js agent via agent-bridge, sets up the terminal,
//! and runs the TUI event loop.
//!
//! Key design choices (Codex parity):
//! - crossterm `EventStream` for truly async event reading (no blocking poll)
//! - 32ms tick rate (~30 FPS) for smooth spinner animation
//! - Frame rate limiter (120 FPS cap) to prevent redundant redraws
//! - Biased select: terminal events > server messages > tick

use std::io;
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

use decipher_agent_bridge::AgentBridge;
use decipher_protocol::{ClientMessage, ServerMessage};
use decipher_tui::app::{self, App};
use decipher_tui::render;

/// Tick rate: 32ms ≈ 31.25 FPS — smooth spinner animation matching Codex.
const TICK_RATE: Duration = Duration::from_millis(32);

/// Minimum interval between draws: 120 FPS cap (~8.3ms).
const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_334);

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut bridge = AgentBridge::spawn().await?;

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

    let result = run_app(&mut stdout, &mut bridge).await;

    if has_keyboard_enhancement {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(stdout, DisableBracketedPaste, DisableFocusChange)?;
    println!();

    bridge.shutdown().await;
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

    /// Returns true if enough time has passed since the last draw.
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

    /// Force mark a draw happened (used when we must draw regardless).
    fn mark_drawn(&mut self) {
        self.last_draw = Some(Instant::now());
    }
}

async fn run_app(
    stdout: &mut io::Stdout,
    bridge: &mut AgentBridge,
) -> io::Result<()> {
    let mut app = App::new();
    let mut need_prompt_redraw = true;
    let mut fps = FrameRateLimiter::new();

    // Async event stream — no blocking poll, no spawn_blocking overhead.
    let mut events = EventStream::new();

    loop {
        if need_prompt_redraw && fps.should_draw() {
            render::draw_prompt(stdout, &mut app)?;
            need_prompt_redraw = false;
        }

        tokio::select! {
            biased;

            // Terminal events — truly async, resolves immediately on event.
            Some(result) = events.next() => {
                match result {
                    Ok(Event::Key(key)) => {
                        if key.kind != KeyEventKind::Press { continue; }
                        let action = handle_key(&mut app, key);
                        match action {
                            KeyAction::Redraw => { need_prompt_redraw = true; }
                            KeyAction::Submit(msg) => {
                                render::clear_prompt(stdout, &mut app)?;
                                render::print_user_input(stdout, &app.last_submitted)?;
                                fps.mark_drawn();
                                need_prompt_redraw = true;
                                bridge.send(&msg).await?;
                            }
                            KeyAction::None => {}
                        }
                    }
                    Ok(Event::Paste(text)) => {
                        app.input.insert_str(app.cursor, &text);
                        app.cursor += text.len();
                        need_prompt_redraw = true;
                    }
                    Ok(Event::Resize(_, _)) => { need_prompt_redraw = true; }
                    Ok(Event::FocusGained) => { app.terminal_focused = true; }
                    Ok(Event::FocusLost) => { app.terminal_focused = false; }
                    Ok(_) => {}
                    Err(_) => { break; }
                }
            }

            // Server messages from the Node.js agent.
            Some(msg) = bridge.rx.recv() => {
                match &msg {
                    ServerMessage::AgentMessageDelta { delta } => {
                        app.stream.push(delta);
                        render::commit_delta_lines(stdout, &mut app)?;
                        fps.mark_drawn();
                        app.handle_server_message(msg);
                    }
                    _ => {
                        if app.stream.active {
                            render::flush_delta_buffer(stdout, &mut app)?;
                        }
                        // Set terminal title when we receive the banner
                        if let ServerMessage::Banner { ref model, .. } = msg {
                            render::set_terminal_title(stdout, &format!("DeCIpher — {model}"))?;
                        }
                        // Send notification on mission complete when terminal unfocused
                        if let ServerMessage::MissionComplete { ref outcome, ref summary, .. } = msg {
                            if !app.terminal_focused {
                                let _ = render::send_notification(stdout, &format!("DeCIpher: {outcome} — {summary}"));
                            }
                        }
                        render::clear_prompt(stdout, &mut app)?;
                        render::print_server_message(stdout, &msg, &mut app)?;
                        fps.mark_drawn();
                        app.handle_server_message(msg);
                        need_prompt_redraw = true;
                    }
                }
            }

            // Tick timer — fires every 32ms for smooth animation.
            _ = tokio::time::sleep(TICK_RATE) => {
                if app.stream.active {
                    if render::commit_delta_lines(stdout, &mut app)? {
                        fps.mark_drawn();
                    }
                }
                if app.spinner_label.is_some() || app.stream.active {
                    app.spinner_frame += 1;
                    need_prompt_redraw = true;
                }
            }
        }

        // Dispatch queued message when agent becomes idle
        if !app.agent_busy {
            if let Some(queued) = app.queued_message.take() {
                render::clear_prompt(stdout, &mut app)?;
                render::print_user_input(stdout, &app.last_submitted)?;
                fps.mark_drawn();
                need_prompt_redraw = true;
                bridge.send(&queued).await?;
            }
        }

        if app.should_quit {
            render::clear_prompt(stdout, &mut app)?;
            render::print_goodbye(stdout)?;
            break;
        }
    }

    Ok(())
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
            app.stream.reset();
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
        if !app.input.is_empty() {
            app.input.clear();
            app.cursor = 0;
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
            // Send SIGTSTP to self — terminal will suspend
            libc::raise(libc::SIGTSTP);
        }
        // After resume: terminal state is restored by crossterm
        return KeyAction::Redraw;
    }

    match app.mode {
        app::InputMode::ApprovalPending => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                KeyAction::Submit(app.respond_approval(true))
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Always approve for this session
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
                    app.search_match_index = None; // reset to search from end
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
                app.pager_scroll = usize::MAX; // will be clamped in renderer
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
                // Cancel: remove the @ and query from input
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
                // Accept selection: replace @query with the file path
                if let Some(result) = app.file_search_results.get(app.file_search_index) {
                    let path = result.path.clone();
                    let end = app.file_search_at_pos + 1 + app.file_search_query.len();
                    let end = end.min(app.input.len());
                    app.input.replace_range(app.file_search_at_pos..end, &path);
                    app.cursor = app.file_search_at_pos + path.len();
                } else {
                    // No result, just keep the @query
                    app.cursor = app.file_search_at_pos + 1 + app.file_search_query.len();
                }
                app.mode = app::InputMode::Normal;
                app.file_search_results.clear();
                KeyAction::Redraw
            }
            KeyCode::Backspace => {
                if app.file_search_query.is_empty() {
                    // Remove the @ itself
                    if app.file_search_at_pos < app.input.len() {
                        app.input.remove(app.file_search_at_pos);
                        app.cursor = app.file_search_at_pos;
                    }
                    app.mode = app::InputMode::Normal;
                    app.file_search_results.clear();
                } else {
                    app.file_search_query.pop();
                    // Update input
                    let end = app.file_search_at_pos + 1 + app.file_search_query.len() + 1;
                    let end = end.min(app.input.len());
                    let new_text = format!("@{}", app.file_search_query);
                    app.input.replace_range(app.file_search_at_pos..end, &new_text);
                    app.cursor = app.file_search_at_pos + new_text.len();
                    // Re-search
                    let cwd = std::env::current_dir().unwrap_or_default();
                    app.file_search_results = decipher_tui::file_search::search_files(&cwd, &app.file_search_query);
                    app.file_search_index = 0;
                }
                KeyAction::Redraw
            }
            KeyCode::Char(c) => {
                app.file_search_query.push(c);
                // Update input text
                let at_end = app.file_search_at_pos + 1 + app.file_search_query.len() - 1;
                let at_end = at_end.min(app.input.len());
                let new_text = format!("@{}", app.file_search_query);
                app.input.replace_range(app.file_search_at_pos..at_end, &new_text);
                app.cursor = app.file_search_at_pos + new_text.len();
                // Re-search
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
                        // Ctrl+X: open $EDITOR with current input
                        if let Some(result) = open_editor(&app.input) {
                            app.input = result;
                            app.cursor = app.input.len();
                        }
                        return KeyAction::Redraw;
                    }
                    KeyCode::Char('v') => {
                        if let Some(img) = decipher_clipboard::paste_image() {
                            app.last_submitted = "[Image pasted from clipboard]".into();
                            let text = app.input.trim().to_string();
                            app.input.clear(); app.cursor = 0;
                            return KeyAction::Submit(ClientMessage::UserInput {
                                text: if text.is_empty() { "Analyze this image".into() } else { text },
                                images: vec![img],
                            });
                        }
                        return KeyAction::None;
                    }
                    _ => {}
                }
            }

            // Alt+ keybindings (word navigation, deletion)
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
                    // Enter file search mode
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
                    // Tab: submit if idle, queue if busy
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
/// Temporarily exits raw mode so the editor can function normally.
fn open_editor(initial_text: &str) -> Option<String> {
    use std::fs;
    use std::process::Command;

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    // Write current input to a temp file
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("decipher-edit.tmp");
    if fs::write(&tmp_path, initial_text).is_err() {
        return None;
    }

    // Exit raw mode so the editor works
    let _ = crossterm::terminal::disable_raw_mode();

    // Spawn editor
    let status = Command::new(&editor)
        .arg(&tmp_path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    // Re-enter raw mode
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
