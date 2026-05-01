use archon_core::skills::{SkillContext, SkillOutput, builtin::register_builtins};

#[test]
fn default_registry_includes_all_5_archon_skills() {
    let registry = register_builtins();
    assert!(registry.resolve("spec-to-tasks").is_some());
    assert!(registry.resolve("compose-pipeline").is_some());
    assert!(registry.resolve("ci-gate-walker").is_some());
    assert!(registry.resolve("setup-archon-skills").is_some());
    assert!(registry.resolve("write-a-skill").is_some());
}

#[test]
fn archon_skill_executes_emits_prompt() {
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("spec-to-tasks").unwrap();
    let out = skill.execute(&[], &ctx);
    assert!(matches!(out, SkillOutput::Prompt(_)));
}

#[test]
fn archon_skill_uses_embedded_when_no_overrides() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: tmp.path().to_path_buf(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("ci-gate-walker").unwrap();
    let out = skill.execute(&[], &ctx);
    match out {
        SkillOutput::Prompt(body) => {
            assert!(body.contains("ci-gate.sh"), "expected embedded body marker");
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn archon_skill_workdir_flat_override_wins() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skills_dir = tmp.path().join(".archon/skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(
        skills_dir.join("ci-gate-walker.md"),
        "---\nname: ci-gate-walker\ndescription: override\n---\nOVERRIDE_BODY_MARKER_ARCHON_XYZ\n",
    )
    .unwrap();
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: tmp.path().to_path_buf(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("ci-gate-walker").unwrap();
    let out = skill.execute(&[], &ctx);
    match out {
        SkillOutput::Prompt(body) => {
            assert!(
                body.contains("OVERRIDE_BODY_MARKER_ARCHON_XYZ"),
                "expected override body marker"
            );
            assert!(
                !body.contains("ci-gate.sh"),
                "should NOT contain embedded marker"
            );
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn archon_skill_workdir_subdir_override_wins() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skills_dir = tmp.path().join(".archon/skills/ci-gate-walker");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(
        skills_dir.join("SKILL.md"),
        "---\nname: ci-gate-walker\ndescription: subdir override\n---\nSUBDIR_OVERRIDE_ARCHON_ABC\n",
    )
    .unwrap();
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: tmp.path().to_path_buf(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("ci-gate-walker").unwrap();
    let out = skill.execute(&[], &ctx);
    match out {
        SkillOutput::Prompt(body) => {
            assert!(
                body.contains("SUBDIR_OVERRIDE_ARCHON_ABC"),
                "expected subdir override body marker"
            );
            assert!(
                !body.contains("ci-gate.sh"),
                "should NOT contain embedded marker"
            );
        }
        _ => panic!("expected Prompt"),
    }
}
