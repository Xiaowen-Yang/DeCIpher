//! DeCIpher CLI — entry point.
//!
//! Spawns the Node.js agent via agent-bridge, sets up the terminal,
//! and runs the TUI event loop.

use std::io;
use std::time::{Duration, Instant};


use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste,
        EnableFocusChange, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};

use decipher_agent_bridge::AgentBridge;
use decipher_protocol::{ClientMessage, ServerMessage};
use decipher_tui::app::{self, App};
use decipher_tui::render;

const TICK_RATE: Duration = Duration::from_millis(80);

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

async fn run_app(
    stdout: &mut io::Stdout,
    bridge: &mut AgentBridge,
) -> io::Result<()> {
    let mut app = App::new();
    let mut need_prompt_redraw = true;

    loop {
        if need_prompt_redraw {
            render::draw_prompt(stdout, &mut app)?;
            need_prompt_redraw = false;
        }

        tokio::select! {
            biased;

            result = poll_terminal_event() => {
                match result? {
                    Some(Event::Key(key)) => {
                        if key.kind != KeyEventKind::Press { continue; }
                        let action = handle_key(&mut app, key);
                        match action {
                            KeyAction::Redraw => { need_prompt_redraw = true; }
                            KeyAction::Submit(msg) => {
                                render::clear_prompt(stdout, &mut app)?;
                                render::print_user_input(stdout, &app.last_submitted)?;
                                need_prompt_redraw = true;
                                bridge.send(&msg).await?;
                            }
                            KeyAction::None => {}
                        }
                    }
                    Some(Event::Paste(text)) => {
                        app.input.insert_str(app.cursor, &text);
                        app.cursor += text.len();
                        need_prompt_redraw = true;
                    }
                    Some(Event::Resize(_, _)) => { need_prompt_redraw = true; }
                    Some(Event::FocusGained) => { app.terminal_focused = true; }
                    Some(Event::FocusLost) => { app.terminal_focused = false; }
                    _ => {}
                }
            }

            Some(msg) = bridge.rx.recv() => {
                match &msg {
                    ServerMessage::AgentMessageDelta { delta } => {
                        app.stream.push(delta);
                        render::commit_delta_lines(stdout, &mut app)?;
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
                        app.handle_server_message(msg);
                        need_prompt_redraw = true;
                    }
                }
            }

            _ = tokio::time::sleep(TICK_RATE) => {
                if app.stream.active {
                    render::commit_delta_lines(stdout, &mut app)?;
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

    match app.mode {
        app::InputMode::ApprovalPending => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                KeyAction::Submit(app.respond_approval(true))
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
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

async fn poll_terminal_event() -> io::Result<Option<Event>> {
    tokio::task::spawn_blocking(move || {
        if event::poll(TICK_RATE)? { Ok(Some(event::read()?)) } else { Ok(None) }
    }).await.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
}
