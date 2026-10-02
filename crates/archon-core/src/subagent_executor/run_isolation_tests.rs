use super::*;

fn request() -> SubagentRequest {
    SubagentRequest {
        prompt: "p".into(),
        model: None,
        allowed_tools: Vec::new(),
        max_turns: 1,
        timeout_secs: 1,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    }
}

#[test]
fn inherited_extra_dirs_preserve_parent_project_when_cwd_changes() {
    let parent = ToolContext {
        working_dir: PathBuf::from("/project-1"),
        extra_dirs: vec![PathBuf::from("assets"), PathBuf::from("/shared")],
        ..ToolContext::default()
    };

    let dirs = inherited_extra_dirs(&parent, Path::new("/repo"));

    assert_eq!(
        dirs,
        vec![
            PathBuf::from("/project-1"),
            PathBuf::from("/project-1/assets"),
            PathBuf::from("/shared"),
        ]
    );
}

#[test]
fn inherited_extra_dirs_do_not_duplicate_child_working_dir() {
    let parent = ToolContext {
        working_dir: PathBuf::from("/repo"),
        extra_dirs: vec![PathBuf::from("/repo")],
        ..ToolContext::default()
    };

    assert!(inherited_extra_dirs(&parent, Path::new("/repo")).is_empty());
}

#[test]
fn an_unknown_request_value_is_refused_naming_value_and_source() {
    let request = SubagentRequest {
        isolation: Some("sealed-ish".into()),
        ..request()
    };
    let error = requested(&request, None).expect_err("unknown value");
    assert!(error.contains("'sealed-ish'"), "{error}");
    assert!(
        error.contains("the spawn request's isolation field"),
        "{error}"
    );
}

#[test]
fn an_unknown_definition_value_is_refused_naming_the_definition() {
    let request = request();
    let definition = CustomAgentDefinition {
        agent_type: "auditor".into(),
        isolation: Some("sealed-ish".into()),
        ..CustomAgentDefinition::default()
    };
    let error = requested(&request, Some(&definition)).expect_err("unknown value");
    assert!(error.contains("'sealed-ish'"), "{error}");
    assert!(error.contains("agent definition 'auditor'"), "{error}");
}

#[test]
fn the_boundary_value_parses_from_the_request() {
    let request = SubagentRequest {
        isolation: Some(Isolation::WorkspaceBoundary.as_str().into()),
        ..request()
    };
    assert_eq!(
        requested(&request, None),
        Ok(Some(Isolation::WorkspaceBoundary))
    );
}

#[test]
fn a_relative_read_root_is_refused() {
    let error = WorkspaceBoundary::new(&["prds/spec.md".into()]).expect_err("relative");
    assert!(error.contains("prds/spec.md"), "{error}");
}

#[test]
fn boundary_write_roots_always_hold_the_working_dir_and_never_read_roots() {
    let temp = tempfile::tempdir().unwrap();
    let read = temp.path().join("spec.md");
    std::fs::write(&read, "spec").unwrap();
    let boundary = WorkspaceBoundary::new(&[read.display().to_string()]).unwrap();
    let work = temp.path().join("work");

    let writes = boundary.write_roots(Vec::new(), &work);
    assert_eq!(writes, vec![work.clone()]);
    let reads = boundary.extra_dirs(&writes, &work);
    assert_eq!(reads, vec![read], "the read root is readable, not writable");
}

#[test]
fn boundary_extra_dirs_leave_out_a_missing_root() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    let boundary = WorkspaceBoundary::new(&[missing.display().to_string()]).unwrap();
    assert!(boundary.extra_dirs(&[], temp.path()).is_empty());
}
