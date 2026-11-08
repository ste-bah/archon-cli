use super::*;

#[test]
fn named_inputs_report_absence_exclusion_and_shared_budget_status() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("credentials")).unwrap();
    std::fs::write(root.path().join("credentials/input"), "excluded").unwrap();
    std::fs::write(
        root.path().join("first"),
        "a".repeat(archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES),
    )
    .unwrap();
    std::fs::write(root.path().join("second"), "second content").unwrap();

    let roots = JudgeRoots {
        covered_texts: Default::default(),
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let missing = command_check("operation missing/check.test", TrustedCwd::ProjectRoot);
    let missing_prompt = batched_judge_prompt_with_roots(&missing, &roots).unwrap();
    assert_eq!(
        prompt_json(&missing_prompt)[0]["named_files"],
        serde_json::json!([{ "path": "missing/check.test", "status": "absent", "detail": "The host found no file at this path. It may be a pattern or text argument, not a file; this alone is not evidence that behaviour is missing." }])
    );

    let excluded = command_check("operation credentials/input", TrustedCwd::ProjectRoot);
    let excluded_prompt = batched_judge_prompt_with_roots(&excluded, &roots).unwrap();
    assert_eq!(
        prompt_json(&excluded_prompt)[0]["named_files"],
        serde_json::json!([{ "path": "credentials/input", "status": "not_supplied" }])
    );

    let first = command_check("operation first", TrustedCwd::ProjectRoot)
        .acceptance
        .remove(0);
    let mut second = command_check("operation second", TrustedCwd::ProjectRoot)
        .acceptance
        .remove(0);
    second.id = "AC-X-002".into();
    let subject = AcceptanceContract {
        acceptance: vec![first.clone(), second],
        ..contract()
    };
    let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
    let checks = prompt_json(&prompt);
    let rendered = checks[1]["named_files"].as_array().unwrap();
    assert_eq!(rendered[0]["path"], "second");
    assert_eq!(rendered[0]["status"], "not_supplied");
    assert!(rendered[0].get("content").is_none(), "{rendered:?}");
    assert!(
        rendered[0]["detail"]
            .as_str()
            .unwrap()
            .contains("byte limit")
    );

    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    let exact_first = named_files_with_budget(&first, &roots, &mut remaining);
    let exact_second = named_files_with_budget(&subject.acceptance[1], &roots, &mut remaining);
    assert_eq!(checks[0]["named_files"], serde_json::json!(exact_first));
    assert_eq!(checks[1]["named_files"], serde_json::json!(exact_second));
    let mut digest_remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    assert_eq!(
        named_files_digest_with_budget(&first, &roots, &mut digest_remaining),
        named_files_digest_for_list(&exact_first)
    );
    assert_eq!(
        named_files_digest_with_budget(&subject.acceptance[1], &roots, &mut digest_remaining),
        named_files_digest_for_list(&exact_second)
    );
}

#[test]
fn a_budget_exhausted_entry_is_stamped_with_its_full_content_digest() {
    let root = tempfile::tempdir().unwrap();
    let large = "a".repeat(archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES);
    std::fs::write(root.path().join("first"), large).unwrap();
    std::fs::write(root.path().join("second"), "second content").unwrap();
    let roots = roots_at(root.path());
    let mut first = command_check("operation first", TrustedCwd::ProjectRoot)
        .acceptance
        .remove(0);
    let mut second = command_check("operation second", TrustedCwd::ProjectRoot)
        .acceptance
        .remove(0);
    first.id = "AC-X-001".into();
    second.id = "AC-X-002".into();
    let subject = AcceptanceContract {
        acceptance: vec![first, second.clone()],
        ..contract()
    };
    let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
    assert_eq!(
        prompt_json(&prompt)[1]["named_files"][0]["status"],
        "not_supplied"
    );

    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    let _ = named_files_digest_with_budget(&subject.acceptance[0], &roots, &mut remaining);
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment_with_content_digest(
        &mut second,
        "model-x",
        Some("provider-x"),
        &roots,
    );

    assert_eq!(
        second.judgment.sampling.as_ref().unwrap()["named_file_digest"],
        super::named_files_content_digest(&subject.acceptance[1], &roots),
        "the stamp must identify full input content independently of prompt budget"
    );
}

