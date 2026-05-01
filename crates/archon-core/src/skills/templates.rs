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

pub const AI_AGENT_PRD: &str =
    include_str!("../../../../assets/templates/ai-agent-prd.md");
pub const PRD_TO_SPEC: &str =
    include_str!("../../../../assets/templates/prdtospec.md");

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
        if user_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_path) {
                return (content, TemplateSource::UserOverride);
            }
        }
    }

    let workdir_path: PathBuf = workdir.join("assets/templates").join(format!("{name}.md"));
    if workdir_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&workdir_path) {
            return (content, TemplateSource::WorkdirOverride);
        }
    }

    let embedded = match name {
        "ai-agent-prd" => AI_AGENT_PRD,
        "prdtospec" => PRD_TO_SPEC,
        _ => return (String::new(), TemplateSource::Missing),
    };
    (embedded.to_string(), TemplateSource::Embedded)
}
