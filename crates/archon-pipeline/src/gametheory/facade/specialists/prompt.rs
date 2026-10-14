use std::path::Path;

use super::super::super::agents::GameTheoryAgent;
use super::super::super::errors::GameTheoryError;

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

pub fn load_agent_prompt_body(agent: &GameTheoryAgent) -> Result<String, GameTheoryError> {
    let path = workspace_root().join(agent.prompt_source_path);
    if !path.is_file() {
        return Err(GameTheoryError::MissingAgentFile {
            path: path.display().to_string(),
        });
    }
    let content = std::fs::read_to_string(&path).map_err(|error| GameTheoryError::Io {
        message: format!("read {} failed: {error}", path.display()),
    })?;
    let (_frontmatter, body) =
        crate::agent_loader::parse_frontmatter(&content).map_err(|error| {
            GameTheoryError::Validation {
                message: format!("parse {} failed: {error}", path.display()),
            }
        })?;
    if body.trim().is_empty() {
        return Err(GameTheoryError::Validation {
            message: format!("agent source file has empty body: {}", path.display()),
        });
    }
    Ok(body)
}
