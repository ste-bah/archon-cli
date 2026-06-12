//! TASK-WC-009 — end-to-end acceptance tests (AC-WC-001..014) for the
//! coordinated implementation fanout, backed by real tempfile git repos.
//! Deterministic, no LLM/network. Restricted-cargo only.

mod wc_common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_workflow::write_coordinator::ManifestStatus;
use archon_workflow::write_coordinator::SerialFallbackReason;
use archon_workflow::write_coordinator::coordinator::FanoutError;
use archon_workflow::write_coordinator::patch_apply::{ApplyError, with_repo_lock};
use archon_workflow::write_coordinator::patch_manifest::PatchError;

use wc_common::{MockAgentRunner, git, init_git_repo, init_plain_dir, run_coordinated};

fn cfg() -> archon_workflow::write_coordinator::WriteCoordinatorConfig {
    archon_workflow::write_coordinator::WriteCoordinatorConfig::default()
}

#[test]
fn ac_wc_001_two_disjoint_files_one_wave() {
    let repo = init_git_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"]), ("b", &["src/b.rs"])];
    let h = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::writing("// w\n"),
        &cfg(),
    )
    .unwrap();
    assert_eq!(h.outcome.waves.len(), 1);
    assert_eq!(h.outcome.waves[0].items.len(), 2);
    assert_eq!(
        h.outcome.item_status.get("implement-0"),
        Some(&ManifestStatus::Applied)
    );
    assert_eq!(
        h.outcome.item_status.get("implement-1"),
        Some(&ManifestStatus::Applied)
    );
    assert!(repo.path().join("src/a.rs").exists());
    assert!(repo.path().join("src/b.rs").exists());
}

#[test]
fn ac_wc_002_two_overlapping_files_serialize() {
    let repo = init_git_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"]), ("b", &["src/a.rs"])];
    let h = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::writing("// w\n"),
        &cfg(),
    )
    .unwrap();
    assert_eq!(h.outcome.waves.len(), 2, "overlapping targets serialize");
    assert_eq!(h.outcome.waves[0].items, vec!["implement-0".to_string()]);
    assert_eq!(h.outcome.waves[1].items, vec!["implement-1".to_string()]);
}

#[test]
fn ac_wc_003_undeclared_write_contained() {
    let repo = init_git_repo();
    let action: wc_common::AgentAction = Arc::new(|root, item, declared| {
        for f in declared {
            std::fs::write(root.join(f), format!("// {item}\n")).unwrap();
        }
        // An undeclared write inside the isolated workspace.
        std::fs::write(root.join("src/SNEAKY.rs"), "// undeclared\n").unwrap();
        format!("implemented {item}")
    });
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let h = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::with_action(action),
        &cfg(),
    )
    .unwrap();
    assert_eq!(
        h.outcome.item_status.get("implement-0"),
        Some(&ManifestStatus::Applied)
    );
    // The undeclared write never reaches canonical (scoped capture + declared-only apply).
    assert!(
        !repo.path().join("src/SNEAKY.rs").exists(),
        "undeclared write must not reach canonical"
    );
    assert!(repo.path().join("src/a.rs").exists());
}

#[test]
fn ac_wc_004_direct_canonical_mutation_fails_wave() {
    let repo = init_git_repo();
    let canonical = repo.path().to_path_buf();
    let action: wc_common::AgentAction = Arc::new(move |root, item, declared| {
        for f in declared {
            std::fs::write(root.join(f), format!("// {item}\n")).unwrap();
        }
        std::fs::write(canonical.join("src/a.rs"), "// TAMPER\n").unwrap();
        format!("tampered {item}")
    });
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let h = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::with_action(action),
        &cfg(),
    )
    .unwrap();
    assert_eq!(h.outcome.waves.len(), 1);
    assert!(
        h.outcome.waves[0]
            .failure
            .as_deref()
            .is_some_and(|f| f.contains("CanonicalMutation"))
    );
    assert_ne!(
        h.outcome.item_status.get("implement-0"),
        Some(&ManifestStatus::Applied)
    );
}

