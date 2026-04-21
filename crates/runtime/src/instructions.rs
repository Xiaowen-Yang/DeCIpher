//! Instruction file loader for DeCIpher.
//!
//! Instructions are markdown files that provide persistent user or project-level
//! guidance injected into the agent system prompt.
//!
//! Search paths:
//!  1. `~/.decipher/DECIPHER.md`  — user-level instructions
//!  2. `<workspace>/DECIPHER.md`  — project-level instructions

use std::path::{Path, PathBuf};

/// Loaded instruction files from user and/or project level.
#[derive(Debug, Clone, Default)]
pub struct InstructionFiles {
    pub user_path: Option<PathBuf>,
    pub user_content: Option<String>,
    pub project_path: Option<PathBuf>,
    pub project_content: Option<String>,
}

impl InstructionFiles {
    /// Returns true if no instruction content was loaded from either level.
    pub fn is_empty(&self) -> bool {
        self.user_content.is_none() && self.project_content.is_none()
    }

    /// Returns a display-friendly string listing the loaded file paths.
    ///
    /// Uses `~/.decipher/DECIPHER.md` for the user-level path and
    /// `./DECIPHER.md` for the project-level path regardless of the actual
    /// filesystem paths, so the output is consistent across machines.
    ///
    /// Returns `None` if no files were loaded.
    pub fn loaded_paths_display(&self) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        if self.user_content.is_some() {
            parts.push("~/.decipher/DECIPHER.md");
        }
        if self.project_content.is_some() {
            parts.push("./DECIPHER.md");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

/// Load instruction files from the user home and project workspace directories.
///
/// Both paths are optional — missing or empty files are silently ignored.
pub fn load_instructions(decipher_home: &Path, workspace: &Path) -> InstructionFiles {
    let user_path = decipher_home.join("DECIPHER.md");
    let project_path = workspace.join("DECIPHER.md");

    let (user_path_opt, user_content) = load_file(&user_path);
    let (project_path_opt, project_content) = load_file(&project_path);

    InstructionFiles {
        user_path: user_path_opt,
        user_content,
        project_path: project_path_opt,
        project_content,
    }
}

/// Format loaded instructions into a system prompt section.
///
/// - If no files loaded: returns empty string.
/// - If one layer: `## Project Instructions\n<content>` (no sub-headers).
/// - If both layers: sub-headers for each layer.
pub fn format_instructions_section(files: &InstructionFiles) -> String {
    match (&files.user_content, &files.project_content) {
        (None, None) => String::new(),
        (Some(user), None) => {
            format!("## Project Instructions\n\n{}", user).trim_end().to_string()
        }
        (None, Some(project)) => {
            format!("## Project Instructions\n\n{}", project).trim_end().to_string()
        }
        (Some(user), Some(project)) => {
            format!(
                "## Project Instructions\n\n### User Instructions (~/.decipher/DECIPHER.md)\n{}\n\n### Workspace Instructions (./DECIPHER.md)\n{}",
                user, project
            )
            .trim_end()
            .to_string()
        }
    }
}

/// Read a file and return its trimmed content.
///
/// Returns `(Some(path), Some(content))` if the file exists and has non-empty
/// content after trimming, `(None, None)` otherwise.
fn load_file(path: &Path) -> (Option<PathBuf>, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() {
                (None, None)
            } else {
                (Some(path.to_path_buf()), Some(trimmed))
            }
        }
        Err(_) => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn no_files_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        // Use two separate non-existent sub-dirs so neither file exists.
        let home = tmp.path().join("home");
        let workspace = tmp.path().join("workspace");

        let instructions = load_instructions(&home, &workspace);

        assert!(instructions.is_empty());
        assert!(instructions.loaded_paths_display().is_none());
        assert!(instructions.user_content.is_none());
        assert!(instructions.project_content.is_none());
    }

    #[test]
    fn user_level_only() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&home).unwrap();

        fs::write(home.join("DECIPHER.md"), "User instructions here.").unwrap();

        let instructions = load_instructions(&home, &workspace);

        assert!(!instructions.is_empty());
        assert_eq!(
            instructions.user_content.as_deref(),
            Some("User instructions here.")
        );
        assert!(instructions.project_content.is_none());
        assert_eq!(
            instructions.loaded_paths_display().as_deref(),
            Some("~/.decipher/DECIPHER.md")
        );
    }

    #[test]
    fn project_level_only() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        fs::write(workspace.join("DECIPHER.md"), "Project instructions here.").unwrap();

        let instructions = load_instructions(&home, &workspace);

        assert!(!instructions.is_empty());
        assert_eq!(
            instructions.project_content.as_deref(),
            Some("Project instructions here.")
        );
        assert!(instructions.user_content.is_none());
        assert_eq!(
            instructions.loaded_paths_display().as_deref(),
            Some("./DECIPHER.md")
        );
    }

    #[test]
    fn both_layers_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&workspace).unwrap();

        fs::write(home.join("DECIPHER.md"), "User level.").unwrap();
        fs::write(workspace.join("DECIPHER.md"), "Project level.").unwrap();

        let instructions = load_instructions(&home, &workspace);

        assert!(!instructions.is_empty());
        assert_eq!(instructions.user_content.as_deref(), Some("User level."));
        assert_eq!(
            instructions.project_content.as_deref(),
            Some("Project level.")
        );
        assert_eq!(
            instructions.loaded_paths_display().as_deref(),
            Some("~/.decipher/DECIPHER.md, ./DECIPHER.md")
        );
    }

    #[test]
    fn empty_file_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&workspace).unwrap();

        // Write files containing only whitespace — should be treated as missing.
        fs::write(home.join("DECIPHER.md"), "   \n\t  \n").unwrap();
        fs::write(workspace.join("DECIPHER.md"), "\n\n   ").unwrap();

        let instructions = load_instructions(&home, &workspace);

        assert!(instructions.is_empty());
        assert!(instructions.loaded_paths_display().is_none());
    }

    #[test]
    fn format_single_layer_no_subheaders() {
        let files = InstructionFiles {
            project_content: Some("Do the thing.".to_string()),
            ..Default::default()
        };
        let output = format_instructions_section(&files);
        assert!(output.contains("## Project Instructions"));
        assert!(output.contains("Do the thing."));
        assert!(!output.contains("### Workspace Instructions"));
        assert!(!output.contains("### User Instructions"));
    }

    #[test]
    fn format_both_layers_with_subheaders() {
        let files = InstructionFiles {
            user_content: Some("User guidance.".to_string()),
            project_content: Some("Project guidance.".to_string()),
            ..Default::default()
        };
        let output = format_instructions_section(&files);
        assert!(output.contains("## Project Instructions"));
        assert!(output.contains("### User Instructions (~/.decipher/DECIPHER.md)"));
        assert!(output.contains("### Workspace Instructions (./DECIPHER.md)"));
        assert!(output.contains("User guidance."));
        assert!(output.contains("Project guidance."));
    }

    #[test]
    fn format_empty_returns_empty() {
        let files = InstructionFiles::default();
        assert_eq!(format_instructions_section(&files), "");
    }
}
