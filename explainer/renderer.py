import json
import html
from typing import Dict, Any

def render_html(result: Dict[str, Any], output_path: str):
    """Generates a single-file HTML report from the analysis result."""
    
    meta = result.get("meta", {})
    classification = result.get("classification", {})
    analysis = result.get("analysis", {})
    slicing = result.get("slicing", {})
    
    # Color coding for status
    status_color = "#e74c3c" if classification.get("path") == "known" else "#f39c12"
    if meta.get("exit_code") == 0:
        status_color = "#2ecc71"

    html_content = f"""
    <!DOCTYPE html>
    <html lang="en">
    <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <title>DeCIpher Report - Run {meta.get('run_id', 'N/A')}</title>
        <style>
            body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; line-height: 1.6; color: #333; max_width: 960px; margin: 0 auto; padding: 20px; }}
            h1, h2, h3 {{ color: #2c3e50; }}
            .header {{ background: #f8f9fa; padding: 20px; border-radius: 8px; border-left: 5px solid {status_color}; }}
            .badge {{ display: inline-block; padding: 4px 8px; border-radius: 4px; color: white; font-weight: bold; font-size: 0.9em; }}
            .badge-known {{ background: #e74c3c; }}
            .badge-unknown {{ background: #f39c12; }}
            .section {{ margin-top: 30px; }}
            .evidence-box {{ background: #2d3436; color: #dfe6e9; padding: 15px; border-radius: 5px; overflow-x: auto; font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; font-size: 0.9em; }}
            .evidence-line {{ display: block; }}
            .highlight {{ background: #d63031; color: white; display: inline-block; width: 100%; }}
            .card {{ border: 1px solid #ddd; border-radius: 8px; padding: 15px; margin-bottom: 15px; box-shadow: 0 2px 4px rgba(0,0,0,0.05); }}
            .tag {{ background: #e1ecf4; color: #39739d; padding: 2px 6px; border-radius: 3px; font-size: 0.85em; margin-right: 5px; }}
        </style>
    </head>
    <body>
        <div class="header">
            <h1>DeCIpher Analysis Report</h1>
            <p>
                <strong>Status:</strong> <span class="badge badge-{classification.get('path', 'unknown')}">{classification.get('category', 'Unknown').upper()}</span>
                &nbsp;|&nbsp; <strong>Exit Code:</strong> {meta.get('exit_code', 'N/A')}
                &nbsp;|&nbsp; <strong>Confidence:</strong> {float(classification.get('confidence', 0))*100:.1f}%
            </p>
        </div>

        <div class="section">
            <h2>🔎 AI Analysis</h2>
            <div class="card">
                <h3>{analysis.get('summary', 'No summary provided.')}</h3>
                
                <h4>Hypotheses</h4>
                <ul>
                {"".join(f"<li><strong>{h.get('title')}</strong> (Confidence: {h.get('confidence')})<br>{', '.join(h.get('validation_steps', []))}</li>" for h in analysis.get('hypotheses', []))}
                </ul>

                <h4>Suggested Actions</h4>
                <ul>
                {"".join(f"<li>{action}</li>" for action in analysis.get('suggested_actions', []))}
                </ul>
            </div>
        </div>

        <div class="section">
            <h2>📜 Evidence Snippets</h2>
            {_render_snippets(slicing.get('evidence_snippets', []))}
        </div>

        <div class="section">
            <h2>✂️ Slicing Info</h2>
            <p>
                Stage: <strong>{slicing.get('focus_window', {}).get('stage', 'N/A')}</strong><br>
                Anchor Line: {slicing.get('focus_window', {}).get('anchor_line', 'N/A')}
            </p>
        </div>
    </body>
    </html>
    """
    
    with open(output_path, "w", encoding="utf-8") as f:
        f.write(html_content)

def _render_snippets(snippets):
    html_out = ""
    for snippet in snippets:
        lines = snippet.get("text", "").split("\n")
        start_line = snippet.get("lines", [0, 0])[0]
        
        code_block = ""
        for i, line in enumerate(lines):
            line_num = start_line + i
            # Simple highlight if it looks like an error (MVP heuristic)
            css_class = ""
            if "Error" in line or "FAIL" in line or "exit code" in line or "Killed" in line:
                css_class = "highlight"
            code_block += f'<span class="evidence-line {css_class}"><span style="color:#636e72; margin-right:10px;">{line_num}</span>{html.escape(line)}</span>'
            
        html_out += f"""
        <div class="evidence-box">
            <strong>Snippet {snippet.get('id')}</strong>
            <pre>{code_block}</pre>
        </div><br>
        """
    return html_out
