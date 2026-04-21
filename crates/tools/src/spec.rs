use serde_json::{json, Value};

use decipher_policy::ToolClass;

use crate::classify::tool_class;

// ── ToolName ──────────────────────────────────────────────────────────────────

/// Every tool that DeCIpher agents can invoke.
///
/// This enum is the authoritative source of tool names. Both the agent loop
/// (R2 `crates/runtime`) and the approval policy (`crates/policy`) reference it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    // ── Filesystem ──────────────────────────────────────────
    ExecCommand,
    ReadFile,
    WriteFile,
    ApplyPatch,
    ListFiles,
    // ── Search ──────────────────────────────────────────────
    Search,
    GrepSearch,
    FileSearch,
    // ── Kubernetes ──────────────────────────────────────────
    KubectlGet,
    KubectlLogs,
    KubectlDescribe,
    KubectlEvents,
    // ── Internal / meta ─────────────────────────────────────
    UpdatePlan,
    Done,
    // ── Subagents ────────────────────────────────────────────
    SpawnAgent,
}

impl ToolName {
    /// The canonical string name sent to the LLM.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExecCommand => "exec_command",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ApplyPatch => "apply_patch",
            Self::ListFiles => "list_files",
            Self::Search => "search",
            Self::GrepSearch => "grep_search",
            Self::FileSearch => "file_search",
            Self::KubectlGet => "kubectl_get",
            Self::KubectlLogs => "kubectl_logs",
            Self::KubectlDescribe => "kubectl_describe",
            Self::KubectlEvents => "kubectl_events",
            Self::UpdatePlan => "update_plan",
            Self::Done => "done",
            Self::SpawnAgent => "spawn_agent",
        }
    }

    /// Parse from the string name sent by the LLM.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "exec_command" => Some(Self::ExecCommand),
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "apply_patch" => Some(Self::ApplyPatch),
            "list_files" => Some(Self::ListFiles),
            "search" => Some(Self::Search),
            "grep_search" => Some(Self::GrepSearch),
            "file_search" => Some(Self::FileSearch),
            "kubectl_get" => Some(Self::KubectlGet),
            "kubectl_logs" => Some(Self::KubectlLogs),
            "kubectl_describe" => Some(Self::KubectlDescribe),
            "kubectl_events" => Some(Self::KubectlEvents),
            "update_plan" => Some(Self::UpdatePlan),
            "done" => Some(Self::Done),
            "spawn_agent" => Some(Self::SpawnAgent),
            _ => None,
        }
    }

    /// All tools in the default agent registry (excludes internal meta-tools).
    ///
    /// These are sent to the LLM as the `tools` array in each API request.
    pub fn agent_tools() -> &'static [ToolName] {
        &[
            Self::ExecCommand,
            Self::ReadFile,
            Self::WriteFile,
            Self::ApplyPatch,
            Self::ListFiles,
            Self::Search,
            Self::GrepSearch,
            Self::FileSearch,
            Self::KubectlGet,
            Self::KubectlLogs,
            Self::KubectlDescribe,
            Self::KubectlEvents,
            Self::UpdatePlan,
            Self::Done,
            Self::SpawnAgent,
        ]
    }

    /// All tool names including internal meta-tools.
    pub fn all() -> &'static [ToolName] {
        Self::agent_tools()
    }

    /// The `ToolClass` for this tool (used by the policy engine).
    pub fn tool_class(&self) -> ToolClass {
        tool_class(*self)
    }

    /// Full schema spec for LLM function-calling API.
    pub fn spec(&self) -> ToolSpec {
        match self {
            Self::ExecCommand => ToolSpec {
                name: self.as_str(),
                description: "Run any shell command. Returns stdout, stderr, and exit code. \
                    Use this to build images, run tests, install deps, inspect files.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "cmd": {
                            "type": "string",
                            "description": "The shell command to execute"
                        },
                        "workdir": {
                            "type": "string",
                            "description": "Optional working directory (default: workspace root)"
                        }
                    },
                    "required": ["cmd"]
                }),
            },

            Self::ReadFile => ToolSpec {
                name: self.as_str(),
                description: "Read the full content of a file. \
                    Path may be absolute or relative to the workspace.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path (absolute or relative to workspace)"
                        }
                    },
                    "required": ["path"]
                }),
            },

            Self::WriteFile => ToolSpec {
                name: self.as_str(),
                description: "Write content to a file, creating it and parent directories if needed. \
                    Replaces the entire file. Requires approval.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path (absolute or relative to workspace)"
                        },
                        "content": {
                            "type": "string",
                            "description": "The complete file content to write"
                        }
                    },
                    "required": ["path", "content"]
                }),
            },

            Self::ApplyPatch => ToolSpec {
                name: self.as_str(),
                description: "Apply a unified diff to files in the workspace. \
                    Must be valid unified diff format (--- a/file, +++ b/file). \
                    Prefer write_file for small files. Requires approval.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "patch": {
                            "type": "string",
                            "description": "Unified diff content"
                        },
                        "target_file": {
                            "type": "string",
                            "description": "Target file path (if not determinable from patch headers)"
                        }
                    },
                    "required": ["patch"]
                }),
            },

            Self::ListFiles => ToolSpec {
                name: self.as_str(),
                description: "List files in a directory. \
                    Returns a tree of file names and sizes.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "directory": {
                            "type": "string",
                            "description": "Directory path (default: workspace root)"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Whether to recurse into subdirectories (default: false)"
                        }
                    }
                }),
            },

            Self::Search => ToolSpec {
                name: self.as_str(),
                description: "Search for files matching a glob pattern in the workspace.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern to match (e.g. '**/*.rs', 'src/*.ts')"
                        },
                        "directory": {
                            "type": "string",
                            "description": "Directory to search in (default: workspace root)"
                        }
                    },
                    "required": ["pattern"]
                }),
            },

            Self::GrepSearch => ToolSpec {
                name: self.as_str(),
                description: "Search file contents using ripgrep. \
                    Returns matching lines with file paths and line numbers.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex or literal pattern to search for"
                        },
                        "directory": {
                            "type": "string",
                            "description": "Directory to search in (default: workspace root)"
                        },
                        "glob": {
                            "type": "string",
                            "description": "File glob filter (e.g. '*.rs', '*.{ts,tsx}')"
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "description": "Whether search is case-sensitive (default: false)"
                        }
                    },
                    "required": ["pattern"]
                }),
            },

            Self::FileSearch => ToolSpec {
                name: self.as_str(),
                description: "Fuzzy-search for files by name in the workspace.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "File name query (substring or fuzzy match)"
                        },
                        "directory": {
                            "type": "string",
                            "description": "Directory to search in (default: workspace root)"
                        }
                    },
                    "required": ["query"]
                }),
            },

            Self::KubectlGet => ToolSpec {
                name: self.as_str(),
                description: "Run kubectl get to inspect cluster resources. \
                    Use output=json for machine-readable, wide for human overview.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "resource": {
                            "type": "string",
                            "description": "Resource type: pods, deployments, services, nodes, etc."
                        },
                        "namespace": { "type": "string", "description": "Kubernetes namespace" },
                        "output": {
                            "type": "string",
                            "enum": ["json", "yaml", "wide", "name"],
                            "description": "Output format"
                        },
                        "selector": { "type": "string", "description": "Label selector" }
                    },
                    "required": ["resource"]
                }),
            },

            Self::KubectlLogs => ToolSpec {
                name: self.as_str(),
                description: "Fetch logs from a Kubernetes pod. \
                    Use previous=true for crashed containers.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pod": { "type": "string", "description": "Pod name" },
                        "namespace": { "type": "string", "description": "Kubernetes namespace" },
                        "container": { "type": "string", "description": "Container name in pod" },
                        "previous": {
                            "type": "boolean",
                            "description": "Fetch logs from previous (crashed) instance"
                        },
                        "tail": {
                            "type": "integer",
                            "description": "Number of lines from end (default 200)"
                        }
                    },
                    "required": ["pod"]
                }),
            },

            Self::KubectlDescribe => ToolSpec {
                name: self.as_str(),
                description: "Run kubectl describe for detailed resource information \
                    including events, conditions, and status. \
                    Essential for diagnosing CrashLoopBackOff, OOMKilled, Pending pods.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "resource": { "type": "string", "description": "Resource type (pod, deployment, etc.)" },
                        "name": { "type": "string", "description": "Resource name" },
                        "namespace": { "type": "string", "description": "Kubernetes namespace" }
                    },
                    "required": ["resource", "name"]
                }),
            },

            Self::KubectlEvents => ToolSpec {
                name: self.as_str(),
                description: "List recent Kubernetes events sorted by timestamp. \
                    Shows warnings, failures, scheduling decisions, and resource lifecycle events.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "namespace": { "type": "string", "description": "Kubernetes namespace" },
                        "field_selector": { "type": "string", "description": "Field selector filter" }
                    }
                }),
            },

            Self::UpdatePlan => ToolSpec {
                name: self.as_str(),
                description: "Update the displayed plan with current step statuses. \
                    Use this to communicate progress to the user.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "steps": {
                            "type": "array",
                            "description": "Plan steps with their current status",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "step": { "type": "string" },
                                    "status": {
                                        "type": "string",
                                        "enum": ["pending", "in_progress", "completed", "failed"]
                                    }
                                },
                                "required": ["step", "status"]
                            }
                        }
                    },
                    "required": ["steps"]
                }),
            },

            Self::Done => ToolSpec {
                name: self.as_str(),
                description: "Declare the mission complete. ALWAYS provide a detailed summary. \
                    PASS = goal achieved. FAIL = could not achieve. PARTIAL = some steps succeeded. \
                    Include files_modified, errors_encountered, and next_steps (for FAIL/PARTIAL).",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "Detailed summary of what was accomplished"
                        },
                        "outcome": {
                            "type": "string",
                            "enum": ["PASS", "FAIL", "PARTIAL"],
                            "description": "Mission outcome"
                        },
                        "files_modified": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of file paths that were modified"
                        },
                        "errors_encountered": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of error descriptions encountered"
                        },
                        "next_steps": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Suggested follow-up actions for FAIL/PARTIAL"
                        }
                    },
                    "required": ["summary", "outcome"]
                }),
            },

            Self::SpawnAgent => ToolSpec {
                name: self.as_str(),
                description: "Spawn a subagent to handle a parallel or focused subtask. \
                    The subagent runs to completion and returns its result. \
                    Use for isolated sub-missions that do not depend on in-flight work.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "The mission goal for the subagent"
                        },
                        "workspace": {
                            "type": "string",
                            "description": "Optional working directory override (default: current workspace)"
                        },
                        "max_turns": {
                            "type": "integer",
                            "description": "Maximum turns for the subagent (default: 20)",
                            "default": 20
                        }
                    },
                    "required": ["task"]
                }),
            },
        }
    }
}

