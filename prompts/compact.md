# Context Compaction

You are compacting a conversation history to fit within a token budget.
Preserve information critical for the agent to continue its mission.

## What to keep
- The mission goal and current status
- Key decisions made and their rationale
- Files read or modified (paths and what was learned)
- Commands executed and their outcomes (especially failures)
- The current plan and which steps are done vs. pending
- Any error patterns or debugging insights discovered
- The most recent 2-3 tool results (verbatim — the agent needs fresh context)

## What to discard
- Redundant file read outputs (keep only "read X, found Y")
- Successful command outputs that were informational only
- Intermediate debugging steps that led nowhere
- Duplicate error messages (keep one representative instance)
- Tool call JSON formatting (summarize as prose)

## Output format

Respond with a single block of text that an agent can use as context
to continue the mission. Structure it as:

```
Mission: <goal>
Status: <current state — what's done, what's pending>
Workspace: <path>

Key findings:
- <finding 1>
- <finding 2>

Actions taken:
- <action and outcome>
- <action and outcome>

Current approach: <what the agent was doing when compacted>

Errors/blockers: <any unresolved issues>
```

Be concise. Target ~25% of the original length.
