import json
import os
from typing import List, Dict, Any

CONFIG_DIR = os.path.join(os.path.expanduser("~"), ".decipher")
CONFIG_PATH = os.path.join(CONFIG_DIR, "config.json")

def read_log(path: str) -> List[str]:
    """
    Reads a log file and returns a list of strings, preserving line order.
    Normalizes line endings to \n.
    """
    if not os.path.exists(path):
        raise FileNotFoundError(f"Log file not found: {path}")
    
    with open(path, 'r', encoding='utf-8', errors='replace') as f:
        # Read all lines and strip trailing newlines while keeping content
        lines = [line.rstrip('\n').rstrip('\r') for line in f]
    return lines

def load_config() -> Dict[str, Any]:
    if not os.path.exists(CONFIG_PATH):
        return {}
    try:
        with open(CONFIG_PATH, "r", encoding="utf-8") as f:
            data = json.load(f)
        if isinstance(data, dict):
            return data
    except (OSError, json.JSONDecodeError):
        return {}
    return {}

def save_config(data: Dict[str, Any]) -> None:
    os.makedirs(CONFIG_DIR, exist_ok=True)
    with open(CONFIG_PATH, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=True)
