use super::*;

#[test]
fn flat_file_loader_finds_basic_agent() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("over-engineering-therapist.md"),
        "---\nname: over-engineering-therapist\ndescription: Therapeutic code reviewer\n\
             tools: Read, Grep, Glob, Bash\nmodel: sonnet\ncolor: teal\n---\n\n\
             # Over-Engineering Therapist\n\nYou are a specialized therapeutic code reviewer.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);
    let a = &agents[0];
    assert_eq!(a.agent_type, "over-engineering-therapist");
    assert_eq!(a.description, "Therapeutic code reviewer");
    assert!(
        a.system_prompt
            .contains("You are a specialized therapeutic code reviewer")
    );
    assert_eq!(a.model.as_deref(), Some("sonnet"));
    assert_eq!(a.color.as_deref(), Some("teal"));
    assert_eq!(a.source, AgentSource::Project);
}

#[test]
fn flat_file_loader_recursive_subdirs() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(agents_dir.join("templates")).unwrap();
    fs::write(
        agents_dir.join("templates/sub-agent.md"),
        "---\nname: sub-agent\ndescription: A subdirectory agent\n---\n\nSub agent body.\n",
    )
    .unwrap();
    fs::write(
        agents_dir.join("top-agent.md"),
        "---\nname: top-agent\ndescription: A top-level agent\n---\n\nTop agent body.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
    assert!(
        names.contains(&"sub-agent"),
        "sub-agent from subdir must be found; got {:?}",
        names
    );
    assert!(names.contains(&"top-agent"), "top-agent must be found");
    assert_eq!(agents.len(), 2);
}

#[test]
fn flat_file_loader_skips_custom_subdir() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(agents_dir.join("custom")).unwrap();
    fs::write(
        agents_dir.join("custom/should-skip.md"),
        "---\nname: should-skip\ndescription: This is in custom/\n---\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        agents_dir.join("top.md"),
        "---\nname: top\ndescription: Top-level agent\n---\n\nTop body.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
    assert!(names.contains(&"top"), "top agent should be loaded");
    assert!(!names.contains(&"should-skip"), "custom/ must be skipped");
    assert_eq!(agents.len(), 1);
}

#[test]
fn flat_file_loader_skips_no_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("README.md"),
        "# README\n\nThis is not an agent — just a readme.\n",
    )
    .unwrap();
    fs::write(
        agents_dir.join("real-agent.md"),
        "---\nname: real-agent\ndescription: A real one\n---\n\nReal body.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
    assert!(names.contains(&"real-agent"), "real-agent should be loaded");
    assert!(
        !names.contains(&"README"),
        "README (no frontmatter) must be skipped"
    );
    assert_eq!(agents.len(), 1);
}

#[test]
fn flat_file_loader_malformed_frontmatter_logs_skips() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("broken.md"),
        "---\n{this is not valid yaml]]]\n---\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        agents_dir.join("good.md"),
        "---\nname: good\ndescription: Works fine\n---\n\nGood body.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
    assert!(names.contains(&"good"), "good agent must still be loaded");
    assert!(!names.contains(&"broken"), "malformed YAML must be skipped");
    assert_eq!(agents.len(), 1);
}

#[test]
fn flat_file_loader_missing_root_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does-not-exist");
    let agents = load_flat_file_agents(&nonexistent, AgentSource::Project).unwrap();
    assert!(agents.is_empty());
}

#[test]
fn flat_file_loader_tools_csv() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("tooled.md"),
        "---\nname: tooled\ndescription: Has tools\n\
             tools: Read, Grep, Glob, Bash\n---\n\nBody.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    let tools = agents[0].allowed_tools.as_ref().unwrap();
    assert_eq!(tools, &vec!["Read", "Grep", "Glob", "Bash"]);
}

#[test]
fn flat_file_loader_tools_yaml_array() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("tooled.yaml.md"),
        "---\nname: tooled\ndescription: Has tools as array\n\
             tools:\n  - Read\n  - Grep\n  - Glob\n  - Bash\n---\n\nBody.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    let tools = agents[0].allowed_tools.as_ref().unwrap();
    assert_eq!(tools, &vec!["Read", "Grep", "Glob", "Bash"]);
}

#[test]
fn flat_file_loader_runtime_frontmatter_fields() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("configured.md"),
        "---\nname: configured\ndescription: Has runtime metadata\nversion: 2.1.0\n\
         model: sonnet\neffort: high\npermissions:\n  default_mode: auto\n---\n\nBody.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);
    let agent = &agents[0];
    assert_eq!(agent.model.as_deref(), Some("sonnet"));
    assert_eq!(agent.effort.as_deref(), Some("high"));
    assert_eq!(
        agent.permission_mode,
        Some(crate::agents::definition::PermissionMode::Auto)
    );
    assert_eq!(agent.meta.version, "2.1.0");
}

#[test]
fn flat_file_loader_filename_stem_fallback() {
    let tmp = TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("nameless.md"),
        "---\ndescription: No name field here\n---\n\nBody text.\n",
    )
    .unwrap();

    let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_type, "nameless");
    assert_eq!(agents[0].description, "No name field here");
}
