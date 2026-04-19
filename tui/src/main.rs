//! DeCIpher TUI — Rust terminal frontend.
//!
//! Renders inline (no alternate screen) to match the Node.js UI exactly.
//! Uses crossterm for raw mode + bracketed paste, prints to stdout directly.
//! Chat history scrolls naturally in the terminal scrollback.

mod app;
mod clipboard;
mod markdown;
mod protocol;
mod render;

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode,
        KeyEvent, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use std::process::Stdio;

use app::App;
use protocol::{ClientMessage, ServerMessage};

const TICK_RATE: Duration = Duration::from_millis(80);

#[tokio::main]
async fn main() -> io::Result<()> {
    let bin_path = find_agent_script();

    // Spawn Node.js agent in server mode
    let mut child = Command::new("node")
        .arg(&bin_path)
        .arg("--server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            eprintln!("Failed to start agent: {e}\nTried: node {bin_path}");
            io::Error::new(io::ErrorKind::Other, format!("Failed to start agent: {e}"))
        })?;

    let child_stdin = child.stdin.take().expect("child stdin");
    let child_stdout = child.stdout.take().expect("child stdout");
    let child_stderr = child.stderr.take().expect("child stderr");

    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<ServerMessage>();

    let tx1 = agent_tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(child_stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<ServerMessage>(&line) {
                Ok(msg) => { let _ = tx1.send(msg); }
                Err(_) => { let _ = tx1.send(ServerMessage::AgentMessage { text: line }); }
            }
        }
    });

    tokio::spawn(async move {
        let mut lines = BufReader::new(child_stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = agent_tx.send(ServerMessage::Error { message: line });
        }
    });

    // Raw mode + bracketed paste. NO alternate screen.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnableBracketedPaste)?;
    // Do NOT enable keyboard enhancement flags — they cause double key events
    // on many terminals (Press + Release both fire).

    let result = run_app(&mut stdout, child_stdin, &mut agent_rx).await;

    disable_raw_mode()?;
    execute!(stdout, DisableBracketedPaste)?;
    // Print final newline so shell prompt appears on new line
    println!();

    let _ = child.kill().await;
    result
}

