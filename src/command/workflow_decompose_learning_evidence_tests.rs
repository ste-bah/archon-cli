#[test]
fn hashed_call_ids_use_record_role_and_capability_for_author_and_probe_lessons() {
    let author_call_id = "call-9e5a";
    let probe_call_id = "call-a18c";
    let author_before = author_call_id.contains("author") || author_call_id.contains("body");
    let probe_before = probe_call_id.contains("probe") || probe_call_id.contains("baseline");
    let author = super::super::workflow_decompose_learning::lesson_records_with_metadata(
        "run",
        "",
        author_call_id,
        1,
        "author",
        None,
        &serde_json::json!({"response":"accepted correction"}),
        &[],
    );
    let probe = super::super::workflow_decompose_learning::lesson_records_with_metadata(
        "run",
        "",
        probe_call_id,
        1,
        "host",
        Some("acceptance-probe"),
        &serde_json::json!({"stdout":"probe output"}),
        &[],
    );
    println!(
        "EVIDENCE #284 hashed-author BEFORE: call_id_match={author_before} AFTER: {:?}",
        author.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    println!(
        "EVIDENCE #284 hashed-probe BEFORE: call_id_match={probe_before} AFTER: {:?}",
        probe.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    assert!(
        author
            .iter()
            .any(|r| r.name.contains("accepted correction"))
    );
    assert!(probe.iter().any(|r| r.name.contains("probe output")));
}

#[test]
fn reasoning_bank_hook_is_selected_for_the_fold_write_sink() {
    let mut config = archon_core::config::LearningConfig::default();
    config.reasoning_bank.enabled = true;
    config.desc.enabled = false;
    let before: Vec<String> = crate::command::learning_workflow_hooks::derive_learning_hooks(
        "review acceptance",
        None,
        &config,
    )
    .into_iter()
    .filter(|hook| hook == "reasoning_bank")
    .collect();
    let hooks = super::super::workflow_decompose_learning::hooks("review acceptance", &config);
    println!("EVIDENCE #284 bank-sink BEFORE: {before:?} AFTER: {hooks:?}");
    assert!(hooks.iter().any(|hook| hook == "reasoning_bank"));
    let warning =
        super::super::workflow_decompose_learning::unavailable_hook_warning(&before, &hooks);
    assert_eq!(warning, None);
}

#[test]
fn no_remaining_learning_hook_has_an_operator_visible_reason() {
    let warning = super::super::workflow_decompose_learning::unavailable_hook_warning(&[], &[]);
    assert_eq!(
        warning,
        Some((
            "none".into(),
            "no configured learning hook matched this run".into()
        ))
    );
}
