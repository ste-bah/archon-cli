use super::*;

// Captured from the pre-screen prompt at 5b75b8a11 for contract("check").
const PRE_SCHEMA_PROMPT: &str = r#"Adversarially judge every acceptance check below. The toolchain is fixed: the shell, the operating system, environment variables, PATH, and every executable that the repository does not itself build are out of bounds, and a counterexample that stubs, wraps, replaces or shadows any executable, or edits PATH, is invalid and must not refute a check. Everything the implementation produces may vary: the repository's own source and the program it builds from that source, and every file, directory and data artifact under the project root, including any data root the check names. The implementation is fallible, not adversarial: it may be missing, partial, wrong, stale, empty, malformed or hand-placed, its tests may be absent or cover less than the criterion, and its output may be an error message; it does not write source, tests or data whose purpose is to satisfy this check while the criterion is false. A named-file regression guard is valid only when its pass or fail is decided by the existing test or implementation it names, and it fails whenever that test fails. A guard names its deciding file by its literal path relative to the check's working directory. Naming a file or asserting that it exists does not make it the deciding assertion. Judge supplied named_files content as written. These regression-guard rules apply to a check whose deciding test or implementation exists. A named path marked absent means the host found no file at that path; it may be a pattern or text argument, not a file, and absence alone is not evidence that behaviour is missing. Only when the path is established to name a deciding file does its absence mean behaviour not implemented yet, judged by the general rules. A deciding input marked not_supplied has unknown assertions, so the check cannot be accepted as a regression guard on assumed content. A deciding input marked not_judgeable has binary content the host cannot show as text; its assertions are unknown and cannot be assumed to accept a regression guard. Refute a check if an in-bounds state lets its deciding test fail or not run while the check still passes, including when the command discards or masks that test's status, runs it so its failure cannot fail the check, or decides by matching text in a source or test file. A duplicated or relocated implementation may be an in-bounds counterexample when the delivered path uses it while the named test checks an old copy; a speculative counterexample requiring deliberate gaming remains invalid. The host may use a move-aside proof only for named inputs it actually moves; requiring the check to fail when such an input is moved aside is proof of the guard, never a counterexample. Inputs the host does not move, including scripts run through an interpreter and build manifests, do not qualify for that proof. A counterexample that needs such deliberate gaming, a program that answers true for the check's sake or a test written to pass trivially, is invalid and must not refute a check: under it no check could ever pass. For each id, try to construct such an in-bounds state where the check passes while the criterion is false. Return JSON only as {"decisions":[{"id":"...","verdict":"accepted|refuted","counterexample":"...","reason":"..."}]} with exactly one decision for every input id and no extra ids. counterexample and reason must each be a non-empty sentence, even when the verdict is accepted: say what the closest passing-but-false state would be, or that none is constructible. Every string must be a single line with newlines escaped as \n; emit the JSON document alone. Checks: [{"check":{"command":"check","cwd":"project_root","kind":"command"},"criterion":"output is valid","id":"AC-X-001"}]"#;

#[tokio::test]
#[allow(non_snake_case)]
async fn R1_b_pre_schema_saved_judgment_is_reasked_without_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let client = CountingJudge {
        calls: calls.clone(),
    };
    let asked = contract("check");
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let store = JudgeStore::at(dir.path().to_path_buf());
    // Before this change the prompt lacked these required schema keys.
    // Saved contracts themselves never contained kind or cited_paths.
    let old_prompt = PRE_SCHEMA_PROMPT;
    let old_key = content_digest(
        serde_json::json!([
            "acceptance-judge-batch-v1",
            old_prompt,
            expected,
            client.resolve_model_alias("sonnet"),
            client.provider_id(),
            current_logic_identity(&client)
        ])
        .to_string()
        .as_bytes(),
    );
    let mut stale = asked.clone();
    stale.acceptance[0].judgment.verdict =
        archon_workflow::task_set_contract::JudgeDecision::Accepted;
    stale.acceptance[0].judgment.reason = "legacy verdict must not be reused".into();
    stale.acceptance[0].judgment.counterexample = "none is constructible".into();
    stale.acceptance[0].judgment.host_call_id = "acceptance-judge-batch:AC-X-001".into();
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment_with_content_digest(
        &mut stale.acceptance[0],
        &client.resolve_model_alias("sonnet"),
        client.provider_id().as_deref(),
        &Default::default(),
    );
    assert!(store.save(&old_key, &stale));
    assert!(
        store
            .load(&old_key, &asked, current_logic_identity(&client).as_ref())
            .is_some()
    );
    let saved: Saved =
        serde_json::from_slice(&std::fs::read(dir.path().join(format!("{old_key}.json"))).unwrap())
            .unwrap();
    assert_eq!(
        saved.judged.acceptance[0].judgment.reason,
        "legacy verdict must not be reused"
    );
    assert_ne!(
        old_key,
        JudgeStore::key(&client, &asked, &expected).unwrap()
    );
    let judged = store.judge(&client, asked, &expected).await.unwrap();
    assert_eq!(calls.load(SeqCst), 1);
    assert_eq!(judged.acceptance[0].judgment.reason, "jq rejects it");
}
