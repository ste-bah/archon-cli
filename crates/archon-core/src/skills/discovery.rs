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

/// Discover user-defined skills from `.claude/skills/` and `.archon/skills/` directories.
///
/// Each skill is expected to live in its own subdirectory containing a `SKILL.md` file
/// with YAML front-matter (`name`, `description`) followed by a body.
pub fn discover_user_skills(working_dir: &Path) -> Vec<UserSkill> {
    let search_roots = [
        working_dir.join(".claude/skills"),
        working_dir.join(".archon/skills"),
    ];

    let mut skills = Vec::new();

    for root in &search_roots {
        if !root.is_dir() {
            continue;
        }

        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let skill_file = entry.path().join("SKILL.md");
            if !skill_file.is_file() {
                continue;
            }

            let content = match std::fs::read_to_string(&skill_file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(skill) = parse_skill_md(&content) {
                skills.push(skill);
            }
        }
    }

    skills
}

/// Parse a `SKILL.md` file with YAML front-matter delimited by `---`.
fn parse_skill_md(content: &str) -> Option<UserSkill> {
    let trimmed = content.trim();

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
}
