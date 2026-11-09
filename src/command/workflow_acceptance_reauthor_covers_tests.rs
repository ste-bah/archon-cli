use super::*;

#[tokio::test]
async fn covered_requirement_text_refutation_drops_id_and_owes_supplementary_without_idle_loop() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let text =
        "- REQ-X-001: The first output is ready.\n- REQ-X-002: The second output is ready.\n";
    std::fs::write(&set.prd, text).unwrap();
    let mut before = set.contract();
    before.acceptance[0].covers = vec!["REQ-X-001".into(), "REQ-X-002".into()];
    let refuter = ScriptedAuthorJudge::new(|_, _| panic!("judge only"), |_, _| false)
        .with_judge_reply(r#"{"decisions":[{"id":"AC-F-001","verdict":"refuted","kind":"state","cited_paths":[],"counterexample":"The first output is ready while the second output is absent and the check passes.","reason":"REQ-X-002 can be violated while this check passes."}]}"#);
    before = super::super::judge::judge_entries_with_roots(
        &refuter,
        before,
        "sonnet",
        &super::super::judge::JudgeRoots {
            covered_texts: scope(&set).covered_texts().unwrap(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        before.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
    assert!(before.acceptance[0].judgment.reason.contains("REQ-X-002"));
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            assert_eq!(
                entry["covers"],
                serde_json::json!(["REQ-X-001", "REQ-X-002"])
            );
            assert_eq!(
                entry["covers_text"]["REQ-X-002"],
                "The second output is ready."
            );
            let mut reply = entry.clone();
            reply["covers"] = serde_json::json!(["REQ-X-001"]);
            reply.to_string()
        },
        |_, _| true,
    );
    let after = reauthor(
        &client,
        &before,
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("coverage-only repair makes progress");
    assert_eq!(client.authored(), 1);
    assert_eq!(after.acceptance[0].covers, vec!["REQ-X-001"]);
    assert_eq!(after.acceptance[0].check, before.acceptance[0].check);
    assert_eq!(
        after.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    let prompts = client.judge_prompts.lock().unwrap();
    let checks: Vec<serde_json::Value> =
        serde_json::from_str(prompts[0].rsplit_once("Checks: ").unwrap().1).unwrap();
    assert_eq!(
        checks[0]["covers_text"]["REQ-X-001"],
        "The first output is ready."
    );
    assert!(checks[0]["covers_text"].get("REQ-X-002").is_none());
    let prepared = crate::command::workflow_task_set::findings::finish_acceptance(
        set.project.path(),
        &set.tasks,
        &set.prd,
        text,
        archon_workflow::task_set_contract::FreezeGateMode::Enforce,
        &after,
        Vec::new(),
        None.into(),
        String::new(),
    )
    .unwrap();
    let findings: Vec<_> = prepared
        .findings
        .into_iter()
        .filter(|finding| {
            finding
                .deterministic_defect
                .as_ref()
                .is_some_and(|defect| defect.code == "uncovered_requirement")
        })
        .collect();
    let mut child = std::process::Command::new("node")
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/command/workflow_acceptance_reauthor_covers_route_driver.cjs"),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(serde_json::to_vec(&serde_json::json!({
        "contract": after, "findings": crate::command::workflow_gate::GateEvaluation::new("coverage repair", findings.clone()).into_envelope().unwrap().policy_findings, "texts": scope(&set).covered_texts().unwrap()
    })).unwrap().as_slice()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].subject, "SUP-REQ-X-002");
    assert_eq!(
        findings[0].deterministic_defect.as_ref().unwrap().code,
        "uncovered_requirement"
    );
    assert!(
        findings[0]
            .text
            .contains("author supplementary check SUP-REQ-X-002")
    );
}

