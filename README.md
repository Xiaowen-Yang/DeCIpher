# DeCIpher

```
     /\_/\
    ( •ᴥ• )   DeCIpher v0.1.0
```

A mission-driven local execution agent for CI/deployment tasks. Give it a goal — it plans, executes, verifies, and adapts until the job is done.

## Install

```bash
# Clone and install
git clone <repo-url> && cd DeCIpher
pnpm install

# Set up your AI provider
./bin/decipher setting set provider openai
./bin/decipher setting set api-key sk-xxx
```

Requires **Node.js 20+** and **pnpm**.

Optional: **Docker** (for container scenarios).

## Usage

```bash
# Start interactive mode
./bin/decipher

# Check environment
./bin/decipher doctor
```

### Interactive Mode

```
    /\_/\
   ( •ᴥ• )  DeCIpher v0.1.0

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
./bin/decipher setting set provider openai
./bin/decipher setting set api-key sk-xxx

# Anthropic
./bin/decipher setting set provider anthropic
./bin/decipher setting set api-key sk-ant-xxx

# Custom OpenAI-compatible (DeepSeek, Ollama, etc.)
./bin/decipher setting set provider custom
./bin/decipher setting set base_url https://your-api.com/v1/chat/completions
./bin/decipher setting set model your-model
./bin/decipher setting set api-key your-key
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

- Node.js 18+ (ESM)
- `node:test` (built-in test runner)
- `picocolors` (terminal colors)
- Native `fetch` (API calls)
- No heavy frameworks

## License

MIT
