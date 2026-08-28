use super::*;

fn identity(suffix: &str) -> RuntimeIdentity {
    RuntimeIdentity {
        source_revision: format!("source-{suffix}"),
        binary_sha256: format!("binary-{suffix}"),
        binary_revision: format!("revision-{suffix}"),
        script_digest: format!("script-{suffix}"),
        catalog_digest: format!("catalog-{suffix}"),
    }
}

#[test]
fn missing_synthetic_clearance_refuses_external_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let error = read_clearance(&temp.path().join("missing.json")).unwrap_err();
    assert!(error.contains("synthetic clearance"), "{error}");
}

#[test]
fn mismatched_synthetic_identity_refuses_external_preflight() {
    let clearance = SyntheticClearance {
        schema_version: 1,
        identity: identity("old"),
        fixture_digest: synthetic_fixture_digest(),
        evidence_manifest_digest: "manifest".into(),
    };
    let error = require_clearance_identity(&clearance, &identity("new")).unwrap_err();
    assert!(error.contains("differs"), "{error}");
}

#[test]
fn external_command_allowlist_refuses_implementation() {
    let error = require_external_action(&ProofAction::ImplementSynthetic {
        tasks: "tasks/PRD-X".into(),
    })
    .unwrap_err();
    assert!(error.contains("decomposition-only"), "{error}");
}

#[test]
fn synthetic_implementation_uses_default_v3_surface() {
    let args = ProofAction::ImplementSynthetic {
        tasks: "tasks/PRD-X".into(),
    }
    .args();
    assert_eq!(&args[..4], ["workflow", "run", "--live", "--yes"]);
    assert!(!args.iter().any(|arg| arg == "--decomposed"));
}

#[test]
fn protected_snapshot_detects_any_byte_change() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("one.txt"), b"before").unwrap();
    let before = snapshot_tree(temp.path()).unwrap();
    std::fs::write(temp.path().join("one.txt"), b"after").unwrap();
    let after = snapshot_tree(temp.path()).unwrap();
    assert!(require_unchanged(&before, &after).is_err());
}

#[test]
fn evidence_manifest_is_sorted_deterministic_and_self_excluding() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("b.txt"), b"b").unwrap();
    std::fs::write(temp.path().join("a.txt"), b"a").unwrap();
    std::fs::write(temp.path().join("manifest.json"), b"old").unwrap();
    let first = build_evidence_manifest("synthetic", identity("same"), temp.path()).unwrap();
    let second = build_evidence_manifest("synthetic", identity("same"), temp.path()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.entries[0].relative_path, "a.txt");
    assert!(
        !first
            .entries
            .iter()
            .any(|entry| entry.relative_path == "manifest.json")
    );
}

#[test]
fn started_run_id_parser_requires_persisted_fixed_prefix() {
    assert_eq!(
        parse_started_run_id("noise\nFixed decomposition started: wf-123\n").unwrap(),
        "wf-123"
    );
    assert!(parse_started_run_id("completed without id").is_err());
}

#[test]
fn observe_config_requires_explicit_project_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    assert!(require_observe_config(temp.path()).is_err());
    std::fs::create_dir_all(temp.path().join(".archon")).unwrap();
    std::fs::write(
        temp.path().join(".archon/config.toml"),
        "[workflow]\ngate_mode = \"observe\"\n",
    )
    .unwrap();
    require_observe_config(temp.path()).unwrap();
}

#[test]
fn process_inventory_distinguishes_idle_tui_from_work_and_toolchain() {
    let inventory = classify_process_inventory(
        "101 1 /opt/archon /opt/archon --dangerously-skip-permissions\n\
         102 1 /opt/archon /opt/archon workflow status wf-x\n\
         103 1 /usr/bin/cargo cargo test\n\
         104 1 /tmp/workflow_probe /tmp/target/debug/deps/workflow_probe --test-threads=1\n\
         105 1 /usr/bin/ld ld -o binary\n",
        999,
    )
    .unwrap();
    assert_eq!(inventory.idle_tui_count, 1);
    assert_eq!(inventory.conflicts.len(), 4, "{inventory:#?}");
}

