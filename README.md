# DeCIpher
CI log explainer with deterministic slicing, conservative known-failure classification, optional LLM triage for unknowns, and a single-file HTML report

## Quick Start
```bash
python -m explainer.cli run --log examples/oom_137.log --exit-code 137
```
Outputs:
- `out/result.json`
- `out/report.html`

## Commands
Analyze + render (one shot):
```bash
python -m explainer.cli run --log examples/pytest_fail.log --exit-code 1
```

Analyze only:
```bash
python -m explainer.cli analyze --log examples/pytest_fail.log --exit-code 1 --out out/result.json
```

Render only:
```bash
python -m explainer.cli render --input out/result.json --out out/report.html
```

Interactive mode:
```bash
python -m explainer.cli
```

## Design and Architecture
High-level flow:
1) Ingest raw CI log lines.
2) Slice log into stages, pick a failure anchor, and extract a focus window + evidence snippets.
3) Classify with deterministic known rules.
4) If unknown, run optional AI triage (off/mock/llm) anchored to evidence.
5) Write `result.json` and render a single-file `report.html`.

Key modules:
- `explainer/utils.py`: log reading, config persistence.
- `explainer/log_slicer.py`: stage detection, failure anchor, window extraction, evidence snippets.
- `explainer/known_classifier.py`: deterministic known-failure rules with confidence.
- `explainer/unknown_triage.py`: unknown triage, LLM health check, evidence anchoring.
- `explainer/renderer.py`: HTML report rendering.
- `explainer/cli.py`: analyze/render/run/test-llm orchestration.

## AI Modes
AI is used only for unknown failures, and only when `AI_MODE=llm` (or `--ai-mode llm`).

- `off` (default): no API calls, template analysis
- `mock`: deterministic fake AI output
- `llm`: real API call for unknown triage

Examples:
```bash
AI_MODE=mock python -m explainer.cli run --log examples/unknown_vague.log
python -m explainer.cli run --log examples/unknown_vague.log --ai-mode llm --api-key YOUR_KEY
```

## LLM Config Persistence
LLM config can be saved to `~/.decipher/config.json`.

Save while running:
```bash
python -m explainer.cli run --log examples/unknown_vague.log --ai-mode llm --api-key YOUR_KEY --save-config
```

Health check:
```bash
python -m explainer.cli test-llm
```

Set a custom API base:
```bash
python -m explainer.cli test-llm \
  --api-base http://your-llm-endpoint/v1/chat/completions \
  --api-model gemini-2.5-pro \
  --save-config
```

## Example Logs
All logs in `examples/` are designed to exercise the pipeline:
- `docker_build_fail.log`
- `oom_137.log`
- `arch_mismatch.log`
- `pytest_fail.log`
- `unknown_vague.log`

## Outputs
`result.json` schema (minimum fields):
- `meta`: `ci_system`, `run_id`, `exit_code`, `ai_mode`
- `slicing`: `stages`, `focus_window`, `evidence_snippets`
- `classification`: `path`, `category`, `confidence`, `signals`
- `analysis`: `summary`, `hypotheses`, `suggested_actions`, `uncertainty`
