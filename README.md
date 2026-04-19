# DeCIpher

```
     /\_/\
    ( •ᴥ• )   DeCIpher v0.1.5
```

A mission-driven local execution agent for CI/deployment tasks. Give it a goal — it plans, executes, verifies, and adapts until the job is done.

## Install

Requires **Node.js 22+**. Check your version:

```bash
node -v   # Should show v22.x.x or higher
```

If you don't have Node.js or need to upgrade:

```bash
# macOS / Linux (via nvm — recommended)
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
nvm install 22
nvm use 22

# Or via Homebrew (macOS)
brew install node@22

# Windows — download the installer from:
# https://nodejs.org/en/download
# Or via winget:
winget install OpenJS.NodeJS.LTS
```

Then install DeCIpher:

```bash
npm install -g decipher-cli
decipher
```

That's it. On first run, DeCIpher will ask you to configure an AI provider.

### Update

DeCIpher checks for new versions automatically and notifies you on startup. To update:

```bash
npm update -g decipher-cli
```

Optional: **Docker** (for container missions).

## Usage

```bash
# Start interactive mode
decipher

# Check environment
decipher doctor
```

### Interactive Mode

The terminal UI is built in Rust (crossterm) for smooth input handling, multi-line paste, and streaming output. It communicates with the Node.js agent backend via JSON over stdin/stdout.

```
    /\_/\
   ( •ᴥ• )  DeCIpher v0.1.5

  provider   openai
  model      gpt-4o
  directory  ~/my-project
  approval   on-request  last: idle
  api key    ● configured

  Type a mission, paste a path, or /help for commands.
  ctrl+r history  ctrl+c quit

│ ❯ Fix this Docker build failure

┌ MISSION ────────────────────────────────────────────────
  Understood: Fix this Docker build failure

  Plan:
    1. Inspect target
    2. Execute action
    3. Verify result

  ✓ exec_command — Clone the repository (6.4s)
  ✓ read_file — Read Dockerfile (2.1s)
  ✗ exec_command — Docker build failed (12.2s)

────────────────────────────────────────────────
  [RESULT]
  Outcome:     PASS (42.3s)
  Turns:       8
  Summary:     Fixed COPY path and rebuilt successfully.
────────────────────────────────────────────────
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Submit input |
| `Shift+Enter` | Insert newline (multi-line input) |
| `/` | Open command palette |
| `Ctrl+C` | Interrupt agent / quit |
| `Ctrl+D` | Quit (on empty input) |
| `Ctrl+A` / `Ctrl+E` | Jump to start / end of line |
| `Ctrl+K` / `Ctrl+U` | Kill to end / start of line |
| `Ctrl+W` | Kill word backward |
| `Ctrl+Y` | Yank (paste killed text) |
| `Alt+Left` / `Alt+Right` | Word-by-word cursor movement |
| `Up` / `Down` | Input history navigation |

### Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/model [name]` | Show or change AI model |
| `/setting show\|set` | Manage configuration |
| `/status` | Current session snapshot |
| `/plan` | Current mission plan |
| `/review` | Review patch before write-back |
| `/resume` | Resume last interrupted mission |
| `/transcript` | Show command transcript |
| `/artifacts` | Show saved artifacts |
| `/doctor` | Check environment |
| `/agents` | List available agents |
| `/quit` | Exit |

## Configuration

```bash
# OpenAI (default)
decipher setting set provider openai
decipher setting set api-key sk-xxx

# Anthropic
decipher setting set provider anthropic
decipher setting set api-key sk-ant-xxx

# Custom OpenAI-compatible (DeepSeek, Ollama, etc.)
decipher setting set provider custom
decipher setting set base_url https://your-api.com/v1/chat/completions
decipher setting set model your-model
decipher setting set api-key your-key
```

Config stored at `~/.decipher/config.json`.

## How It Works

```
User Goal
   → Mission Planner (understand + decompose)
   → Execution Loop (commands, file generation, repair)
   → Verification Layer (check results)
   → Adapt or Complete
```

DeCIpher stops early with `NEEDS_HUMAN_REVIEW` when confidence is low, the same fix is attempted twice, or changes touch too many files.

## Architecture

```
Rust TUI (crossterm)          Node.js Agent Backend
┌──────────────────┐          ┌──────────────────────┐
│ Input handling    │  JSON    │ Mission planner      │
│ Prompt rendering  │◄────────►│ Execution loop       │
│ Markdown output   │ stdin/   │ Tool runner          │
│ Approval flow     │ stdout   │ Verification layer   │
│ Streaming display │          │ LLM API abstraction  │
└──────────────────┘          └──────────────────────┘
```

## Tech Stack

- **TUI**: Rust + crossterm (inline rendering, no alternate screen)
- **Agent**: Node.js 22+ (ESM)
- **Tests**: `node:test` (built-in)
- **Colors**: `picocolors` (Node.js), ANSI RGB (Rust)
- **API**: Native `fetch`, OpenAI/Anthropic/custom providers

## License

MIT