// ── ToolSpec ──────────────────────────────────────────────────────────────────

/// A single tool's definition — name, description, and JSON Schema for arguments.
///
/// This is provider-agnostic. The runtime converts it to the provider-specific
/// format (`providers::ToolDefinition`) when building API requests.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Canonical name sent to the LLM.
    pub name: &'static str,
    /// Human-readable description sent to the LLM.
    pub description: &'static str,
    /// JSON Schema for the tool's arguments (Anthropic `input_schema` / OpenAI `parameters`).
    pub input_schema: Value,
}

/// Build the full list of agent tool specs.
///
/// Use this when constructing the `tools` array for an API request.
pub fn all_tool_specs() -> Vec<ToolSpec> {
    ToolName::agent_tools().iter().map(|t| t.spec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_names_round_trip() {
        for tool in ToolName::all() {
            let s = tool.as_str();
            let parsed = ToolName::from_str(s);
            assert_eq!(parsed, Some(*tool), "round-trip failed for {s}");
        }
    }

    #[test]
    fn unknown_tool_returns_none() {
        assert_eq!(ToolName::from_str("nonexistent_tool"), None);
        assert_eq!(ToolName::from_str(""), None);
    }

    #[test]
    fn all_tool_specs_have_valid_schemas() {
        for tool in ToolName::all() {
            let spec = tool.spec();
            assert_eq!(spec.name, tool.as_str(), "spec name mismatch for {tool:?}");
            assert!(!spec.description.is_empty(), "empty description for {}", spec.name);
            // input_schema must be a JSON object with "type": "object"
            assert_eq!(
                spec.input_schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "input_schema for {} must be type=object",
                spec.name,
            );
        }
    }

    #[test]
    fn required_fields_are_present() {
        // exec_command requires "cmd"
        let exec = ToolName::ExecCommand.spec();
        let required = exec.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("cmd")));

        // write_file requires "path" and "content"
        let write = ToolName::WriteFile.spec();
        let required = write.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
        assert!(required.iter().any(|v| v.as_str() == Some("content")));

        // done requires "summary" and "outcome"
        let done = ToolName::Done.spec();
        let required = done.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("summary")));
        assert!(required.iter().any(|v| v.as_str() == Some("outcome")));
    }

    #[test]
    fn all_tool_specs_coverage() {
        let specs = all_tool_specs();
        assert_eq!(specs.len(), ToolName::agent_tools().len());
        // Verify each known tool name appears exactly once
        let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
        assert!(names.contains(&"exec_command"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"apply_patch"));
        assert!(names.contains(&"done"));
        assert!(names.contains(&"kubectl_get"));
    }

    #[test]
    fn done_tool_has_outcome_enum() {
        let done = ToolName::Done.spec();
        let outcome_enum = done.input_schema["properties"]["outcome"]["enum"]
            .as_array()
            .unwrap();
        let values: Vec<&str> = outcome_enum.iter().filter_map(|v| v.as_str()).collect();
        assert!(values.contains(&"PASS"));
        assert!(values.contains(&"FAIL"));
        assert!(values.contains(&"PARTIAL"));
    }
}