#[test]
fn covered_requirement_text_repair_cannot_add_ids_or_hide_invalid_covers() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let frozen = &set.contract().acceptance[0];
    for covers in [serde_json::json!(["REQ-X-999"]), serde_json::json!(null)] {
        let mut reply: serde_json::Value = serde_json::from_str(&command_entry(
            &serde_json::to_value(frozen).unwrap(),
            "exit 1",
        ))
        .unwrap();
        reply["covers"] = covers;
        assert!(super::super::reply_parse::candidate_entry(&reply.to_string(), frozen).is_err());
    }
}

#[test]
fn covered_requirement_text_read_error_is_propagated() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let scope = scope(&set);
    std::fs::remove_file(&set.prd).unwrap();
    let frozen = &set.contract().acceptance[0];
    let prefetched = super::super::author::prefetch_files(&scope, frozen, &[]);
    assert!(
        super::super::author::author_prompt_with_prefetch(&scope, frozen, &[], 1, &prefetched, &[])
            .is_err()
    );
}

#[test]
fn covered_requirement_text_drop_guards_use_sets_refutations_and_routes() {
    use super::super::coverage_repair::{refuted_ids, validate};
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let mut frozen = set.contract().acceptance.remove(0);
    frozen.covers = vec!["REQ-X-001".into(), "REQ-X-002".into(), "G-X-003".into()];
    frozen.judgment.reason =
        "REQ-X-002 and G-X-003 are not decided; REQ-X-0010 is unrelated".into();
    let refuted = refuted_ids(&frozen);
    assert_eq!(refuted, vec!["REQ-X-002", "G-X-003"]);
    let routed = ids(&["REQ-X-001", "REQ-X-002"]);
    for (covers, expected) in [
        (vec!["REQ-X-002", "G-X-003"], "was not named"),
        (vec!["REQ-X-001", "REQ-X-002"], "no repair route"),
        (vec!["REQ-X-999"], "only retain or drop"),
        (
            vec!["G-X-003", "REQ-X-002", "REQ-X-001"],
            "repeats the check",
        ),
        (
            vec!["REQ-X-001", "REQ-X-002", "G-X-003", "REQ-X-001"],
            "repeats the check",
        ),
    ] {
        let mut candidate = frozen.clone();
        candidate.covers = covers.into_iter().map(str::to_string).collect();
        let reason = validate(candidate, &frozen, &refuted, &routed).unwrap_err();
        assert!(reason.contains(expected), "{reason}");
    }
    let mut candidate = frozen.clone();
    candidate.covers = vec!["G-X-003".into(), "REQ-X-001".into()];
    assert!(validate(candidate, &frozen, &refuted, &routed).is_ok());
    let mut empty = frozen.clone();
    empty.covers.clear();
    assert!(
        validate(
            empty,
            &frozen,
            &frozen.covers,
            &frozen.covers.iter().cloned().collect()
        )
        .is_ok()
    );
    frozen.id = "SUP-REQ-X-002".into();
    let mut candidate = frozen.clone();
    candidate.covers = vec!["REQ-X-001".into(), "G-X-003".into()];
    assert!(
        validate(candidate, &frozen, &refuted, &routed)
            .unwrap_err()
            .contains("own requirement")
    );
}

#[tokio::test]
async fn covered_requirement_text_unrelated_drop_is_refused_even_with_changed_check() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    std::fs::write(&set.prd, "- REQ-X-001: First.\n- REQ-X-002: Second.\n").unwrap();
    let mut before = set.contract();
    before.acceptance[0].covers = vec!["REQ-X-001".into(), "REQ-X-002".into()];
    before.acceptance[0].judgment.reason = "REQ-X-002 is not checked".into();
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            let mut reply: serde_json::Value =
                serde_json::from_str(&command_entry(entry, "exit 1")).unwrap();
            reply["covers"] = serde_json::json!(["REQ-X-002"]);
            reply.to_string()
        },
        |_, _| panic!("unauthorized drop must not reach judge"),
    );
    let error = reauthor(
        &client,
        &before,
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("was not named"), "{error}");
    assert_eq!(client.authored(), REAUTHOR_ATTEMPTS);
}

