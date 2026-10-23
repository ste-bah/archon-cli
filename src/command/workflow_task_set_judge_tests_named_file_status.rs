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
    assert!(rendered[0]["content"].as_str().unwrap().is_empty());
    assert!(
        rendered[0]["detail"]
            .as_str()
            .unwrap()
            .contains("truncated")
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