#[test]
fn ac_wc_005_dirty_tracked_baseline_reproduced() {
    // AC-WC-005 is about the dirty tracked overlay being VISIBLE in the isolated
    // worktree before the agent runs. Record what the agent saw.
    let repo = init_git_repo();
    std::fs::write(repo.path().join("src/seed.rs"), "// DIRTY\n").unwrap();
    let seen = Arc::new(std::sync::Mutex::new(None::<String>));
    let seen2 = seen.clone();
    let action: wc_common::AgentAction = Arc::new(move |root, item, declared| {
        let content = std::fs::read_to_string(root.join(&declared[0])).unwrap();
        *seen2.lock().unwrap() = Some(content.clone());
        std::fs::write(root.join(&declared[0]), format!("{content}// {item}\n")).unwrap();
        format!("implemented {item}")
    });
    let targets: &[(&str, &[&str])] = &[("a", &["src/seed.rs"])];
    let _ = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::with_action(action),
        &cfg(),
    );
    let observed = seen.lock().unwrap().clone().expect("agent ran");
    assert!(
        observed.contains("DIRTY"),
        "isolated worktree must reproduce dirty tracked content, saw: {observed}"
    );
}

#[test]
fn ac_wc_006_declared_untracked_target_reproduced() {
    // AC-WC-006: a declared untracked target is VISIBLE in the isolated worktree.
    let repo = init_git_repo();
    std::fs::write(repo.path().join("src/untracked.rs"), "// UNTRACKED\n").unwrap();
    let seen = Arc::new(std::sync::Mutex::new(None::<String>));
    let seen2 = seen.clone();
    let action: wc_common::AgentAction = Arc::new(move |root, item, declared| {
        let content = std::fs::read_to_string(root.join(&declared[0])).unwrap();
        *seen2.lock().unwrap() = Some(content.clone());
        std::fs::write(root.join(&declared[0]), format!("{content}// {item}\n")).unwrap();
        format!("implemented {item}")
    });
    let targets: &[(&str, &[&str])] = &[("a", &["src/untracked.rs"])];
    let _ = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::with_action(action),
        &cfg(),
    );
    let observed = seen.lock().unwrap().clone().expect("agent ran");
    assert!(
        observed.contains("UNTRACKED"),
        "isolated worktree must reproduce declared untracked content"
    );
}

#[test]
fn ac_wc_007_persistence_layout() {
    let repo = init_git_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let h = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::writing("// w\n"),
        &cfg(),
    )
    .unwrap();
    let stage_root = h.run_root.join("write-coordination/stages/implement");
    assert!(
        stage_root.join("manifests/implement-0.json").exists(),
        "manifest json"
    );
    assert!(
        stage_root.join("patches/implement-0.patch").exists(),
        "patch file"
    );
    assert!(stage_root.join("apply/0.json").exists(), "apply record");
    assert!(
        stage_root.join("tests/0.json").exists(),
        "wave verify record"
    );
}

#[test]
fn ac_wc_008_cross_process_repo_lock() {
    if std::env::var("ARCHON_WC_LOCK_SUBPROCESS").as_deref() == Ok("hold") {
        let canonical =
            std::path::PathBuf::from(std::env::var("ARCHON_WC_LOCK_CANONICAL").unwrap());
        with_repo_lock(&canonical, || {
            std::thread::sleep(Duration::from_secs(3));
            Ok::<_, ApplyError>(())
        })
        .unwrap();
        return;
    }
    let repo = init_git_repo();
    let me = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(&me)
        .arg("ac_wc_008_cross_process_repo_lock")
        .arg("--exact")
        .env("ARCHON_WC_LOCK_SUBPROCESS", "hold")
        .env("ARCHON_WC_LOCK_CANONICAL", repo.path())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(700));
    let start = Instant::now();
    with_repo_lock(repo.path(), || Ok::<_, ApplyError>(())).unwrap();
    let waited = start.elapsed();
    assert!(
        waited >= Duration::from_secs(2),
        "parent acquired lock without waiting: {waited:?}"
    );
    child.wait().unwrap();
}

#[test]
fn ac_wc_009_status_displays_coordinator_state() {
    use archon_workflow::write_coordinator::status::{read_status, render_compact};
    let repo = init_git_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"]), ("b", &["src/b.rs"])];
    let h = run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::writing("// w\n"),
        &cfg(),
    )
    .unwrap();
    let status = read_status(&h.store, &h.run_id, "implement")
        .unwrap()
        .expect("status present");
    let block = render_compact(&status);
    assert_eq!(block.lines().count(), 6);
    assert!(block.starts_with("write_coordination: enabled\n"));
    assert!(block.contains("apply: applied"));
}