async fn run_app(
    stdout: &mut io::Stdout,
    mut child_stdin: tokio::process::ChildStdin,
    agent_rx: &mut mpsc::UnboundedReceiver<ServerMessage>,
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
                        // CRITICAL: Only handle Press events.
                        // Without this filter, characters appear twice.
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        let action = handle_key(&mut app, key);
                        match action {
                            KeyAction::Redraw => {
                                need_prompt_redraw = true;
                            }
                            KeyAction::Submit(msg) => {
                                render::clear_prompt(stdout, &mut app)?;
                                render::print_user_input(stdout, &app.last_submitted)?;
                                need_prompt_redraw = true;
                                send_message(&mut child_stdin, &msg).await?;
                            }
                            KeyAction::None => {}
                        }
                    }
                    Some(Event::Paste(text)) => {
                        app.input.insert_str(app.cursor, &text);
                        app.cursor += text.len();
                        need_prompt_redraw = true;
                    }
                    Some(Event::Resize(_, _)) => {
                        need_prompt_redraw = true;
                    }
                    _ => {}
                }
            }

            Some(msg) = agent_rx.recv() => {
                match &msg {
                    ServerMessage::AgentMessageDelta { delta } => {
                        // Streaming delta: render inline without redrawing prompt
                        if need_prompt_redraw {
                            // First delta after prompt was shown — clear prompt first
                            render::clear_prompt(stdout, &mut app)?;
                            use std::io::Write;
                            write!(stdout, "  ")?; // indent to match agent message style
                            need_prompt_redraw = false;
                        }
                        render::print_delta(stdout, delta)?;
                        app.handle_server_message(msg);
                    }
                    _ => {
                        render::clear_prompt(stdout, &mut app)?;
                        render::print_server_message(stdout, &msg, &mut app)?;
                        app.handle_server_message(msg);
                        need_prompt_redraw = true;
                    }
                }
            }

            _ = tokio::time::sleep(TICK_RATE) => {
                if app.spinner_label.is_some() {
                    app.spinner_frame += 1;
                    // Spinner updates happen in the prompt area
                    need_prompt_redraw = true;
                }
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
    // ── Ctrl+C: interrupt agent or quit ─────────────────────────────────
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if app.agent_busy {
            // First Ctrl+C while agent is busy → interrupt
            app.agent_busy = false;
            app.spinner_label = None;
            return KeyAction::Submit(ClientMessage::Interrupt);
        }
        // Double Ctrl+C within 1s → force quit
        let now = Instant::now();
        if let Some(last) = app.last_ctrl_c {
            if now.duration_since(last) < Duration::from_secs(1) {
                app.should_quit = true;
                return KeyAction::Redraw;
            }
        }
        app.last_ctrl_c = Some(now);
        // Single Ctrl+C when idle → clear input or signal quit intent
        if !app.input.is_empty() {
            app.input.clear();
            app.cursor = 0;
            return KeyAction::Redraw;
        }
        app.should_quit = true;
        return KeyAction::Redraw;
    }

    // ── Ctrl+D: quit on empty, forward-delete otherwise ─────────────────
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

        app::InputMode::CommandPopup => match key.code {
            KeyCode::Esc => {
                app.mode = app::InputMode::Normal;
                app.input.clear();
                app.cursor = 0;
                KeyAction::Redraw
            }
            KeyCode::Up => {
                if app.popup_index > 0 { app.popup_index -= 1; }
                KeyAction::Redraw
            }
            KeyCode::Down => {
                let max = app.filtered_commands().len().saturating_sub(1);
                if app.popup_index < max { app.popup_index += 1; }
                KeyAction::Redraw
            }
            KeyCode::Enter | KeyCode::Tab => {
                let filtered = app.filtered_commands();
                if let Some(cmd) = filtered.get(app.popup_index) {
                    app.input = cmd.name.clone();
                    app.cursor = app.input.len();
                }
                app.mode = app::InputMode::Normal;
                app.popup_filter.clear();
                app.popup_index = 0;
                KeyAction::Redraw
            }
            KeyCode::Char(c) => {
                app.popup_filter.push(c);
                app.popup_index = 0;
                app.input = format!("/{}", app.popup_filter);
                app.cursor = app.input.len();
                KeyAction::Redraw
            }
            KeyCode::Backspace => {
                app.popup_filter.pop();
                if app.popup_filter.is_empty() {
                    app.mode = app::InputMode::Normal;
                    app.input.clear();
                    app.cursor = 0;
                } else {
                    app.input = format!("/{}", app.popup_filter);
                    app.cursor = app.input.len();
                }
                KeyAction::Redraw
            }
            _ => KeyAction::None,
        },

        app::InputMode::Normal => {
            // ── Emacs editing keys (Ctrl+*) ─────────────────────────────
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('a') => { app.cursor = 0; return KeyAction::Redraw; }
                    KeyCode::Char('e') => { app.cursor = app.input.len(); return KeyAction::Redraw; }
                    KeyCode::Char('k') => { app.kill_to_end(); return KeyAction::Redraw; }
                    KeyCode::Char('u') => { app.kill_to_start(); return KeyAction::Redraw; }
                    KeyCode::Char('w') => { app.kill_word_backward(); return KeyAction::Redraw; }
                    KeyCode::Char('y') => { app.yank(); return KeyAction::Redraw; }
                    KeyCode::Char('v') => {
                        if let Some(img) = clipboard::paste_image() {
                            app.last_submitted = "[Image pasted from clipboard]".into();
                            let text = app.input.trim().to_string();
                            app.input.clear();
                            app.cursor = 0;
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

            // Alt+Arrow for word navigation
            if key.modifiers.contains(KeyModifiers::ALT) {
                match key.code {
                    KeyCode::Left => { app.word_left(); return KeyAction::Redraw; }
                    KeyCode::Right => { app.word_right(); return KeyAction::Redraw; }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    // Shift+Enter inserts a newline (multi-line input)
                    app.input.insert(app.cursor, '\n');
                    app.cursor += 1;
                    KeyAction::Redraw
                }
                KeyCode::Enter => {
                    if let Some(msg) = app.submit_input() {
                        KeyAction::Submit(msg)
                    } else {
                        KeyAction::None
                    }
                }
                KeyCode::Char('/') if app.input.is_empty() => {
                    app.mode = app::InputMode::CommandPopup;
                    app.popup_filter.clear();
                    app.popup_index = 0;
                    app.input = "/".into();
                    app.cursor = 1;
                    KeyAction::Redraw
                }
                KeyCode::Char(c) => {
                    app.input.insert(app.cursor, c);
                    app.cursor += 1;
                    KeyAction::Redraw
                }
                KeyCode::Backspace => {
                    if app.cursor > 0 {
                        app.cursor -= 1;
                        app.input.remove(app.cursor);
                    }
                    KeyAction::Redraw
                }
                KeyCode::Delete => {
                    if app.cursor < app.input.len() { app.input.remove(app.cursor); }
                    KeyAction::Redraw
                }
                KeyCode::Left => {
                    if app.cursor > 0 { app.cursor -= 1; }
                    KeyAction::Redraw
                }
                KeyCode::Right => {
                    if app.cursor < app.input.len() { app.cursor += 1; }
                    KeyAction::Redraw
                }
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
        if event::poll(TICK_RATE)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
}

async fn send_message(
    stdin: &mut tokio::process::ChildStdin,
    msg: &ClientMessage,
) -> io::Result<()> {
    let json = serde_json::to_string(msg).unwrap();
    stdin.write_all(json.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

fn find_agent_script() -> String {
    if let Ok(path) = std::env::var("DECIPHER_AGENT_SCRIPT") {
        if PathBuf::from(&path).exists() { return path; }
    }
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let c = dir.join("decipher");
        if c.exists() { return c.to_string_lossy().to_string(); }
        let c2 = dir.join("../../bin/decipher");
        if c2.exists() { return c2.canonicalize().unwrap_or(c2).to_string_lossy().to_string(); }
    }
    let c = PathBuf::from("bin/decipher");
    if c.exists() { return c.canonicalize().unwrap_or(c).to_string_lossy().to_string(); }
    "bin/decipher".to_string()
}