fn roots_at(root: &std::path::Path) -> JudgeRoots {
    JudgeRoots {
        covered_texts: Default::default(),
        project: Some(root.to_path_buf()),
        repository: None,
        task_root: Some(root.to_path_buf()),
        repository_commit: None,
    }
}

/// R8: a named input the host cannot read is reported, never dropped.
#[cfg(unix)]
#[test]
fn an_unreadable_named_input_is_not_supplied_never_dropped() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input");
    std::fs::write(&path, "content").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = std::fs::read(&path).is_err();
    let subject = command_check("operation input", TrustedCwd::ProjectRoot);
    let files = named_files(&subject.acceptance[0], &roots_at(root.path()));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    if !unreadable {
        return; // a user that reads every file cannot make this failure
    }
    assert_eq!(
        files,
        vec![serde_json::json!({ "path": "input", "status": "not_supplied" })]
    );
}

/// R8: not_supplied content has unknown assertions, so none is sent: a file
/// the judged-source byte limit would cut carries no partial content, and
/// it spends none of the batch's budget.
#[test]
fn a_named_input_past_the_byte_limit_is_not_supplied_without_partial_content() {
    let root = tempfile::tempdir().unwrap();
    let limit = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    std::fs::write(root.path().join("large"), vec![b'x'; limit + 1]).unwrap();
    std::fs::write(root.path().join("small"), "small content").unwrap();
    let roots = roots_at(root.path());
    let large = command_check("operation large", TrustedCwd::ProjectRoot);
    let small = command_check("operation small", TrustedCwd::ProjectRoot);
    let mut remaining = limit;
    let files = named_files_with_budget(&large.acceptance[0], &roots, &mut remaining);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0]["status"], "not_supplied");
    assert!(files[0].get("content").is_none(), "{files:?}");
    assert!(
        (files[0]["detail"].as_str().unwrap()).contains("byte limit"),
        "{files:?}"
    );
    assert_eq!(remaining, limit, "nothing was sent");
    let files = named_files_with_budget(&small.acceptance[0], &roots, &mut remaining);
    assert_eq!(files[0]["status"], "supplied");
    assert_eq!(files[0]["content"], "small content");
}

#[test]
fn a_slash_pattern_is_not_evidence_of_missing_behaviour_in_either_prompt_view() {
    assert_missing_path_is_uncertain("grep -q 'ABC-1/2 budget' src/x.rs", "ABC-1/2");
}

#[test]
fn a_real_missing_file_remains_absent_without_guessing_argument_intent() {
    assert_missing_path_is_uncertain("test -f src/new_module.rs", "src/new_module.rs");
}

fn assert_missing_path_is_uncertain(command: &str, missing: &str) {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "judge@example.test"]);
    git(root.path(), &["config", "user.name", "Judge Test"]);
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/x.rs"), "existing assertion").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-qm", "initial"]);
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        let mut roots = roots_at(root.path());
        roots.repository = Some(root.path().to_path_buf());
        let subject = command_check(command, cwd);
        let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
        let checks = prompt_json(&prompt);
        let judge_files = checks[0]["named_files"].as_array().unwrap();
        let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
        let author_files = named_files_for_reauthor_cached(
            &subject.acceptance[0],
            &each::with_commit(&roots),
            &[],
            &mut remaining,
            &mut ReauthorPrefetchCache::default(),
        );
        assert_eq!(judge_files, &author_files);
        let file = judge_files
            .iter()
            .find(|file| file["path"] == missing)
            .unwrap();
        assert_eq!(file["status"], "absent");
        let detail = file["detail"]
            .as_str()
            .expect("absence must explain uncertainty");
        assert!(
            detail.contains("may be a pattern or text argument"),
            "{file}"
        );
        assert!(
            detail.contains("not evidence that behaviour is missing"),
            "{file}"
        );
        assert!(prompt.contains("Only when the path is established to name a deciding file"));
        if command.contains("src/x.rs") {
            let existing = judge_files
                .iter()
                .find(|file| file["path"] == "src/x.rs")
                .unwrap();
            assert_eq!(existing["status"], "supplied");
            assert_eq!(existing["content"], "existing assertion");
        }
    }
}
