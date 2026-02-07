import os
from typing import List

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
