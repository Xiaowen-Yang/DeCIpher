import argparse
import json
import uuid
import sys
import os

from .utils import read_log
from .log_slicer import slice_log
from .known_classifier import classify_failure
from .unknown_triage import triage_unknown
from .renderer import render_html

def analyze(args):
    """Analyze a log file and produce a JSON result."""
    print(f"Analyzing {args.log}...")
    
    raw_lines = read_log(args.log)
    
    # 1. Slicing
    slicing_result = slice_log(raw_lines, args.exit_code)
    focus_window = slicing_result["focus_window"]
    evidence = slicing_result["evidence_snippets"]
    
    # 2. Classification
    classification = classify_failure(focus_window, args.exit_code)
    
    # 3. Triage (if unknown) or fill generic analysis
    ai_mode = getattr(args, 'ai_mode', None) or os.environ.get("AI_MODE", "off")
    ai_config = {
        "ai_mode": ai_mode,
        "api_key": getattr(args, 'api_key', None),
        "api_base": getattr(args, 'api_base', None),
        "api_model": getattr(args, 'api_model', None),
        "api_timeout": getattr(args, 'api_timeout', None),
    }
    
    if classification["path"] == "unknown":
        # Fix: Pass ai_config to triage_unknown
        analysis = triage_unknown(classification, focus_window, evidence, ai_config)
    else:
        # Simple analysis for known issues
        analysis = {
            "summary": f"Identified known failure pattern: {classification['category']}",
            "hypotheses": [],
            "suggested_actions": ["Fix the issue identified in the category rules."],
            "uncertainty": {"level": "Low", "reasons": []}
        }

    # 4. Construct Result
    result = {
        "meta": {
            "ci_system": "local",
            "run_id": str(uuid.uuid4()),
            "exit_code": args.exit_code,
            "ai_mode": ai_mode
        },
        "slicing": slicing_result,
        "classification": classification,
        "analysis": analysis
    }
    
    # Ensure directory exists
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    
    with open(args.out, "w") as f:
        json.dump(result, f, indent=2)
    print(f"Analysis written to {args.out}")

def render(args):
    """Render a JSON result to HTML."""
    print(f"Rendering {args.input} to {args.out}...")
    with open(args.input, "r") as f:
        result = json.load(f)
    
    # Ensure directory exists
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    
    render_html(result, args.out)
    print(f"Report written to {args.out}")

def interactive():
    """Interactive mode for step-by-step execution."""
    print("\n--- DeCIpher Interactive Mode ---")
    print("Tip: Press Enter to accept default values.\n")
    
    command = input("Choose command [analyze/render/run] (default: run): ").strip().lower() or "run"
    
    args = argparse.Namespace()
    
    # Common inputs for analyze/run
    if command in ["analyze", "run"]:
        args.log = input("Log file path: ").strip()
        while not args.log:
            print("Error: Log file path is required.")
            args.log = input("Log file path: ").strip()
            
        exit_code = input("Exit code (default: 1): ").strip()
        args.exit_code = int(exit_code) if exit_code else 1
        
        args.ai_mode = input("AI Mode [off/mock/llm] (default: off): ").strip() or "off"
        
        # Initialize optional args
        args.api_key = None
        args.api_base = None
        args.api_model = None
        args.api_timeout = None
        
        if args.ai_mode == "llm":
             args.api_key = input("API Key: ").strip() or None
             args.api_base = input("API Base URL (default: https://api.openai.com/v1/chat/completions): ").strip() or None
             args.api_model = input("API Model (default: gpt-4o-mini): ").strip() or None
             timeout_val = input("API Timeout (default: 30): ").strip()
             args.api_timeout = float(timeout_val) if timeout_val else None
    
    if command == "analyze":
        args.out = input("Output JSON path (default: out/result.json): ").strip() or "out/result.json"
        analyze(args)
        
    elif command == "render":
        args.input = input("Input JSON path: ").strip()
        while not args.input:
             args.input = input("Input JSON path: ").strip()
             
        args.out = input("Output HTML path (default: out/report.html): ").strip() or "out/report.html"
        render(args)
        
    elif command == "run":
        ensure_dir = input("Output directory (default: out): ").strip() or "out"
        os.makedirs(ensure_dir, exist_ok=True)
        
        json_out = os.path.join(ensure_dir, "result.json")
        html_out = os.path.join(ensure_dir, "report.html")
        
        print(f"\nProcessing...\nJSON will be saved to: {json_out}\nHTML will be saved to: {html_out}\n")
        
        args.out = json_out
        analyze(args)
        
        args.input = json_out
        args.out = html_out
        render(args)

def main():
    parser = argparse.ArgumentParser(description="DeCIpher: CI Failure Explainer")
    subparsers = parser.add_subparsers(dest="command")
    
    # Analyze Command
    analyze_parser = subparsers.add_parser("analyze")
    analyze_parser.add_argument("--log", required=True, help="Path to CI log file")
    analyze_parser.add_argument("--exit-code", type=int, default=1, help="Process exit code")
    analyze_parser.add_argument("--out", required=True, help="Output JSON path")
    analyze_parser.add_argument("--ai-mode", choices=["off", "mock", "llm"], help="AI mode (overrides AI_MODE)")
    analyze_parser.add_argument("--api-key", help="API key (overrides AI_API_KEY)")
    analyze_parser.add_argument("--api-base", help="API base URL (overrides AI_API_BASE)")
    analyze_parser.add_argument("--api-model", help="Model name (overrides AI_API_MODEL)")
    analyze_parser.add_argument("--api-timeout", type=float, help="Timeout in seconds (overrides AI_API_TIMEOUT)")
    
    # Render Command
    render_parser = subparsers.add_parser("render")
    render_parser.add_argument("--input", required=True, help="Input JSON path")
    render_parser.add_argument("--out", required=True, help="Output HTML path")
    
    # Run Command (Convenience)
    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--log", required=True, help="Path to CI log file")
    run_parser.add_argument("--exit-code", type=int, default=1, help="Process exit code")
    run_parser.add_argument("--ai-mode", choices=["off", "mock", "llm"], help="AI mode (overrides AI_MODE)")
    run_parser.add_argument("--api-key", help="API key (overrides AI_API_KEY)")
    run_parser.add_argument("--api-base", help="API base URL (overrides AI_API_BASE)")
    run_parser.add_argument("--api-model", help="Model name (overrides AI_API_MODEL)")
    run_parser.add_argument("--api-timeout", type=float, help="Timeout in seconds (overrides AI_API_TIMEOUT)")
    
    # Interactive Command
    subparsers.add_parser("interactive")

    # If no args provided, default to interactive
    if len(sys.argv) == 1:
        interactive()
        return

    args = parser.parse_args()
    
    if args.command == "interactive":
        interactive()
    elif args.command == "analyze":
        analyze(args)
    elif args.command == "render":
        render(args)
    elif args.command == "run":
        # Hacky internal call sharing
        json_out = "out/result.json"
        html_out = "out/report.html"
        
        args.out = json_out
        analyze(args)
        
        args.input = json_out
        args.out = html_out
        render(args)

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\nOperation cancelled.")
        sys.exit(130)
