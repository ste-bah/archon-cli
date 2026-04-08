//! Smoke test: load real agent .md files from disk.

use archon_pipeline::agent_loader::{load_coding_agents, load_research_agents, parse_frontmatter};
fn project_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .unwrap()
        .parent() // project root
        .unwrap()
        .to_path_buf()
}

#[test]
fn smoke_parse_frontmatter_real_file() {
    let coding_dir = project_root().join(".archon/agents/coding-pipeline");
    let content = std::fs::read_to_string(coding_dir.join("task-analyzer.md"))
        .expect("read task-analyzer.md");
    let (yaml, body) = parse_frontmatter(&content).expect("parse frontmatter");
    assert_eq!(yaml["name"].as_str(), Some("task-analyzer"));
    assert!(body.len() > 100, "body too short: {} chars", body.len());
    assert!(
        !body.contains("\n---\n"),
        "body should not contain frontmatter delimiters"
    );
    println!(
        "parse_frontmatter: name={:?}, body_len={}",
        yaml["name"].as_str(),
        body.len()
    );
}

#[test]
fn smoke_load_coding_agents_real_dir() {
    let coding_dir = project_root().join(".archon/agents/coding-pipeline");
    let agents = load_coding_agents(&coding_dir).expect("load coding agents");
    println!("Loaded {} coding agents", agents.len());
    assert!(
        agents.len() >= 40,
        "expected 40+ coding agents, got {}",
        agents.len()
    );
    for a in &agents[..3] {
        println!(
            "  key={}, name={}, body_len={}",
            a.key,
            a.name,
            a.prompt_body.len()
        );
        assert!(!a.name.is_empty());
        assert!(!a.prompt_body.is_empty());
    }
}

#[test]
fn smoke_load_research_agents_real_dir() {
    let research_dir = project_root().join(".archon/agents/phdresearch");
    let agents = load_research_agents(&research_dir).expect("load research agents");
    println!("Loaded {} research agents", agents.len());
    assert!(
        agents.len() >= 40,
        "expected 40+ research agents, got {}",
        agents.len()
    );
    for a in &agents[..3] {
        println!(
            "  key={}, name={}, body_len={}",
            a.key,
            a.name,
            a.prompt_body.len()
        );
        assert!(!a.name.is_empty());
        assert!(!a.prompt_body.is_empty());
    }
}