#[test]
fn preflight_refuses_active_work_missing_runtime_and_stale_target() {
    let baseline = ProofPreflightFacts {
        runtime_exists: true,
        runtime_is_file: true,
        target_is_fresh: true,
        observe_mode: true,
        active_work: Vec::new(),
        idle_tui_count: 0,
        expected_idle_tui_count: 0,
        deployed_binaries_match: true,
        protected_snapshot_valid: true,
    };
    evaluate_preflight(&baseline).unwrap();
    for (mut facts, needle) in [
        (
            ProofPreflightFacts {
                active_work: vec!["cargo test".into()],
                ..baseline.clone()
            },
            "active",
        ),
        (
            ProofPreflightFacts {
                runtime_exists: false,
                ..baseline.clone()
            },
            "runtime",
        ),
        (
            ProofPreflightFacts {
                target_is_fresh: false,
                ..baseline.clone()
            },
            "not fresh",
        ),
        (
            ProofPreflightFacts {
                observe_mode: false,
                ..baseline.clone()
            },
            "gate_mode",
        ),
        (
            ProofPreflightFacts {
                deployed_binaries_match: false,
                ..baseline.clone()
            },
            "binaries differ",
        ),
        (
            ProofPreflightFacts {
                protected_snapshot_valid: false,
                ..baseline.clone()
            },
            "protected",
        ),
    ] {
        let error = evaluate_preflight(&facts).unwrap_err();
        assert!(error.contains(needle), "{error}");
        facts.active_work.clear();
    }
}

#[test]
fn common_existing_ancestor_binds_runtime_project_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let prd = temp.path().join("prds/PRD.md");
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(&prd, b"prd").unwrap();
    assert_eq!(
        common_existing_ancestor(&prd, &tasks).unwrap(),
        temp.path().canonicalize().unwrap()
    );
}

#[test]
fn clearance_manifest_validation_detects_tampered_evidence() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("evidence.txt"), b"original").unwrap();
    let manifest = build_evidence_manifest("synthetic", identity("same"), temp.path()).unwrap();
    write_json(&temp.path().join("manifest.json"), &manifest).unwrap();
    validate_evidence_manifest(temp.path(), &manifest.digest).unwrap();
    std::fs::write(temp.path().join("evidence.txt"), b"tampered").unwrap();
    assert!(validate_evidence_manifest(temp.path(), &manifest.digest).is_err());
}

#[test]
fn rust_function_extractor_ignores_signature_text_in_other_functions() {
    let source = r#"fn scanner() { let text = "fn target()"; } fn target() { call(); }"#;
    assert_eq!(
        rust_function_body(source, "fn target()").unwrap().trim(),
        "call();"
    );
}

#[test]
fn decomposition_log_validation_rejects_unstructured_candidate_content() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".decompose.log");
    std::fs::write(
        &path,
        format!(
            "event=run_started run_id=wf-x binary_revision=rev script_digest={} catalog_digest={}\n",
            "a".repeat(64),
            "b".repeat(64)
        ),
    )
    .unwrap();
    validate_decomposition_log(&path).unwrap();
    std::fs::write(&path, "candidate body leaked here\n").unwrap();
    assert!(validate_decomposition_log(&path).is_err());
    std::fs::write(
        &path,
        "event_id=1 phase=bodies subject=TASK-SECRET-010 attempt=1 disposition=accepted findings=0 status=accepted reused=false\n",
    )
    .unwrap();
    assert!(validate_decomposition_log(&path).is_err());
    std::fs::write(
        &path,
        format!(
            "event_id=1 phase=bodies subject_digest={} attempt=1 disposition=accepted findings=0 status=accepted reused=false\n",
            "a".repeat(64)
        ),
    )
    .unwrap();
    validate_decomposition_log(&path).unwrap();
}

#[test]
fn local_gate_mode_overrides_project_observe_mode() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join(".archon")).unwrap();
    std::fs::write(
        temp.path().join(".archon/config.toml"),
        "[workflow]\ngate_mode = \"observe\"\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join(".archon/config.local.toml"),
        "[workflow]\ngate_mode = \"enforce\"\n",
    )
    .unwrap();
    let error = require_observe_config(temp.path()).unwrap_err();
    assert!(error.contains("enforce"), "{error}");
}

#[test]
fn manifest_validation_rejects_unlisted_extra_file() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("evidence.txt"), b"original").unwrap();
    let manifest = build_evidence_manifest("synthetic", identity("same"), temp.path()).unwrap();
    write_json(&temp.path().join("manifest.json"), &manifest).unwrap();
    std::fs::write(temp.path().join("unlisted.txt"), b"extra").unwrap();
    assert!(validate_evidence_manifest(temp.path(), &manifest.digest).is_err());
}