#[test]
fn ac_wc_010_resume_skips_accepted() {
    use archon_workflow::classify_resume;
    use archon_workflow::write_coordinator::patch_manifest::{
        PATCH_MANIFEST_SCHEMA, PatchManifest, persist_manifest_status_update,
    };
    let dir = tempfile::tempdir().unwrap();
    let mk = |item: &str, status: ManifestStatus| PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.into(),
        run_id: "run1".into(),
        stage_id: "implement".into(),
        item_id: item.into(),
        baseline_commit: "abc".into(),
        patch_path: std::path::PathBuf::from("x.patch"),
        declared_target_files: vec!["src/a.rs".into()],
        changed_files: vec!["src/a.rs".into()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: Default::default(),
        post_hashes: Default::default(),
        verify_command: None,
        agent_artifact_path: None,
        status,
    };
    for (item, st) in [
        ("a", ManifestStatus::Applied),
        ("b", ManifestStatus::IdempotentNoop),
    ] {
        let m = mk(item, st);
        persist_manifest_status_update(dir.path(), "run1", "implement", &m.item_id, &m).unwrap();
    }
    let items = vec!["a".to_string(), "b".to_string()];
    let c = classify_resume(dir.path(), "implement", &items);
    assert_eq!(
        c.skip,
        vec!["a".to_string(), "b".to_string()],
        "accepted items are skipped"
    );
    assert!(c.reexecute.is_empty(), "no accepted item is re-applied");
}

#[test]
fn ac_wc_011_non_git_root_serial_fallback() {
    let dir = init_plain_dir();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let h = run_coordinated(
        dir.path(),
        targets,
        &MockAgentRunner::writing("// w\n"),
        &cfg(),
    )
    .unwrap();
    assert_eq!(
        h.outcome.serial_fallback,
        Some(SerialFallbackReason::NonGitRoot)
    );
    assert!(h.outcome.waves.is_empty());
}

#[test]
fn ac_wc_012_non_implementation_fanout_untouched() {
    // A non-implementation fanout never produces write-coordination artifacts.
    let repo = init_git_repo();
    let h = run_coordinated(
        repo.path(),
        &[("a", &["src/a.rs"])],
        &MockAgentRunner::writing("// w\n").no_boundary(),
        &cfg(),
    )
    .unwrap();
    // Boundary-unavailable runner forces serial fallback; no coordinated apply ran.
    assert_eq!(
        h.outcome.serial_fallback,
        Some(SerialFallbackReason::BoundaryUnavailable)
    );
    assert!(
        !h.run_root.join("write-coordination").exists(),
        "no coordinator artifacts written"
    );
}

#[test]
fn ac_wc_013_subagent_concurrency_cap_respected() {
    let repo = init_git_repo();
    let targets: &[(&str, &[&str])] = &[
        ("a", &["src/a.rs"]),
        ("b", &["src/b.rs"]),
        ("c", &["src/c.rs"]),
        ("d", &["src/d.rs"]),
    ];
    let runner = MockAgentRunner::writing("// w\n").cap(2);
    let h = run_coordinated(repo.path(), targets, &runner, &cfg()).unwrap();
    // 4 disjoint items, cap 2 -> 2 waves of 2 (width capped, not 4-wide).
    assert_eq!(
        h.outcome.waves.len(),
        2,
        "cap 2 splits 4 disjoint items into 2 waves"
    );
    for wave in &h.outcome.waves {
        assert!(wave.items.len() <= 2, "no wave exceeds the cap");
    }
}

#[test]
fn ac_wc_014_file_size_byte_budget() {
    let limit = 256u64;
    let make = |bytes: usize| -> wc_common::AgentAction {
        Arc::new(move |root, _item, declared| {
            std::fs::write(root.join(&declared[0]), vec![b'x'; bytes]).unwrap();
            "implemented".into()
        })
    };
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let c = archon_workflow::write_coordinator::WriteCoordinatorConfig {
        max_file_bytes: limit,
        ..cfg()
    };
    // limit - 1 and exactly limit succeed (check is strictly greater-than).
    for ok_size in [limit as usize - 1, limit as usize] {
        let repo = init_git_repo();
        run_coordinated(
            repo.path(),
            targets,
            &MockAgentRunner::with_action(make(ok_size)),
            &c,
        )
        .unwrap_or_else(|e| panic!("size {ok_size} should apply: {e}"));
    }
    // limit + 1 is rejected with FileTooLarge.
    let repo = init_git_repo();
    match run_coordinated(
        repo.path(),
        targets,
        &MockAgentRunner::with_action(make(limit as usize + 1)),
        &c,
    ) {
        Ok(_) => panic!("oversize file must be rejected"),
        Err(FanoutError::Patch(PatchError::FileTooLarge { .. })) => {}
        Err(e) => panic!("expected FileTooLarge, got {e}"),
    }
    let _ = git; // silence unused import on some cfgs
}
