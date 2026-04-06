use archon_core::skills::SkillContext;
use archon_core::skills::SkillOutput;
use archon_core::skills::builtin::register_builtins;

fn make_ctx() -> SkillContext {
    SkillContext {
        session_id: "test-session".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        model: "test-model".to_string(),
    }
}

#[test]
fn all_expanded_skills_registered() {
    let reg = register_builtins();
    let all = reg.list_all();
    // Builtins (13) + expanded (33) = 46 unique skills
    assert!(
        all.len() >= 45,
        "Expected at least 45 skills total, got {}: {:?}",
        all.len(),
        all
    );
}

#[test]
fn context_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("context").is_some(),
        "context skill should be registered"
    );
}

#[test]
fn copy_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("copy").is_some(),
        "copy skill should be registered"
    );
}

#[test]
fn tag_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("tag").is_some(),
        "tag skill should be registered"
    );
}

#[test]
fn rename_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("rename").is_some(),
        "rename skill should be registered"
    );
}

#[test]
fn restore_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("restore").is_some(),
        "restore skill should be registered"
    );
}

#[test]
fn btw_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("btw").is_some(),
        "btw skill should be registered"
    );
}

#[test]
fn bug_skill_produces_output() {
    let reg = register_builtins();
    let skill = reg.resolve("bug").expect("bug skill should be registered");
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Text(text) | SkillOutput::Markdown(text) => {
            assert!(!text.is_empty(), "bug skill output should be non-empty");
        }
        SkillOutput::Error(_) => panic!("bug skill should not return error"),
        SkillOutput::Prompt(_) => panic!("bug skill should not return prompt"),
    }
}

// ---------------------------------------------------------------------------
// New expanded skills existence checks
// ---------------------------------------------------------------------------

#[test]
fn resume_skill_exists() {
    let reg = register_builtins();
    assert!(reg.resolve("resume").is_some());
}

#[test]
fn fork_skill_exists() {
    let reg = register_builtins();
    assert!(
        reg.resolve("fork").is_some(),
        "fork (conversation branch) skill should be registered"
    );
}

#[test]
fn git_branch_skill_not_overwritten() {
    let reg = register_builtins();
    // git branch from builtin.rs should still exist and not be overwritten
    let skill = reg
        .resolve("branch")
        .expect("git branch skill must survive");
    // Git branch skill returns branch listing, not conversation fork
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Text(t) => {
            // Git branch skill output should NOT contain "fork"
            assert!(
                !t.contains("Forking conversation"),
                "git /branch should not be the conversation fork skill"
            );
        }
        _ => {} // errors are fine (no git repo in test dir)
    }
}

#[test]
fn recall_skill_exists() {
    let reg = register_builtins();
    assert!(reg.resolve("recall").is_some());
}

#[test]
fn agents_skill_exists() {
    let reg = register_builtins();
    assert!(reg.resolve("agents").is_some());
}

#[test]
fn theme_skill_accepts_valid_input() {
    let reg = register_builtins();
    let skill = reg.resolve("theme").expect("theme");
    let ctx = make_ctx();
    let output = skill.execute(&["dark".to_string()], &ctx);
    match output {
        SkillOutput::Text(t) => assert!(t.contains("dark")),
        _ => panic!("expected text output"),
    }
}

#[test]
fn theme_skill_rejects_invalid() {
    let reg = register_builtins();
    let skill = reg.resolve("theme").expect("theme");
    let ctx = make_ctx();
    let output = skill.execute(&["neon".to_string()], &ctx);
    match output {
        SkillOutput::Error(e) => assert!(e.contains("Unknown theme")),
        _ => panic!("expected error for invalid theme"),
    }
}

#[test]
fn sandbox_shows_usage_message() {
    let reg = register_builtins();
    let skill = reg.resolve("sandbox").expect("sandbox");
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Text(t) => assert!(t.contains("--sandbox") && t.contains("config.toml")),
        _ => panic!("expected sandbox usage message"),
    }
}

#[test]
fn schedule_shows_usage_message() {
    let reg = register_builtins();
    let skill = reg.resolve("schedule").expect("schedule");
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Text(t) => assert!(t.contains("CronCreate")),
        _ => panic!("expected schedule usage message"),
    }
}

#[test]
fn remote_control_shows_usage_message() {
    let reg = register_builtins();
    let skill = reg.resolve("remote-control").expect("remote-control");
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Text(t) => assert!(t.contains("archon remote") || t.contains("archon serve")),
        _ => panic!("expected remote control usage message"),
    }
}

#[test]
fn logout_skill_exists() {
    let reg = register_builtins();
    assert!(reg.resolve("logout").is_some());
}

#[test]
fn init_skill_exists() {
    let reg = register_builtins();
    assert!(reg.resolve("init").is_some());
}

#[test]
fn add_dir_skill_requires_args() {
    let reg = register_builtins();
    let skill = reg.resolve("add-dir").expect("add-dir");
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Error(e) => assert!(e.contains("Usage")),
        _ => panic!("expected error when no args"),
    }
}

#[test]
fn recall_skill_requires_query() {
    let reg = register_builtins();
    let skill = reg.resolve("recall").expect("recall");
    let ctx = make_ctx();
    let output = skill.execute(&[], &ctx);
    match output {
        SkillOutput::Error(e) => assert!(e.contains("Usage")),
        _ => panic!("expected error when no query"),
    }
}

