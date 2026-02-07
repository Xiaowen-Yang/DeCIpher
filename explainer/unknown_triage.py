import json
import os
import urllib.request
from typing import Dict, Any, List, Optional

def triage_unknown(
    classification: Dict[str, Any],
    focus_window: Dict[str, Any],
    evidence: List[Dict],
    ai_config: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    config = ai_config or {}
    mode = (config.get("ai_mode") or os.environ.get("AI_MODE", "off")).lower()

    if mode == "mock":
        return _mock_triage(evidence)
    if mode == "llm":
        return _llm_triage(classification, focus_window, evidence, config)
    return _generic_triage(evidence)

def _mock_triage(evidence: List[Dict], title_prefix: str = "") -> Dict[str, Any]:
    evidence_ids = [e.get("id") for e in evidence if e.get("id")] or ["E1"]
    return {
        "summary": f"{title_prefix}AI Analysis: Potential network or dependency issue detected.",
        "hypotheses": [
            {
                "title": "Transient Network Failure",
                "confidence": "Medium",
                "evidence_ids": [evidence_ids[0]],
                "validation_steps": ["Check network logs", "Retry job"]
            },
            {
                "title": "Dependency Version Conflict",
                "confidence": "Low",
                "evidence_ids": [evidence_ids[0]],
                "validation_steps": ["Check requirements.txt", "Pin versions"]
            }
        ],
        "suggested_actions": ["Retry the job", "Check network connectivity"],
        "uncertainty": {"level": "Medium", "reasons": ["No explicit error code found"]}
    }

def _generic_triage(evidence: List[Dict], reason: Optional[str] = None) -> Dict[str, Any]:
    evidence_ids = [e["id"] for e in evidence] if evidence else []
    reasons = ["Pattern not in knowledge base"]
    if reason:
        reasons.append(reason)
    return {
        "summary": "Automated triage could not identify a known pattern.",
        "hypotheses": [
            {
                "title": "Unclassified Error",
                "confidence": "Low",
                "evidence_ids": evidence_ids,
                "validation_steps": ["Inspect the highlighted log segment manually."]
            }
        ],
        "suggested_actions": ["Manual Review Required"],
        "uncertainty": {"level": "High", "reasons": reasons}
    }

def _llm_triage(
    classification: Dict[str, Any],
    focus_window: Dict[str, Any],
    evidence: List[Dict],
    config: Dict[str, Any],
) -> Dict[str, Any]:
    api_key = config.get("api_key") or os.environ.get("AI_API_KEY")
    api_base = config.get("api_base") or os.environ.get("AI_API_BASE") or "https://api.openai.com/v1/chat/completions"
    api_model = config.get("api_model") or os.environ.get("AI_API_MODEL") or "gpt-4o-mini"
    timeout = config.get("api_timeout") or os.environ.get("AI_API_TIMEOUT") or 30

    if not api_key:
        return _generic_triage(evidence, reason="AI_MODE=llm but AI_API_KEY not set")

    evidence_text = _format_evidence(evidence)
    prompt = (
        "You are a CI log triage assistant. Return ONLY valid JSON with keys: "
        "summary, hypotheses, suggested_actions, uncertainty. "
        "Each hypothesis must have title, confidence, evidence_ids, validation_steps "
        "(list of strings). Evidence IDs must come from the provided evidence list."
    )
    user_content = {
        "classification": classification,
        "focus_window": {
            "stage": focus_window.get("stage"),
            "anchor_line": focus_window.get("anchor_line"),
            "context_before": focus_window.get("context_before"),
            "context_after": focus_window.get("context_after"),
        },
        "evidence": evidence_text,
    }

    payload = {
        "model": api_model,
        "temperature": 0.2,
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": json.dumps(user_content, ensure_ascii=True)},
        ],
    }

    try:
        request = urllib.request.Request(
            api_base,
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=float(timeout)) as response:
            raw = response.read().decode("utf-8")
        parsed = json.loads(raw)
        content = parsed.get("choices", [{}])[0].get("message", {}).get("content", "")
        data = _parse_json_from_text(content)
        return _normalize_llm_output(data, evidence)
    except Exception as exc:
        return _generic_triage(evidence, reason=f"LLM request failed: {exc}")

def _format_evidence(evidence: List[Dict]) -> List[Dict[str, Any]]:
    formatted = []
    for item in evidence:
        formatted.append(
            {
                "id": item.get("id"),
                "lines": item.get("lines"),
                "text": item.get("text", ""),
            }
        )
    return formatted

def _parse_json_from_text(text: str) -> Dict[str, Any]:
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        end = text.rfind("}")
        if start != -1 and end != -1 and end > start:
            return json.loads(text[start : end + 1])
        raise

def _normalize_llm_output(data: Dict[str, Any], evidence: List[Dict]) -> Dict[str, Any]:
    evidence_ids = [e.get("id") for e in evidence if e.get("id")] or []
    hypotheses = data.get("hypotheses") or []
    normalized_hypotheses = []
    for item in hypotheses:
        evidence_ref = item.get("evidence_ids") or evidence_ids[:1]
        normalized_hypotheses.append(
            {
                "title": item.get("title", "Unknown Issue"),
                "confidence": item.get("confidence", "Low"),
                "evidence_ids": evidence_ref,
                "validation_steps": item.get("validation_steps", []),
            }
        )

    return {
        "summary": data.get("summary", "LLM analysis returned no summary."),
        "hypotheses": normalized_hypotheses,
        "suggested_actions": data.get("suggested_actions", []),
        "uncertainty": data.get("uncertainty", {"level": "Medium", "reasons": []}),
    }
