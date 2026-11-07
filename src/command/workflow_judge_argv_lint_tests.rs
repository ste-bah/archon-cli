use super::*;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct CountingJudge(AtomicUsize);

#[async_trait::async_trait]
impl WorkflowLlmClient for CountingJudge {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::error::WorkflowResult<WorkflowAgentOutcome> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let prompt = messages[0]["content"].as_str().unwrap();
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(prompt.split("Checks: ").last().unwrap()).unwrap();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": checks.iter().map(|check| serde_json::json!({
                "id": check["id"], "kind": "state", "cited_paths": [], "verdict": "accepted",
                "counterexample": "no passing false state", "reason": "the check decides its criterion"
            })).collect::<Vec<_>>()}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        _: f64,
    ) -> archon_workflow::error::WorkflowResult<WorkflowAgentOutcome> {
        self.send_message(messages, system, tools, model).await
    }
}

fn candidate(command: &str) -> AcceptanceContract {
    let mut candidate = contract();
    candidate.supplementary.clear();
    candidate.acceptance[0].check = AcceptanceCheck::Command {
        command: command.into(),
        cwd: TrustedCwd::RepoRoot,
    };
    candidate
}

#[tokio::test]
async fn command_text_lint_precedes_judgment() {
    for (command, linted) in [
        ("cargo test first second", true),
        ("cargo test first second || true", false),
        ("! cargo test first second", false),
        ("if cargo test first second; then echo ok; fi", false),
        ("cargo test first second 2>&1 | tail -5", false),
        ("cargo test first second; cargo test first", false),
        ("set -eu\ncargo test first second", true),
        ("set -eu\ncargo test first second\necho done", true),
        ("FOO=1 cargo test first second", true),
        ("env FOO=1 cargo test first second", true),
        ("time cargo test first second", true),
        ("nohup cargo test first second", true),
        ("echo ready\ncargo test first second", true),
        ("# a comment\n\nset -eu\ncargo test first second", true),
        ("cargo test first \\\nsecond", true),
        ("cargo test -p pkg --test t first", false),
        ("cargo test --features x first", false),
        ("./cargo test first second", false),
        ("tools/cargo test first second", false),
        ("cargo test first", false),
        ("cargo test -- --exact first second", false),
        ("cargo test first -- second third", false),
        ("runner test first second", false),
        ("cargo test --config settings first", false),
        ("cargo test $names $other", false),
        ("cargo test --unknown value first", false),
        ("cargo test a b\n# note \\\necho done", false),
        ("# note \\\ncargo test a b", false),
        ("set\u{a0}-e\ncargo test a b\necho done", false),
        ("\u{a0}set -e\ncargo test a b\necho done", false),
        ("set -e\u{a0}\ncargo test a b\necho done", false),
        ("cargo test a b\r\n", false),
        ("declare -n P=PATH\ncargo test a b", false),
        ("RUSTUP_TOOLCHAIN=x cargo test a b", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let client = CountingJudge::default();
        let roots = judge::JudgeRoots::default();
        let store =
            super::super::super::judge_store::JudgeStore::at(directory.path().join("saved"));
        let judged = judge(
            directory.path(),
            directory.path(),
            &client,
            candidate(command),
            &BTreeSet::from(["AC-1".into()]),
            Some(&store),
            &roots,
        )
        .await
        .unwrap();
        assert_eq!(
            client.0.load(Ordering::SeqCst),
            usize::from(!linted),
            "{command}"
        );
        let entry = &judged.acceptance[0];
        assert_eq!(
            passability::lint_finding(entry).is_some(),
            linted,
            "{command}"
        );
        if linted {
            let note = judge::author_note(entry);
            assert!(note.contains("`cargo test` takes one TESTNAME before `--`; put names after `-- --exact` or run one `cargo test` per name"));
            assert!(note.contains("'first' 'second'"));
            assert!(!note.contains("evidence tree"));
            assert!(!note.contains("host judge"));
            let findings = judge::not_judged_findings(Path::new("checks.json"), &judged);
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].text, note);
            assert_eq!(
                findings[0].gate_stage,
                Some(crate::command::workflow_gate::GateStage::Passability)
            );
        }
    }
}

