use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::agents::get_agent_by_key;
use super::rlm::research_output_namespaces;

#[derive(Clone, Debug)]
pub struct ResearchAgentArtifact {
    pub path: PathBuf,
    pub hash: String,
}

pub fn write_research_agent_artifacts(
    bundle_dir: &Path,
    ordinal: usize,
    agent_key: &str,
    output: &str,
) -> Result<Vec<ResearchAgentArtifact>> {
    let safe_key = safe_segment(agent_key);
    let mut written = Vec::new();
    let canonical = bundle_dir
        .join("outputs")
        .join("markdown")
        .join(format!("{ordinal:03}-{safe_key}.md"));
    write_artifact(&canonical, output)?;
    written.push(ResearchAgentArtifact {
        path: canonical,
        hash: sha256_hex(output.as_bytes()),
    });

    if let Some(agent) = get_agent_by_key(agent_key) {
        let rlm_dir = bundle_dir.join("outputs").join("rlm");
        for namespace in research_output_namespaces(agent) {
            let path = rlm_dir
                .join(safe_artifact_path(&namespace))
                .with_extension("md");
            write_artifact(&path, output)?;
            written.push(ResearchAgentArtifact {
                path,
                hash: sha256_hex(output.as_bytes()),
            });
        }

        let dir = bundle_dir
            .join("outputs")
            .join("artifacts")
            .join(format!("{ordinal:03}-{safe_key}"));
        for artifact in agent.output_artifacts {
            let path = dir.join(safe_artifact_path(artifact));
            write_artifact(&path, output)?;
            written.push(ResearchAgentArtifact {
                path,
                hash: sha256_hex(output.as_bytes()),
            });
        }
    }

    Ok(written)
}

fn write_artifact(path: &Path, output: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, markdown_output(output)).with_context(|| format!("write {}", path.display()))
}

fn markdown_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn safe_artifact_path(path: &str) -> PathBuf {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(safe_segment)
        .collect()
}

fn safe_segment(segment: &str) -> String {
    let mut out = String::new();
    for c in segment.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "artifact".to_string()
    } else {
        trimmed.to_string()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
