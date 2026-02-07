import re
from typing import Dict, Any, List
from .patterns import KNOWN_SIGNALS

def classify_failure(focus_window: Dict[str, Any], exit_code: int) -> Dict[str, Any]:
    text = focus_window.get("text", "")
    stage = focus_window.get("stage", "other")
    
    # 1. Docker Run OOM
    if exit_code == 137 or any(re.search(p, text) for p in KNOWN_SIGNALS['docker_run_oom']):
        if stage in ['docker_run', 'test', 'other']:
            return _result("known", "docker_run_oom", 0.95, ["exit_code_137_or_oom_signal"])
            
    # 2. Arch Mismatch
    matches = [p for p in KNOWN_SIGNALS['arch_mismatch'] if re.search(p, text)]
    if matches:
        return _result("known", "arch_mismatch", 0.95, matches)
        
    # 3. Docker Build Failure
    if stage == 'docker_build':
        matches = [p for p in KNOWN_SIGNALS['docker_build_failure'] if re.search(p, text)]
        if matches:
             return _result("known", "docker_build_failure", 0.85, matches)
             
    # 4. Test Failure
    matches = [p for p in KNOWN_SIGNALS['test_failure'] if re.search(p, text)]
    if matches:
        return _result("known", "test_failure", 0.8, matches)
        
    return _result("unknown", "unknown", 0.1, [])

def _result(path: str, category: str, confidence: float, signals: List[str]) -> Dict[str, Any]:
    return {
        "path": path,
        "category": category,
        "confidence": confidence,
        "signals": signals
    }
