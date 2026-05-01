//! Embedded templates for built-in skills.
//!
//! Templates live in `assets/templates/` at the workspace root. They're
//! embedded into the binary at compile time via `include_str!()` so the
//! binary works without the repo present at runtime.
//!
//! Override resolution order (highest priority first):
//!   1. ~/.config/archon/templates/<name>.md   (per-user override)
//!   2. <workdir>/assets/templates/<name>.md   (per-project override)
//!   3. embedded (compile-time fallback)

use std::path::{Path, PathBuf};

pub const AI_AGENT_PRD: &str = include_str!("../../../../assets/templates/ai-agent-prd.md");
pub const PRD_TO_SPEC: &str = include_str!("../../../../assets/templates/prdtospec.md");

/// Source label so callers/tests can verify which override won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateSource {
    UserOverride,
    WorkdirOverride,
    Embedded,
    Missing,
}

/// Resolve a template by name.
///
/// Returns `(content, source)`. Content is empty when source is `Missing`.
pub fn resolve_template(name: &str, workdir: &Path) -> (String, TemplateSource) {
    if let Some(home) = dirs::config_dir() {
        let user_path: PathBuf = home.join("archon/templates").join(format!("{name}.md"));
        if user_path.exists()
            && let Ok(content) = std::fs::read_to_string(&user_path)
        {
            return (content, TemplateSource::UserOverride);
        }
    }

    let workdir_path: PathBuf = workdir.join("assets/templates").join(format!("{name}.md"));
    if workdir_path.exists()
        && let Ok(content) = std::fs::read_to_string(&workdir_path)
    {
        return (content, TemplateSource::WorkdirOverride);
    }

    let embedded = match name {
        "ai-agent-prd" => AI_AGENT_PRD,
        "prdtospec" => PRD_TO_SPEC,
        _ => return (String::new(), TemplateSource::Missing),
    };
    (embedded.to_string(), TemplateSource::Embedded)
}

/// Resolve the body portion of a SKILL.md by name across override paths.
/// Returns `Some(body)` if any override is found and parses cleanly,
/// `None` otherwise (caller falls back to embedded).
pub fn resolve_skill_body(name: &str, workdir: &Path) -> Option<String> {
    let candidates: Vec<Option<PathBuf>> = vec![
        Some(workdir.join(format!(".archon/skills/{name}.md"))),
        Some(workdir.join(format!(".archon/skills/{name}/SKILL.md"))),
        dirs::config_dir().map(|h| h.join(format!("archon/skills/{name}.md"))),
        dirs::config_dir().map(|h| h.join(format!("archon/skills/{name}/SKILL.md"))),
    ];
    let candidates: Vec<PathBuf> = candidates.into_iter().flatten().collect();

    for path in candidates {
        if !path.is_file() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Some(skill) = crate::skills::discovery::parse_skill_md(&content)
        {
            return Some(skill.body);
        }
    }
    None
}