#[tokio::test]
async fn single_refuted_cover_can_be_dropped_to_empty() {
    let set = frozen_set(&[("AC-F-004", "false", false)]);
    std::fs::write(&set.prd, "- REQ-X-004: The output is ready.\n").unwrap();
    let mut before = set.contract();
    before.acceptance[0].covers = vec!["REQ-X-004".into()];
    before.acceptance[0].judgment.reason = "REQ-X-004 can be violated while this passes.".into();
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            assert_eq!(entry["droppable_covers"], serde_json::json!(["REQ-X-004"]));
            let mut reply = entry.clone();
            reply["covers"] = serde_json::json!([]);
            reply.to_string()
        },
        |_, _| true,
    );
    let after = reauthor(
        &client,
        &before,
        &ids(&["AC-F-004"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .unwrap();
    assert!(after.acceptance[0].covers.is_empty());
    assert_eq!(client.authored(), 1);
    let uncovered = archon_workflow::v2::acceptance_stage::coverage::uncovered_requirements(
        &ids(&["REQ-X-004"]),
        &after,
    );
    assert_eq!(uncovered, ["REQ-X-004"]);
}

// The extra class is unknown before the coverable-ids merge and coverable
// but unrouted afterwards. Using the wider set at the real call site then
// lets the drop reach the judge, which must fail this test.
#[tokio::test]
async fn reauthor_refuses_dropping_a_cover_without_an_active_route() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    std::fs::write(&set.prd, "- REQ-X-001: First.\n- G-X-003: Second.\n").unwrap();
    let mut before = set.contract();
    before.acceptance[0].covers = vec!["REQ-X-001".into(), "G-X-003".into()];
    before.acceptance[0].judgment.reason = "G-X-003 can be violated while this passes.".into();
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            assert_eq!(entry["droppable_covers"], serde_json::json!([]));
            let mut reply = entry.clone();
            reply["covers"] = serde_json::json!(["REQ-X-001"]);
            reply.to_string()
        },
        |_, _| panic!("unrouted drop reached the judge"),
    );
    let error = reauthor(
        &client,
        &before,
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("no repair route"), "{error}");
    assert_eq!(client.authored(), REAUTHOR_ATTEMPTS);
}

#[tokio::test]
async fn saved_coverage_drop_without_reloaded_authority_is_a_cache_miss_not_author_feedback() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    std::fs::write(&set.prd, "- REQ-X-001: First.\n- REQ-X-002: Second.\n").unwrap();
    let scope = scope(&set);
    let mut before = set.contract();
    before.acceptance[0].covers = vec!["REQ-X-001".into(), "REQ-X-002".into()];
    before.acceptance[0].judgment.reason =
        "The original check does not decide the criterion.".into();
    let mut saved = before.acceptance[0].clone();
    saved.covers.pop();
    saved.judgment.verdict = JudgeDecision::Accepted;
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "exit 1"), |_, _| true);
    let model = crate::command::workflow_task_set::republish::test_fixture::FIXTURE_JUDGE_MODEL;
    cache::store(
        &set.tasks,
        &saved,
        &before,
        &scope,
        model,
        Some(super::super::test_client::SCRIPTED_PROVIDER),
        &set.probe,
    )
    .unwrap();
    assert!(
        cache::load(
            &set.tasks,
            &before.acceptance[0],
            &scope,
            model,
            Some(super::super::test_client::SCRIPTED_PROVIDER),
            &set.probe
        )
        .unwrap()
        .is_some()
    );
    let after = reauthor_with_cache(
        &client,
        &before,
        &ids(&["AC-F-001"]),
        &scope,
        model,
        &set.gate(),
        Some(&set.tasks),
    )
    .await
    .unwrap();
    assert_eq!(client.authored(), 1);
    assert_eq!(after.acceptance[0].covers, before.acceptance[0].covers);
    let prompts = client.prompts.lock().unwrap();
    assert!(!prompts[0].contains("was not named in the judge's refutation reason"));
    assert!(!prompts[0].contains("host cache miss"));
}
