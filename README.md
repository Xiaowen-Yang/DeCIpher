# DeCIpher

```
                  /\_/\
                 ( •ᴥ• )
>_ DeCIpher — AI CI Troubleshooter
```

An interactive CLI tool that triages, patches, and verifies CI/deployment failures end-to-end. Think Codex or Claude Code, but narrowed to **CI pipelines, Docker builds, deployment issues, and environment setup**.

## Quick Start

```bash
# Install
pnpm install

# Run interactive mode
./bin/decipher

# Or run a demo scenario directly
node bin/decipher demo scenarios/docker-copy-path-bug
```

## What It Does

```
User describes problem
    → DeCIpher collects context (git, Dockerfiles, workflows)
    → Triage Node classifies the failure
    → Fixer Node proposes a minimal patch
    → Verifier Node runs verification command
    → Structured report output
```

DeCIpher stops early and says `NEEDS_HUMAN_REVIEW` when confidence is low, the same patch is attempted twice, or the fix touches too many files. Knowing when to stop is a feature.

## Demo Scenarios

| Scenario | Failure Type | Auto-Fix |
|----------|-------------|----------|
| `docker-copy-path-bug` | COPY path doesn't exist | `COPY src/ .` → `COPY . .` |
| `ci-python-version-drift` | CI uses Python 3.10, project needs 3.11 | Update workflow |
| `docker-entrypoint-permission` | Entrypoint script not executable | Add `chmod +x` |
| `env-missing-node` | Node.js not installed | Install instructions |

```bash
node bin/decipher demo scenarios/docker-copy-path-bug
node bin/decipher demo scenarios/ci-python-version-drift
node bin/decipher demo scenarios/docker-entrypoint-permission
node bin/decipher demo scenarios/env-missing-node
```

## Interactive Mode

```bash
./bin/decipher
```

```
╭─────────────────────────────────────────────────────────╮
│                   /\_/\                                 │
│                  ( •ᴥ• )                                │
│ >_ DeCIpher (v0.1.0)                                    │
│                                                         │
│ provider:  openai                                       │
│ model:     gpt-4o              /model to change         │
│ directory: ~/my-project                                 │
│ api key:   ● configured                                 │
╰─────────────────────────────────────────────────────────╯

› My Docker build fails with COPY src/ not found
  ...DeCIpher responds conversationally...

› /demo scenarios/docker-copy-path-bug
  ...runs full triage → fix → verify pipeline...

› /doctor
  ✓ Node.js    24.x    (>= 18)
  ✓ pnpm       10.x    (>= 8)
  ✓ Docker     28.x
```

### Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/model [name]` | Show or change AI model |
| `/demo <scenario>` | Run a demo scenario |
| `/doctor` | Check environment dependencies |
| `/agents` | List built-in agents + custom skills |
| `/config show` | Show current configuration |
| `/quit` | Exit |

## Configuration

```bash
# OpenAI
node bin/decipher config set provider openai
node bin/decipher config set api-key sk-xxx

# Anthropic
node bin/decipher config set provider anthropic
node bin/decipher config set api-key sk-ant-xxx

# Custom OpenAI-compatible API (DeepSeek, Ollama, etc.)
node bin/decipher config set provider custom
node bin/decipher config set base_url https://your-api.com/v1/chat/completions
node bin/decipher config set model your-model
node bin/decipher config set api-key your-key
```

Config stored at `~/.decipher/config.json`.

## CLI Commands

```bash
./bin/decipher                          # Interactive mode
node bin/decipher demo <scenario>       # Run demo scenario
node bin/decipher doctor                # Check environment
node bin/decipher triage <log-file>     # Triage a failure log
node bin/decipher verify "<command>"    # Run verification
node bin/decipher config show|set|reset # Manage config
node bin/decipher --help                # Full help
```

## Architecture

```
bin/decipher          CLI entry — boxed header, slash commands, routing
agents/orchestrator/  triage → fix → verify loop (max 3 iterations)
agents/triage/        AI-powered failure classifier (10 taxonomy labels)
agents/fixer/         AI-powered minimal patch proposer
agents/verifier/      Command runner + environment checker
lib/config.js         ~/.decipher/config.json read/write
lib/api-client.js     OpenAI / Anthropic / custom provider abstraction
lib/template.js       {variable} interpolation for prompt templates
lib/reporter.js       7-section structured output formatter
prompts/              Markdown prompt templates with placeholders
skills/               Domain knowledge injected per node
scenarios/            Deterministic demo fixtures
```

## Testing

```bash
pnpm test                # 22 unit tests
make doctor              # Environment check
make verify              # Structural scenario verification
```

## Tech Stack

- Node.js 18+ (ESM)
- `node:test` (built-in test runner)
- `picocolors` (terminal colors)
- Native `fetch` (API calls)
- No heavy frameworks

## License

MIT
