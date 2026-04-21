//! Skill file loader for DeCIpher.
//!
//! Skills are markdown files that provide domain-specific guidance injected
//! into the agent system prompt.
//!
//! Search paths (later entries override earlier ones of the same name):
//!  1. `~/.decipher/skills/<name>/SKILL.md`  — user-level skills
//!  2. `<workspace>/.decipher/skills/<name>/SKILL.md` — project-level skills

use std::collections::HashMap;
use std::path::Path;

/// A single loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
}

/// Load all skills from the user home and project workspace directories.
///
/// Project-level skills (workspace) override user-level skills of the same name.
pub fn load_skills(decipher_home: &Path, workspace: &Path) -> Vec<Skill> {
    let mut skills: HashMap<String, Skill> = HashMap::new();

    // Load user-level skills first (lower priority).
    let user_skills_dir = decipher_home.join("skills");
    if let Ok(entries) = std::fs::read_dir(&user_skills_dir) {
        for entry in entries.flatten() {
            let skill_file = entry.path().join("SKILL.md");
            if skill_file.is_file() {
                if let Some(skill) = parse_skill_file(&skill_file) {
                    skills.insert(skill.name.clone(), skill);
                }
            }
        }
    }

    // Load project-level skills (higher priority — overrides user-level).
    let project_skills_dir = workspace.join(".decipher").join("skills");
    if let Ok(entries) = std::fs::read_dir(&project_skills_dir) {
        for entry in entries.flatten() {
            let skill_file = entry.path().join("SKILL.md");
            if skill_file.is_file() {
                if let Some(skill) = parse_skill_file(&skill_file) {
                    skills.insert(skill.name.clone(), skill);
                }
            }
        }
    }

    let mut result: Vec<Skill> = skills.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

/// Parse a SKILL.md file, extracting frontmatter and body.
///
/// Expected format:
/// ```markdown
/// ---
/// name: deploy
/// description: Kubernetes deployment workflow
/// ---
///
/// When deploying to Kubernetes...
/// ```
fn parse_skill_file(path: &Path) -> Option<Skill> {
    let content = std::fs::read_to_string(path).ok()?;

    // Try to parse YAML frontmatter between --- delimiters.
    let (name, description, body) = if content.starts_with("---") {
        let end = content[3..].find("\n---")?;
        let frontmatter = &content[3..end + 3];
        let body = content[end + 7..].trim_start().to_string();

        let name = extract_frontmatter_field(frontmatter, "name")
            .unwrap_or_else(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });
        let description =
            extract_frontmatter_field(frontmatter, "description").unwrap_or_default();

        (name, description, body)
    } else {
        // No frontmatter — use directory name as skill name.
        let name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        (name, String::new(), content.trim().to_string())
    };

    if name.is_empty() {
        return None;
    }

    Some(Skill {
        name,
        description,
        content: body,
    })
}

fn extract_frontmatter_field(frontmatter: &str, key: &str) -> Option<String> {
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Format loaded skills for injection into a system prompt.
pub fn format_skills_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Skills\n\n");
    for skill in skills {
        out.push_str(&format!("### {}\n", skill.name));
        if !skill.description.is_empty() {
            out.push_str(&format!("_{}_\n\n", skill.description));
        }
        out.push_str(&skill.content);
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_skill(dir: &Path, name: &str, content: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn load_user_skills_from_home() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        write_skill(
            &skills_dir,
            "deploy",
            "---\nname: deploy\ndescription: K8s workflow\n---\nWhen deploying...",
        );

        let skills = load_skills(tmp.path(), Path::new("/nonexistent"));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "deploy");
        assert_eq!(skills[0].description, "K8s workflow");
        assert!(skills[0].content.contains("When deploying"));
    }

    #[test]
    fn project_skills_override_user_skills() {
        let home_tmp = tempfile::tempdir().unwrap();
        let ws_tmp = tempfile::tempdir().unwrap();

        let home_skills = home_tmp.path().join("skills");
        fs::create_dir_all(&home_skills).unwrap();
        write_skill(
            &home_skills,
            "deploy",
            "---\nname: deploy\ndescription: user level\n---\nUser deploy instructions",
        );

        let project_skills = ws_tmp.path().join(".decipher").join("skills");
        fs::create_dir_all(&project_skills).unwrap();
        write_skill(
            &project_skills,
            "deploy",
            "---\nname: deploy\ndescription: project level\n---\nProject deploy instructions",
        );

        let skills = load_skills(home_tmp.path(), ws_tmp.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "project level");
        assert!(skills[0].content.contains("Project deploy instructions"));
    }

    #[test]
    fn load_skills_returns_empty_for_missing_dirs() {
        let skills = load_skills(Path::new("/nonexistent"), Path::new("/also_nonexistent"));
        assert!(skills.is_empty());
    }

    #[test]
    fn format_skills_section_produces_prompt_text() {
        let skills = vec![Skill {
            name: "docker".to_string(),
            description: "Docker build guide".to_string(),
            content: "Always use multi-stage builds.".to_string(),
        }];
        let section = format_skills_section(&skills);
        assert!(section.contains("## Skills"));
        assert!(section.contains("### docker"));
        assert!(section.contains("Docker build guide"));
        assert!(section.contains("Always use multi-stage builds."));
    }

    #[test]
    fn format_skills_section_empty_for_no_skills() {
        let section = format_skills_section(&[]);
        assert!(section.is_empty());
    }

    #[test]
    fn skill_without_frontmatter_uses_dir_name() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        write_skill(&skills_dir, "myskill", "No frontmatter here.");

        let skills = load_skills(tmp.path(), Path::new("/nonexistent"));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "myskill");
        assert!(skills[0].content.contains("No frontmatter here."));
    }
}
