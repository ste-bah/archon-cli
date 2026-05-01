use super::{Skill, SkillContext, SkillOutput};
use std::path::Path;

/// A user-defined skill discovered from a `SKILL.md` file.
#[derive(Debug, Clone)]
pub struct UserSkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

impl Skill for UserSkill {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn execute(&self, _args: &[String], _ctx: &SkillContext) -> SkillOutput {
        SkillOutput::Markdown(self.body.clone())
    }
}

/// Discover user-defined skills from `.archon/skills/` directory (and `.claude/skills/` for backward compat).
///
/// Two-pass scan per root: subdir layout (`<name>/SKILL.md`) first, then
/// flat-file layout (`<name>.md`). Subdir wins on collision.
pub fn discover_user_skills(working_dir: &Path) -> Vec<UserSkill> {
    let search_roots = [
        working_dir.join(".archon/skills"),
        working_dir.join(".claude/skills"), // backward compat
    ];

    let mut skills = Vec::new();

    let archon_root = &search_roots[0];
    for root in &search_roots {
        if !root.is_dir() {
            continue;
        }

        // Warn when loading from deprecated .claude/skills/ path
        if root != archon_root && !archon_root.is_dir() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                root.display(),
                archon_root.display()
            );
        }

        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Collect entries first — read_dir is consumed by iteration
        let dir_entries: Vec<_> = entries.flatten().collect();

        // Pass 1: subdir layout — `<name>/SKILL.md`
        let mut subdir_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in &dir_entries {
            let skill_file = entry.path().join("SKILL.md");
            if !skill_file.is_file() {
                continue;
            }
            let content = match std::fs::read_to_string(&skill_file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(skill) = parse_skill_md(&content) {
                subdir_names.insert(skill.name.clone());
                skills.push(skill);
            }
        }

        // Pass 2: flat-file layout — `<name>.md`
        for entry in &dir_entries {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();
            if !name_str.ends_with(".md") || name_str == "README.md" {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(skill) = parse_skill_md(&content) {
                if subdir_names.contains(&skill.name) {
                    tracing::info!(
                        "skill {} present in both subdir and flat-file form; subdir wins (subdir={}, flat={})",
                        skill.name,
                        path.parent()
                            .unwrap_or(&path)
                            .join(&skill.name)
                            .join("SKILL.md")
                            .display(),
                        path.display()
                    );
                    continue;
                }
                skills.push(skill);
            }
        }
    }

    skills
}

/// Parse a `SKILL.md` file with YAML front-matter delimited by `---`.
pub fn parse_skill_md(content: &str) -> Option<UserSkill> {
    let trimmed = content.trim().trim_start_matches('\u{FEFF}');

    // Must start with "---"
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing "---"
    let after_open = &trimmed[3..];
    let close_pos = after_open.find("---")?;
    let frontmatter = &after_open[..close_pos];
    let body = after_open[close_pos + 3..].trim().to_string();

    let mut name = None;
    let mut description = None;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().to_string());
        }
    }

    Some(UserSkill {
        name: name?,
        description: description.unwrap_or_default(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_skill_md() {
        let content = "---\nname: test\ndescription: A test\n---\nBody text.";
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.name, "test");
        assert_eq!(skill.description, "A test");
        assert_eq!(skill.body, "Body text.");
    }

    #[test]
    fn parse_no_frontmatter() {
        assert!(parse_skill_md("Just text").is_none());
    }

    #[test]
    fn parse_missing_name() {
        assert!(parse_skill_md("---\ndescription: hi\n---\nbody").is_none());
    }

    #[test]
    fn parse_handles_bom() {
        let content = "\u{FEFF}---\nname: test\ndescription: BOM test\n---\nBody.";
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.name, "test");
        assert_eq!(skill.description, "BOM test");
        assert_eq!(skill.body, "Body.");
    }

    #[test]
    fn parse_preserves_body_whitespace() {
        let content =
            "---\nname: test\ndescription: whitespace\n---\n\nParagraph 1.\n\nParagraph 2.\n\n";
        let skill = parse_skill_md(content).unwrap();
        assert!(skill.body.contains("Paragraph 2."));
    }

    #[test]
    fn discover_picks_up_subdir_layout() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".archon/skills/foo");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: foo\ndescription: subdir skill\n---\nBody.",
        )
        .unwrap();
        let skills = discover_user_skills(tmp.path());
        assert!(skills.iter().any(|s| s.name == "foo"));
    }

    #[test]
    fn discover_picks_up_flat_file_layout() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".archon/skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("foo.md"),
            "---\nname: foo\ndescription: flat skill\n---\nBody.",
        )
        .unwrap();
        let skills = discover_user_skills(tmp.path());
        assert!(skills.iter().any(|s| s.name == "foo"));
    }

    #[test]
    fn discover_subdir_takes_precedence() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".archon/skills");
        std::fs::create_dir_all(skills_dir.join("foo")).unwrap();
        std::fs::write(
            skills_dir.join("foo/SKILL.md"),
            "---\nname: foo\ndescription: subdir wins\n---\nSubdir body.",
        )
        .unwrap();
        std::fs::write(
            skills_dir.join("foo.md"),
            "---\nname: foo\ndescription: flat loses\n---\nFlat body.",
        )
        .unwrap();
        let skills = discover_user_skills(tmp.path());
        let foo_skills: Vec<_> = skills.iter().filter(|s| s.name == "foo").collect();
        assert_eq!(foo_skills.len(), 1, "only one foo should be registered");
        assert_eq!(foo_skills[0].description, "subdir wins");
    }

    #[test]
    fn discover_skips_readme_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".archon/skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("README.md"),
            "---\nname: readme\ndescription: should be skipped\n---\nBody.",
        )
        .unwrap();
        let skills = discover_user_skills(tmp.path());
        assert!(!skills.iter().any(|s| s.name == "readme"));
    }

    #[test]
    fn discover_supports_legacy_claude_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".claude/skills/legacy-skill");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: legacy-skill\ndescription: legacy\n---\nBody.",
        )
        .unwrap();
        let skills = discover_user_skills(tmp.path());
        assert!(skills.iter().any(|s| s.name == "legacy-skill"));
    }
}
