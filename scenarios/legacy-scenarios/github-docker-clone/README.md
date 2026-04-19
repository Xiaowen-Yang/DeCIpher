# github-docker-clone — GitHub URL to Docker Demo

## What This Tests

The user pastes a GitHub URL and says one sentence:

> "https://github.com/docker/getting-started-app  I want to run this in Docker"

There are **no pre-existing files**. DeCIpher must:

1. Detect the GitHub URL in the input
2. Clone the repository
3. Read the README to understand the project
4. Find or generate a Dockerfile
5. Build the Docker image
6. Run the container
7. Verify it's running

## How To Run

### Interactive mode (the demo):
```bash
decipher
```
Then type:
```
https://github.com/docker/getting-started-app  Build and run this in Docker
```

### Demo mode:
```bash
decipher demo scenarios/github-docker-clone
```

## Acceptance Criteria

See `acceptance.json` — the agent passes when:
- Repository was cloned
- README.md exists
- A Dockerfile exists (from repo or generated)
- Docker image was built
- Container is currently running

## Notes

- This scenario uses Docker's official getting-started-app (Node.js + React)
- The repo already includes a Dockerfile, so the agent should use it directly
- Any public GitHub repo with a Dockerfile should work with the same flow
