use archon_core::skills::{SkillContext, SkillOutput, builtin::register_builtins};

#[test]
fn default_registry_includes_all_5_engineering_skills() {
    let registry = register_builtins();
    assert!(registry.resolve("grill-me").is_some());
    assert!(registry.resolve("grill-with-docs").is_some());
    assert!(registry.resolve("diagnose").is_some());
    assert!(registry.resolve("tdd").is_some());
    assert!(registry.resolve("zoom-out").is_some());
}

#[test]
fn engineering_skill_executes_emits_prompt() {
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("grill-me").unwrap();
    let out = skill.execute(&[], &ctx);
    assert!(matches!(out, SkillOutput::Prompt(_)));
}

#[test]
fn engineering_skill_uses_embedded_when_no_overrides() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: tmp.path().to_path_buf(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("grill-me").unwrap();
    let out = skill.execute(&[], &ctx);
    match out {
        SkillOutput::Prompt(body) => {
            assert!(
                body.contains("Interview me relentlessly"),
                "expected embedded body marker"
            );
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn engineering_skill_workdir_flat_override_wins() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skills_dir = tmp.path().join(".archon/skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(
        skills_dir.join("grill-me.md"),
        "---\nname: grill-me\ndescription: override\n---\nOVERRIDE_BODY_MARKER_XYZ123\n",
    )
    .unwrap();
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: tmp.path().to_path_buf(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("grill-me").unwrap();
    let out = skill.execute(&[], &ctx);
    match out {
        SkillOutput::Prompt(body) => {
            assert!(
                body.contains("OVERRIDE_BODY_MARKER_XYZ123"),
                "expected override body marker"
            );
            assert!(
                !body.contains("Interview me relentlessly"),
                "should NOT contain embedded marker"
            );
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn engineering_skill_workdir_subdir_override_wins() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skills_dir = tmp.path().join(".archon/skills/grill-me");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(
        skills_dir.join("SKILL.md"),
        "---\nname: grill-me\ndescription: subdir override\n---\nSUBDIR_OVERRIDE_BODY_ABC456\n",
    )
    .unwrap();
    let ctx = SkillContext {
        session_id: "test".into(),
        working_dir: tmp.path().to_path_buf(),
        model: "test".into(),
        agent_registry: None,
    };
    let registry = register_builtins();
    let skill = registry.resolve("grill-me").unwrap();
    let out = skill.execute(&[], &ctx);
    match out {
        SkillOutput::Prompt(body) => {
            assert!(
                body.contains("SUBDIR_OVERRIDE_BODY_ABC456"),
                "expected subdir override body marker"
            );
            assert!(
                !body.contains("Interview me relentlessly"),
                "should NOT contain embedded marker"
            );
        }
        _ => panic!("expected Prompt"),
    }
}
