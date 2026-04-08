//! Smoke test: validate research pipeline.toml against real .md files.

use archon_pipeline::agent_loader::load_research_agents;
use archon_pipeline::manifest::{load_manifest, validate};
use std::path::Path;

const PROJECT_ROOT: &str = "/home/unixdude/Archon-projects/archon/project-work/archon-cli";

#[test]
fn smoke_research_pipeline_toml_parses() {
    let manifest_path = Path::new(PROJECT_ROOT).join(".archon/agents/phdresearch/pipeline.toml");
    let manifest = load_manifest(&manifest_path).expect("should parse research pipeline.toml");

    assert_eq!(manifest.pipeline.name, "phdresearch");
    assert_eq!(manifest.phases.len(), 7, "expected 7 phases");
    assert_eq!(manifest.agents.len(), 46, "expected 46 agents, got {}", manifest.agents.len());

    // Phase 6 and 7 should have full tool_access
    assert_eq!(manifest.phases[5].tool_access.as_deref(), Some("full"));
    assert_eq!(manifest.phases[6].tool_access.as_deref(), Some("full"));
}

#[test]
fn smoke_research_manifest_cross_references_md_files() {
    let manifest_path = Path::new(PROJECT_ROOT).join(".archon/agents/phdresearch/pipeline.toml");
    let manifest = load_manifest(&manifest_path).expect("parse manifest");

    let research_dir = Path::new(PROJECT_ROOT).join(".archon/agents/phdresearch");
    let agents = load_research_agents(&research_dir).expect("load research agents");
    let agent_keys: Vec<String> = agents.iter().map(|a| a.key.clone()).collect();

    let warnings = validate(&manifest, &agent_keys);
    println!("Research validation warnings: {:?}", warnings);
    // Expect very few warnings since research agents haven't been renamed
    assert!(warnings.len() < 5, "too many warnings: {:?}", warnings);
}
