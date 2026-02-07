import re

# Stage Patterns
STAGE_PATTERNS = {
    'github_actions': [
        r'##\[group\]',
        r'^Run ',
        r'^Post '
    ],
    'docker_build': [
        r'^Step \d+/\d+',
        r' => \[.*\]',
        r' => => #',
        r'#\d+ \['
    ],
    'test': [
        r'test session starts',
        r'FAILURES',
        r'Traceback \(most recent call last\):'
    ]
}

# Failure Anchors (Priority Order)
FAILURE_ANCHORS = [
    # 1. Exit Codes
    r'returned a non-zero code',
    r'exit code \d+',
    
    # 2. OOM / Signals
    r'OOMKilled',
    r'Killed',
    r'signal 9',
    r'137',
    
    # 3. Arch / Exec format
    r'exec format error',
    r'no matching manifest',
    
    # 4. General Errors
    r'ERROR',
    r'FATAL',
    r'Traceback',
    r'FAILURES'
]

# Known Failure signals
KNOWN_SIGNALS = {
    'docker_run_oom': [
        r'OOMKilled',
        r'Killed',
        r'signal 9',
        r'137'
    ],
    'arch_mismatch': [
        r'exec format error',
        r'no matching manifest',
        r'wrong architecture'
    ],
    'docker_build_failure': [
        r'failed to solve',
        r'The command .* returned a non-zero code',
        r'COPY failed',
        r'permission denied',
        r'Could not find a version'
    ],
    'test_failure': [
        r'FAILURES',
        r'AssertionError',
        r'E\s+AssertionError',
        r'collected \d+ tests'
    ]
}
