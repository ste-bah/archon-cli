//! Smoke test: validate coding pipeline.toml against real .md files.

use archon_pipeline::agent_loader::load_coding_agents;
use archon_pipeline::manifest::{load_manifest, validate};
fn project_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn smoke_coding_pipeline_toml_parses() {
    let manifest_path =
        project_root().join(".archon/agents/coding-pipeline/pipeline.toml");
    let manifest = load_manifest(&manifest_path).expect("should parse coding pipeline.toml");

    assert_eq!(manifest.pipeline.name, "coding");
    assert_eq!(manifest.phases.len(), 6, "expected 6 phases");
    println!("Loaded {} agents from manifest", manifest.agents.len());
    assert!(
        manifest.agents.len() >= 48,
        "expected 48+ agents, got {}",
        manifest.agents.len()
    );

    // Verify phase reviewers are critical
    for agent in &manifest.agents {
        if agent.key.contains("reviewer") || agent.key == "recovery-agent" {
            assert!(agent.critical, "agent '{}' should be critical", agent.key);
        }
    }
}

#[test]
fn smoke_coding_manifest_cross_references_md_files() {
    let manifest_path =
        project_root().join(".archon/agents/coding-pipeline/pipeline.toml");
    let manifest = load_manifest(&manifest_path).expect("parse manifest");

    let coding_dir = project_root().join(".archon/agents/coding-pipeline");
    let agents = load_coding_agents(&coding_dir).expect("load coding agents");
    let agent_keys: Vec<String> = agents.iter().map(|a| a.key.clone()).collect();

    let warnings = validate(&manifest, &agent_keys);
    println!("Validation warnings: {:?}", warnings);
    // Some warnings are expected (orphaned .md files, agents without .md files)
    // but there should be no more than a handful
    assert!(warnings.len() < 10, "too many warnings: {:?}", warnings);
}