#[tokio::test]
async fn cached_accepted_and_pinned_verdicts_cannot_override_lint() {
    let directory = tempfile::tempdir().unwrap();
    let client = CountingJudge::default();
    let roots = judge::JudgeRoots::default();
    let mut candidate = candidate("cargo test first second");
    let entry = &mut candidate.acceptance[0];
    entry.judgment.sampling = Some(serde_json::json!({
        "model": client.resolve_model_alias(&client.model_for_role("judge", "sonnet")),
        "provider": client.provider_id(),
        "logic_identity": current_logic_identity(&client).unwrap(),
        "named_file_digest": judge::named_files_content_digest(entry, &roots),
    }));
    let canonical = directory.path().canonicalize().unwrap();
    let key = entry_key(
        &client,
        &canonical,
        &candidate.prd.digest,
        &candidate.acceptance[0],
        &roots,
    )
    .unwrap();
    save_entry(directory.path(), &key, &candidate.acceptance[0]).unwrap();
    assert_eq!(
        load_entry(
            directory.path(),
            &key,
            &current_logic_identity(&client).unwrap(),
            &judge::named_files_content_digest(&candidate.acceptance[0], &roots)
        )
        .unwrap()
        .verdict,
        JudgeDecision::Accepted
    );
    let pinned = candidate.clone();
    let store = super::super::super::judge_store::JudgeStore::at(directory.path().join("saved"));
    let expected = BTreeSet::from(["AC-1".into()]);
    let saved = store
        .judge_subset(&client, candidate.clone(), &expected)
        .await
        .unwrap();
    assert_eq!(
        saved.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    let calls = client.0.load(Ordering::SeqCst);
    let mut judged = judge(
        directory.path(),
        directory.path(),
        &client,
        candidate,
        &BTreeSet::from(["AC-1".into()]),
        Some(&store),
        &roots,
    )
    .await
    .unwrap();
    assert_eq!(client.0.load(Ordering::SeqCst), calls);
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
    assert!(keep_pinned_entries(&mut judged, &pinned, &client, &roots).is_empty());
    assert!(passability::lint_finding(&judged.acceptance[0]).is_some());
}

#[tokio::test]
async fn repaired_entry_is_judged_and_supplementary_commands_are_linted() {
    let directory = tempfile::tempdir().unwrap();
    let client = CountingJudge::default();
    let roots = judge::JudgeRoots::default();
    let expected = BTreeSet::from(["AC-1".into()]);
    let mut first = candidate("cargo test first second");
    let mut sibling = first.acceptance[0].clone();
    sibling.id = "SUP-1".into();
    first.supplementary.push(sibling);
    let mut refused = judge(
        directory.path(),
        directory.path(),
        &client,
        first,
        &expected,
        None,
        &roots,
    )
    .await
    .unwrap();
    assert_eq!(client.0.load(Ordering::SeqCst), 0);
    assert!(passability::lint_finding(&refused.supplementary[0]).is_some());
    refused.supplementary.clear();
    refused.acceptance[0].check = candidate("cargo test first").acceptance.remove(0).check;
    let repaired = judge(
        directory.path(),
        directory.path(),
        &client,
        refused,
        &expected,
        None,
        &roots,
    )
    .await
    .unwrap();
    assert_eq!(client.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        repaired.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert!(passability::lint_finding(&repaired.acceptance[0]).is_none());
}

#[tokio::test]
async fn reauthor_subset_lints_before_judging_and_keeps_author_feedback() {
    // Republish and v3 mid-run authoring both use this subset judge through reauthor.
    let client = CountingJudge::default();
    let judged = judge::judge_entries_with_roots(
        &client,
        candidate("cargo test first second"),
        "sonnet",
        &judge::JudgeRoots::default(),
    )
    .await
    .unwrap();
    assert_eq!(client.0.load(Ordering::SeqCst), 0);
    assert!(judge::author_note(&judged.acceptance[0]).contains("invocations cargo rejects:"));
}

#[test]
fn absolute_programs_are_outside_the_lint_allow_list() {
    let repository = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let inside = repository.path().join("cargo");
    let outside = external.path().join("cargo");
    std::fs::write(&inside, "wrapper").unwrap();
    std::fs::write(&outside, "external").unwrap();
    for (program, expected) in [(inside, false), (outside, false)] {
        let mut contract = candidate(&format!("{} test first second", program.display()));
        let linted = passability::lint_commands(&mut contract);
        assert_eq!(!linted.is_empty(), expected);
    }
}

#[tokio::test]
async fn lint_decisions_survive_every_judge_stop_kind() {
    use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeIncomplete, FreezeProgress};
    for kind in 0..4 {
        let mut contract = candidate("cargo test first second");
        let mut sibling = candidate("cargo test first").acceptance.remove(0);
        sibling.id = "SUP-1".into();
        contract.supplementary.push(sibling);
        let error = judge::judge_each(contract, &judge::JudgeRoots::default(), |_| async move {
            let error: anyhow::Error = match kind {
                0 => judge::JudgeIncomplete::provider("provider stopped").into(),
                1 => anyhow::anyhow!("budget stopped"),
                2 => FreezeIncomplete::new(
                    &FreezeBudget::unlimited(),
                    &FreezeProgress::default(),
                    vec!["SUP-1".into()],
                )
                .into(),
                _ => FreezeIncomplete::unsaved("judge stopped").into(),
            };
            Err::<AcceptanceContract, _>(error)
        })
        .await
        .unwrap_err();
        if kind == 0 {
            let stopped = judge::JudgeIncomplete::caused(&error).unwrap();
            assert_eq!(stopped.decided.len(), 1);
            assert!(passability::lint_finding(&stopped.decided[0]).is_some());
        } else {
            assert!(judge::JudgeIncomplete::caused(&error).is_none());
            if kind == 1 {
                assert_eq!(error.to_string(), "budget stopped");
            }
        }
        if kind >= 2 {
            let frozen = FreezeIncomplete::caused(&error).unwrap();
            assert_eq!(frozen.known_findings().len(), 1);
            assert_eq!(frozen.known_findings()[0].subject, "AC-1");
            // A dry-run caller may attach the same findings again.
            let findings = frozen.known_findings().to_vec();
            let mut error = error;
            let frozen = error.downcast_mut::<FreezeIncomplete>().unwrap();
            frozen.add_known_findings(findings);
            assert_eq!(frozen.known_findings().len(), 1);
        }
    }
}

#[test]
fn floor_verifiers_keep_the_post_failure_passability_scope() {
    let mut contract = candidate("cargo test first second");
    contract.acceptance[0].check = AcceptanceCheck::Floor {
        contract: Box::new(
            archon_workflow::task_universe::WorkflowV2DeliverableContract {
                typed_verifier_command: Some("cargo test first second".into()),
                ..Default::default()
            },
        ),
    };
    assert!(passability::lint_commands(&mut contract).is_empty());
}
