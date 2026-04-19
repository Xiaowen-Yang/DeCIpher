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

╭─
│ YOU › Fix this Docker build failure
╰─
  ✓ Fix this Docker build failure (14.8s)

┌ MISSION ────────────────────────────────────────────────
  Understood: Fix this Docker build failure

  Plan:
    1. Inspect target
    2. Execute action
    3. Verify result

┌ CLARIFICATION NEEDED ──────────────────────────────────
  DeCIpher asks: Which directory, Dockerfile, or log
  file should DeCIpher work on?

╭─
│ YOU ›
╰─
```

Type natural language to start a mission. Use slash commands to control the session:

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

## Tech Stack

- Node.js 22+ (ESM)
- `node:test` (built-in test runner)
- `picocolors` (terminal colors)
- Native `fetch` (API calls)
- No heavy frameworks

## License

MIT
