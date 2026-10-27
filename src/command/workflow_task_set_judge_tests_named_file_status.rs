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
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let missing = command_check("operation missing/check.test", TrustedCwd::ProjectRoot);
    let missing_prompt = batched_judge_prompt_with_roots(&missing, &roots).unwrap();
    assert_eq!(
        prompt_json(&missing_prompt)[0]["named_files"],
        serde_json::json!([{ "path": "missing/check.test", "status": "absent" }])
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
