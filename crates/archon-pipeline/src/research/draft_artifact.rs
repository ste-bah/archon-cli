use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::runner::{PipelineSession, QualityScore};

pub fn paper_to_validate(session: &PipelineSession) -> Option<String> {
    let combiner = session
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == "final-paper-combiner")
        .map(|(_, result)| result);
    let repaired = combiner
        .filter(|result| was_repaired(result.quality.as_ref()))
        .map(|result| result.output.clone());
    repaired
        .or_else(|| super::final_assembly::assemble_final_paper(session))
        .or_else(|| combiner.map(|result| result.output.clone()))
}

pub fn was_repaired(quality: Option<&QualityScore>) -> bool {
    quality
        .and_then(|quality| quality.dimensions.get("artifact_repaired"))
        .is_some_and(|value| *value > 0.0)
}

pub fn repair_prompt(current_paper: &str, findings: &str) -> String {
    format!(
        "# Final Paper Repair\n\nApply only the exact changes required by the validator findings below. Preserve all citations and supported claims. Do not add unsupported claims, remove supported material, or make unrelated edits. Return the complete repaired paper in Markdown, without commentary.\n\n## Current paper\n\n{current_paper}\n\n## Exact validator findings\n\n{findings}"
    )
}

pub fn write_draft_research_artifacts(
    bundle_dir: &Path,
    latest_paper: &str,
    findings: &str,
) -> Result<Vec<(String, PathBuf, String)>> {
    let exports = bundle_dir.join("exports");
    fs::create_dir_all(&exports)?;
    let paper = exports.join("draft-paper.md");
    let findings_path = exports.join("draft-paper.findings.md");
    let draft = format!(
        "> **DRAFT — FAILED VALIDATION. This paper did not pass the final validator.**\n\n{}\n",
        latest_paper.trim()
    );
    let findings_body = format!("# Latest validator findings\n\n{}\n", findings.trim());
    fs::write(&paper, draft.as_bytes()).with_context(|| format!("write {}", paper.display()))?;
    fs::write(&findings_path, findings_body.as_bytes())
        .with_context(|| format!("write {}", findings_path.display()))?;
    Ok(vec![
        (
            "research-paper-draft".to_string(),
            paper,
            sha256_hex(draft.as_bytes()),
        ),
        (
            "research-paper-draft-findings".to_string(),
            findings_path,
            sha256_hex(findings_body.as_bytes()),
        ),
    ])
}

pub fn supersede_draft_research_artifacts(
    bundle_dir: &Path,
) -> Result<Vec<(String, PathBuf, String)>> {
    let exports = bundle_dir.join("exports");
    let mut events = Vec::new();
    for name in ["draft-paper.md", "draft-paper.findings.md"] {
        let source = exports.join(name);
        if source.exists() {
            let target = exports.join(format!("superseded-{name}"));
            fs::rename(&source, &target)
                .with_context(|| format!("supersede {}", source.display()))?;
            let hash = sha256_hex(&fs::read(&target)?);
            events.push(("research-paper-draft-superseded".to_string(), target, hash));
        }
    }
    Ok(events)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::runner::{AgentInfo, AgentResult, PipelineSession, PipelineType, ToolAccessLevel};

    use super::{
        paper_to_validate, supersede_draft_research_artifacts, write_draft_research_artifacts,
    };

    #[test]
    fn validator_text_prefers_assembled_chapter_paper_over_combiner_output() {
        let mut session = PipelineSession {
            id: "assembled-paper".to_string(),
            pipeline_type: PipelineType::Research,
            task: "topic".to_string(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        };
        let architecture = r#"{
            "locked": true,
            "generated_at": "now",
            "total_chapters": 1,
            "estimated_total_words": 1000,
            "chapters": [{
                "number": 1,
                "title": "Introduction",
                "assignedAgent": "introduction-writer",
                "wordTarget": 1000,
                "sections": ["Background"],
                "outputFile": "chapter-01.md"
            }],
            "writerMapping": {}
        }"#;
        session
            .agent_results
            .push((agent("dissertation-architect"), result(architecture)));
        session.agent_results.push((
            agent("chapter-writer-001-introduction"),
            result("Assembled chapter text judged by validator."),
        ));
        session.agent_results.push((
            agent("final-paper-combiner"),
            result("Different stored combiner output."),
        ));

        let judged = paper_to_validate(&session).unwrap();
        assert!(judged.contains("Assembled chapter text judged by validator."));
        assert!(!judged.contains("Different stored combiner output."));
    }

    fn agent(key: &str) -> AgentInfo {
        AgentInfo {
            key: key.to_string(),
            display_name: key.to_string(),
            model: "test".to_string(),
            phase: 1,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access_level: ToolAccessLevel::ReadOnly,
        }
    }

    fn result(output: &str) -> AgentResult {
        AgentResult {
            output: output.to_string(),
            tool_use_log: Vec::new(),
            tokens_in: None,
            tokens_out: None,
            cost_usd: 0.0,
            duration: std::time::Duration::ZERO,
            quality: None,
        }
    }

    #[test]
    fn successful_validation_moves_failed_drafts_to_superseded_names() {
        let temp = tempfile::tempdir().unwrap();
        write_draft_research_artifacts(temp.path(), "paper", "finding").unwrap();
        let events = supersede_draft_research_artifacts(temp.path()).unwrap();
        assert_eq!(events.len(), 2);
        assert!(!temp.path().join("exports/draft-paper.md").exists());
        assert!(!temp.path().join("exports/draft-paper.findings.md").exists());
        assert!(
            temp.path()
                .join("exports/superseded-draft-paper.md")
                .exists()
        );
        assert!(
            temp.path()
                .join("exports/superseded-draft-paper.findings.md")
                .exists()
        );
    }

    #[test]
    fn superseding_drafts_twice_is_a_noop() {
        let temp = tempfile::tempdir().unwrap();
        write_draft_research_artifacts(temp.path(), "paper", "finding").unwrap();
        let first = supersede_draft_research_artifacts(temp.path()).unwrap();
        let second = supersede_draft_research_artifacts(temp.path()).unwrap();

        assert_eq!(first.len(), 2);
        assert!(second.is_empty());
        assert!(
            temp.path()
                .join("exports/superseded-draft-paper.md")
                .exists()
        );
        assert!(
            temp.path()
                .join("exports/superseded-draft-paper.findings.md")
                .exists()
        );
    }
}
