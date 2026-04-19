# Environment Bootstrap Mission Support

This skill supports environment setup and generation/repair decisions inside the
mission-driven runtime.

## Dependency Detection

### Node.js
- Check: `node --version`
- Minimum: `>= 18.0.0`
- Install: `fnm` (recommended) or `nvm`, or system package manager

### pnpm
- Check: `pnpm --version`
- Minimum: `>= 8.0.0`
- Install: `npm install -g pnpm` or `corepack enable && corepack prepare pnpm@latest --activate`

### Docker
- Check: `docker --version` and `docker info`
- Required for many container missions
- Install: Docker Desktop (macOS/Windows), or `curl -fsSL https://get.docker.com | sh` (Linux)

### Python
- Check: `python3 --version`
- Minimum: varies by project
- Install: `pyenv install 3.11` or system package manager

## Version Comparison
- Split version string: `node --version` → `v18.19.0` → `18`
- Compare major version numerically

## Safety Rules
- Always prompt before installing anything
- Never use sudo without explicit user consent
- Always show the install command before running it
- Provide OS-specific instructions when the mission requires manual remediation
