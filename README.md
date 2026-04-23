# DeCIpher

```
     /\_/\
    ( •ᴥ• )   DeCIpher v0.1.5
```

A mission-driven local execution agent for CI/deployment tasks. Give it a goal — it plans, executes, verifies, and adapts until the job is done.

---

## Quick Start (from source)

Follow these steps in order. Each step explains what it is and why you need it.

### Step 0: Open a Terminal

- **macOS**: press `Cmd + Space`, type "Terminal", press Enter.
- **Windows**: press `Win + R`, type `cmd`, press Enter. Or search for "PowerShell".
- **Linux**: press `Ctrl + Alt + T`.

You will type (or paste) the commands below into this window, then press Enter to run them.

### Step 1: Install Node.js (v22 or newer)

Node.js is a program that runs JavaScript code. DeCIpher's startup script needs it.

**Check if you already have it:**

```bash
node -v
```

If the output shows `v22.x.x` or higher, skip to Step 2. If it says "command not found" or shows a lower version, install it:

<details>
<summary><strong>macOS</strong></summary>

Option A — Homebrew (if you have it):
```bash
brew install node@22
```

Option B — nvm (version manager):
```bash
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
```
Close and reopen your terminal, then:
```bash
nvm install 22
```
</details>

<details>
<summary><strong>Windows</strong></summary>

Download the installer from https://nodejs.org/en/download and run it.
Or, if you have winget:
```bash
winget install OpenJS.NodeJS.LTS
```
</details>

<details>
<summary><strong>Linux</strong></summary>

```bash
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
```
Close and reopen your terminal, then:
```bash
nvm install 22
```
</details>

### Step 2: Install Rust

Rust is a programming language. DeCIpher's terminal interface (TUI) is written in Rust, so you need the Rust compiler to build it.

**Check if you already have it:**

```bash
rustc --version
```

If it prints a version number, skip to Step 3. Otherwise, install it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When prompted, choose the default installation (press Enter). After it finishes, close and reopen your terminal so the `cargo` command becomes available.

> **Windows users**: download the installer from https://rustup.rs and run it.

### Step 3: Install pnpm

pnpm is a package manager for JavaScript. It downloads the libraries DeCIpher depends on.

```bash
npm install -g pnpm
```

(`npm` was installed automatically when you installed Node.js in Step 1.)

### Step 4: Download the project

If you haven't already, clone (download) the project:

```bash
git clone https://github.com/Xiaowen-Yang/DeCIpher.git
cd DeCIpher
```

> If `git` is not found: install it from https://git-scm.com/downloads.

### Step 5: Install JavaScript dependencies

```bash
pnpm install
```

This reads `package.json` and downloads everything DeCIpher's JavaScript side needs.

### Step 6: Build the Rust TUI

```bash
cargo build --bin decipher-tui
```

This compiles the terminal interface. The first build may take 1–3 minutes (it downloads and compiles Rust dependencies). Subsequent builds are much faster.

### Step 7: Configure an AI provider

DeCIpher needs an API key from an AI provider (OpenAI, Anthropic, etc.) to work. Set one up:

```bash
# For OpenAI:
./bin/decipher setting set provider openai
./bin/decipher setting set api-key sk-YOUR_KEY_HERE

# For Anthropic:
./bin/decipher setting set provider anthropic
./bin/decipher setting set api-key sk-ant-YOUR_KEY_HERE
```

Replace `sk-YOUR_KEY_HERE` with your actual API key (you get this from the provider's website).

### Step 8: Run DeCIpher

```bash
./bin/decipher
```

You should see the DeCIpher interface. Type a task (e.g. "Fix this Docker build failure") and press Enter.

---

## Verify your setup

If something feels wrong, run the built-in health check:

```bash
./bin/decipher doctor
```

It will tell you what is missing or misconfigured.

---

## Updating

```bash
git pull              # get latest code
pnpm install          # update JS dependencies (if any changed)
cargo build --bin decipher-tui   # rebuild the TUI
```

---

## Usage

### Interactive Mode

When you run `./bin/decipher`, the terminal UI starts:

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

### Additional Provider Setup

```bash
# Custom OpenAI-compatible endpoint (DeepSeek, Ollama, etc.)
./bin/decipher setting set provider custom
./bin/decipher setting set base_url https://your-api.com/v1/chat/completions
./bin/decipher setting set model your-model
./bin/decipher setting set api-key your-key
```

Config is stored at `~/.decipher/config.json`.

---

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
