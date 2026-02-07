import re
from typing import List, Dict, Any, Optional
from .patterns import STAGE_PATTERNS, FAILURE_ANCHORS

def slice_log(raw_lines: List[str], exit_code: Optional[int] = None) -> Dict[str, Any]:
    stages = _detect_stages(raw_lines)
    anchor_line = _find_anchor(raw_lines)
    
    focus_window = _extract_window(raw_lines, anchor_line, stages)
    evidence_snippets = _extract_evidence(raw_lines, anchor_line, focus_window)
    
    return {
        "stages": stages,
        "focus_window": focus_window,
        "evidence_snippets": evidence_snippets
    }

def _detect_stages(lines: List[str]) -> List[Dict[str, Any]]:
    stages = []
    current_stage = {"name": "other", "line_start": 0}
    
    for i, line in enumerate(lines):
        new_stage_name = None
        for stage_type, patterns in STAGE_PATTERNS.items():
            for pattern in patterns:
                if re.search(pattern, line):
                    new_stage_name = stage_type
                    break
            if new_stage_name:
                break
        
        if new_stage_name and new_stage_name != current_stage["name"]:
            current_stage["line_end"] = i - 1
            stages.append(current_stage)
            current_stage = {"name": new_stage_name, "line_start": i}
            
    current_stage["line_end"] = len(lines) - 1
    stages.append(current_stage)
    return stages

def _find_anchor(lines: List[str]) -> int:
    # Scan from bottom to top
    for pattern in FAILURE_ANCHORS:
        for i in range(len(lines) - 1, -1, -1):
            if re.search(pattern, lines[i]):
                return i
    return len(lines) - 1  # Default to last line

def _extract_window(lines: List[str], anchor: int, stages: List[Dict]) -> Dict[str, Any]:
    context_before = 200
    context_after = 80
    
    start = max(0, anchor - context_before)
    end = min(len(lines), anchor + context_after)
    
    # Identify stage
    stage_name = "other"
    for stage in stages:
        if stage["line_start"] <= anchor <= stage["line_end"]:
            stage_name = stage["name"]
            break
            
    window_lines = lines[start:end]
    # Simple de-noise: Collapse consecutive duplicates
    cleaned_window = []
    if window_lines:
        prev = window_lines[0]
        cleaned_window.append(prev)
        for line in window_lines[1:]:
            if line != prev:
                cleaned_window.append(line)
                prev = line
                
    return {
        "stage": stage_name,
        "anchor_line": anchor,
        "context_before": context_before,
        "context_after": context_after,
        "text": "\n".join(cleaned_window),
        "lines": cleaned_window # Keep list for easier processing later if needed
    }

def _extract_evidence(lines: List[str], anchor: int, window: Dict) -> List[Dict[str, Any]]:
    snippets = []
    idx = 1
    
    # Primary evidence: Anchor line
    start = max(0, anchor - 3)
    end = min(len(lines), anchor + 4) # +1 for extraction exclusive logic usually, but here just range
    
    snippets.append({
        "id": f"E{idx}",
        "lines": [start+1, end], # 1-based for display
        "text": "\n".join(lines[start:end])
    })
    
    # Secondary evidence: Scan window for other strong signals? (MVP: Just anchor area)
    return snippets