#[test]
fn clearance_rejects_wrong_committed_fixture_digest() {
    let temp = tempfile::tempdir().unwrap();
    let clearance = SyntheticClearance {
        schema_version: 1,
        identity: identity("same"),
        fixture_digest: "wrong".into(),
        evidence_manifest_digest: "manifest".into(),
    };
    let path = temp.path().join("clearance.json");
    write_json(&path, &clearance).unwrap();
    let error = read_clearance(&path).unwrap_err();
    assert!(error.contains("fixture digest"), "{error}");
}

#[test]
fn typed_event_wait_does_not_match_words_only_in_unrelated_detail() {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::project(temp.path());
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "events".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    archon_workflow::WorkflowEventLog::new(store.clone())
        .emit(
            &run.id,
            1,
            archon_workflow::WorkflowEventKind::StageFailed,
            serde_json::json!({"message": "author_attempt_started skeleton-author-1"}),
        )
        .unwrap();
    assert!(
        wait_for_event_line(
            temp.path(),
            &run.id,
            &["author_attempt_started", "skeleton-author-1"],
            std::time::Duration::from_millis(20),
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn protected_snapshot_detects_mode_empty_directory_and_symlink_changes() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("one.txt");
    std::fs::write(&file, b"same bytes").unwrap();
    let baseline = snapshot_tree(temp.path()).unwrap();

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mode_changed = snapshot_tree(temp.path()).unwrap();
    assert!(require_unchanged(&baseline, &mode_changed).is_err());

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let restored = snapshot_tree(temp.path()).unwrap();
    std::fs::create_dir(temp.path().join("empty")).unwrap();
    let directory_added = snapshot_tree(temp.path()).unwrap();
    assert!(require_unchanged(&restored, &directory_added).is_err());

    std::os::unix::fs::symlink(&file, temp.path().join("alias")).unwrap();
    assert!(snapshot_tree(temp.path()).unwrap_err().contains("symlink"));
}

#[test]
fn independent_review_requires_bound_regular_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let artifact = temp.path().join("review.txt");
    std::fs::write(&artifact, b"reviewed evidence").unwrap();
    let review = IndependentReviewReceipt {
        schema_version: 1,
        reviewer: "independent-reviewer".into(),
        evidence_manifest_digest: "manifest".into(),
        runtime_identity_digest: "runtime".into(),
        artifact_path: "review.txt".into(),
        artifact_sha256: bytes_sha256(b"reviewed evidence"),
        approved: true,
        unresolved_critical: 0,
        unresolved_important: 0,
    };
    validate_independent_review_artifact(temp.path(), &review).unwrap();
    std::fs::write(&artifact, b"tampered").unwrap();
    assert!(validate_independent_review_artifact(temp.path(), &review).is_err());
    std::fs::remove_file(&artifact).unwrap();
    assert!(validate_independent_review_artifact(temp.path(), &review).is_err());
}

#[cfg(unix)]
#[test]
fn committed_receipt_copy_refuses_symlinked_source() {
    let temp = tempfile::tempdir().unwrap();
    let live = temp.path().join("live.txt");
    let alias = temp.path().join("alias.txt");
    let target = temp.path().join("evidence.txt");
    std::fs::write(&live, b"committed").unwrap();
    std::os::unix::fs::symlink(&live, &alias).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(b"committed");
    let error = copy_committed_receipt_entry(&alias, 9, &digest, &target).unwrap_err();
    assert!(
        error.contains("regular file") || error.contains("opening"),
        "{error}"
    );
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn evidence_collection_refuses_symlinked_run_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let store = archon_workflow::WorkflowStore::project(&project);
    let run_id = "wf-symlink-evidence";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    let outside = temp.path().join("outside.json");
    std::fs::write(&outside, b"{}\n").unwrap();
    std::os::unix::fs::symlink(&outside, store.state_path(run_id)).unwrap();

    let error = copy_evidence(&project, run_id, &temp.path().join("evidence")).unwrap_err();
    assert!(error.contains("non-symlink"), "{error}");
}

#[test]
fn receipt_collection_calls_nofollow_copy_boundary() {
    let source = include_str!("workflow_decomposition_proof_evidence.rs");
    let signature = concat!("pub fn collect_host_", "command_receipts(");
    let body = rust_function_body(source, signature).unwrap();
    assert!(body.contains("copy_committed_receipt_entry"), "{body}");
}

#[test]
fn child_watchdog_terminates_and_reaps_timed_out_process() {
    let mut child = std::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .unwrap();
    let error = wait_for_child(&mut child, std::time::Duration::from_millis(20)).unwrap_err();
    assert!(error.contains("was reaped"), "{error}");
    assert!(child.try_wait().unwrap().is_some());
}
