use decipher_policy::ToolClass;

use crate::spec::ToolName;

/// Returns the risk class for a given tool.
///
/// This is the authoritative classification — `crates/tui/src/cell.rs`
/// duplicates the read-only check for now (TODO: import from here in R2).
pub fn tool_class(name: ToolName) -> ToolClass {
    match name {
        ToolName::ExecCommand => ToolClass::Exec,
        ToolName::WriteFile => ToolClass::Write,
        ToolName::ApplyPatch => ToolClass::Write,
        ToolName::ReadFile
        | ToolName::ListFiles
        | ToolName::Search
        | ToolName::GrepSearch
        | ToolName::FileSearch
        | ToolName::KubectlGet
        | ToolName::KubectlLogs
        | ToolName::KubectlDescribe
        | ToolName::KubectlEvents
        | ToolName::UpdatePlan
        | ToolName::Done => ToolClass::Read,
        // spawn_agent is write-class: it can modify files via its subagent.
        ToolName::SpawnAgent => ToolClass::Write,
    }
}

/// True for tools that only read state and never modify files or run commands.
///
/// Mirrors `crates/tui/src/cell.rs::is_read_only_tool()`. That function is kept
/// for backward compatibility until the TUI→tools dependency is wired in R2.
pub fn is_read_only(name: ToolName) -> bool {
    matches!(tool_class(name), ToolClass::Read)
}

/// True for tools that write or create files.
pub fn is_write(name: ToolName) -> bool {
    matches!(tool_class(name), ToolClass::Write)
}

/// True for tools that execute shell commands.
pub fn is_exec(name: ToolName) -> bool {
    matches!(tool_class(name), ToolClass::Exec)
}

/// True for tools with destructive potential.
pub fn is_destructive(name: ToolName) -> bool {
    matches!(tool_class(name), ToolClass::Destructive)
}

/// String-based read-only check for use where `ToolName` hasn't been parsed yet.
///
/// Equivalent to `ToolName::from_str(name).map_or(false, is_read_only)`.
pub fn is_read_only_by_name(name: &str) -> bool {
    ToolName::from_str(name).map_or(false, is_read_only)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_command_is_exec_class() {
        assert_eq!(tool_class(ToolName::ExecCommand), ToolClass::Exec);
        assert!(is_exec(ToolName::ExecCommand));
        assert!(!is_read_only(ToolName::ExecCommand));
        assert!(!is_write(ToolName::ExecCommand));
    }

    #[test]
    fn write_tools_are_write_class() {
        assert_eq!(tool_class(ToolName::WriteFile), ToolClass::Write);
        assert_eq!(tool_class(ToolName::ApplyPatch), ToolClass::Write);
        assert!(is_write(ToolName::WriteFile));
        assert!(is_write(ToolName::ApplyPatch));
    }

    #[test]
    fn read_tools_are_read_class() {
        for name in [
            ToolName::ReadFile,
            ToolName::ListFiles,
            ToolName::Search,
            ToolName::GrepSearch,
            ToolName::FileSearch,
            ToolName::KubectlGet,
            ToolName::KubectlLogs,
            ToolName::KubectlDescribe,
            ToolName::KubectlEvents,
        ] {
            assert!(is_read_only(name), "{name:?} should be read-only");
        }
    }

    #[test]
    fn is_read_only_by_name_matches_enum() {
        assert!(is_read_only_by_name("read_file"));
        assert!(is_read_only_by_name("kubectl_get"));
        assert!(!is_read_only_by_name("exec_command"));
        assert!(!is_read_only_by_name("write_file"));
        // Unknown tools are not read-only (safe default)
        assert!(!is_read_only_by_name("unknown_tool"));
    }

    #[test]
    fn spawn_agent_is_write_class() {
        assert_eq!(tool_class(ToolName::SpawnAgent), ToolClass::Write);
        assert!(is_write(ToolName::SpawnAgent));
        assert!(!is_read_only(ToolName::SpawnAgent));
        assert!(!is_exec(ToolName::SpawnAgent));
    }

    #[test]
    fn spawn_agent_is_not_read_only_by_name() {
        assert!(!is_read_only_by_name("spawn_agent"));
    }
}
