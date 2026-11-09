use super::super::tests::Fixture;
use super::*;
#[cfg(unix)]
use test_client::Client;
pub(super) fn fixture(command: &str) -> Fixture {
    let mut f = Fixture::new(command);
    std::fs::write(f.source.path().join("value"), VALUE).unwrap();
    let snapshot = super::view_tests::snapshot(&f, &["value"]);
    f.lock.baseline_commit = Some(snapshot.commit.clone());
    f.classes.baseline_commit = Some(snapshot.commit.clone());
    f.classes.base_snapshot = Some(snapshot);
    for entry in &mut f.classes.entries {
        entry.id = "REQ-VALUE-001".into();
    }
    f.contract.acceptance[0].covers = vec!["REQ-VALUE-001".into()];
    f.lock.baseline_results = archon_workflow::baseline_results::BaselineResults::observed(
        f.lock.baseline_commit.clone(),
        &[(
            "check".into(),
            archon_workflow::baseline_results::BaselineOutcome::Passed,
        )]
        .into(),
        &f.contract,
    )
    .outcomes;
    f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
    f
}
pub(super) const VALUE: &[u8] = b"value() { printf good; }\nvalue\n";
pub(super) const TEXT: &str = "- REQ-VALUE-001: It keeps the good value for every input.\n";
#[cfg(unix)]
pub(super) fn request(f: &Fixture, budget: u64) -> VerdictRequest<'_> {
    VerdictRequest {
        contract: &f.contract,
        chain_digest: &f.lock.digest,
        lock: &f.lock,
        classification: &f.classes,
        prd_text: TEXT,
        max_runs: budget,
        min_deadline_secs: 2,
        llm_deadline_secs: 20,
        call_limits: Default::default(),
    }
}
#[test]
fn parse_scripted_object_and_byte_edits() {
    let f = fixture("unused");
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    for text in [
        super::parse_tests::proposals().to_string(),
        super::parse_tests::proposals()
            .to_string()
            .replace('\n', "\r\n"),
    ] {
        let parsed = prompt::parse(&text, "task", "REQ-VALUE-001", TEXT, &view).unwrap();
        assert_eq!(parsed.mutants.len(), 3);
        assert!(parsed.refusals.is_empty());
        for (m, _) in parsed.mutants {
            assert!(m.patch.ends_with('\n'));
            assert!(m.patch.contains("--- a/value"));
        }
    }
    let raw = prompt::object(&super::parse_tests::proposals().to_string()).unwrap();
    assert_eq!(
        prompt::parse(&raw.to_string(), "task", "item", TEXT, &view)
            .unwrap()
            .mutants
            .len(),
        3
    );
    for (field, value, code) in [
        ("find", "absent", "find_absent"),
        ("key", "elsewhere", "key_not_in_view"),
        ("find", "", "find_ambiguous"),
    ] {
        let mut bad = raw.clone();
        bad["mutants"][0]["edits"][0][field] = value.into();
        let parsed = prompt::parse(&bad.to_string(), "task", "item", TEXT, &view).unwrap();
        assert!(
            parsed
                .refusals
                .iter()
                .any(|r| r.starts_with(&format!("{code}:")))
        );
        assert_eq!(parsed.mutants.len(), 2);
    }
    for word in ["the value", "unrelated different behaviour", ""] {
        let mut bad = raw.clone();
        bad["mutants"][0]["breaks"] = word.into();
        assert!(
            prompt::parse(&bad.to_string(), "task", "item", TEXT, &view)
                .unwrap()
                .refusals
                .iter()
                .any(|r| r.starts_with("breaks_not_in_requirement:"))
        );
    }
    for change in ["kind", "count", "truncated"] {
        let mut bad = raw.clone();
        if change == "kind" {
            bad["mutants"][0]["kind"] = "unknown".into();
        }
        if change == "count" {
            bad["mutants"].as_array_mut().unwrap().pop();
        }
        let text = if change == "truncated" {
            format!("```json\n{bad}")
        } else {
            bad.to_string()
        };
        let parsed = prompt::parse(&text, "task", "item", TEXT, &view);
        if change == "truncated" {
            assert!(parsed.is_err());
        } else {
            let parsed = parsed.unwrap();
            assert_eq!(parsed.mutants.len(), 2);
            if change == "kind" {
                assert!(parsed.refusals[0].starts_with("proposal_invalid:"));
            }
        }
    }
}
#[test]
fn view_is_snapshot_only_and_drop_removes_marker() {
    let f = fixture("unused");
    std::fs::write(f.source.path().join("value"), b"live secret").unwrap();
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    assert_eq!(std::fs::read(view.root().join("value")).unwrap(), VALUE);
    assert_eq!(view.files.len(), 1);
    assert!(!view.root().join("other").exists());
    let root = view.root().to_path_buf();
    let marker = owner::marker(&root);
    assert!(marker.exists());
    drop(view);
    assert!(!root.exists());
    assert!(!marker.exists());
    let empty = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &["value".into()],
    )
    .unwrap();
    assert!(empty.files.is_empty());
}
#[test]
fn directory_operands_preserve_views_but_file_operands_exclude() {
    for command in ["cd . && check", "cmd .", "cmd src"] {
        let f = fixture(command);
        let view = materialise_view(
            f.parent.path(),
            f.source.path(),
            f.snapshot(),
            &[(
                "value".into(),
                archon_workflow::base_snapshot::SnapshotKind::Blob,
            )],
            &[],
        )
        .unwrap();
        assert!(
            named::check_named_keys(
                &f.contract.acceptance[0],
                f.source.path(),
                f.source.path(),
                &view
            )
            .is_empty()
        );
    }
    let f = fixture("cmd value");
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    assert_eq!(
        named::check_named_keys(
            &f.contract.acceptance[0],
            f.source.path(),
            f.source.path(),
            &view
        ),
        vec!["value"]
    );
}
#[cfg(unix)]
#[tokio::test]
async fn real_engine_done_weak_budget_and_persist() {
    for (command, want) in [
        ("test \"$(sh v*)\" = good", Verdict::DoneCandidate),
        ("test -d .", Verdict::WeakTest),
    ] {
        let f = fixture(command);
        let p = f.probe(true);
        let client = Client::default();
        let first = prove_requirements(&p, &client, request(&f, 10)).await;
        assert_eq!(
            first.verdicts.entries[0].verdict, want,
            "{:?}",
            first.verdicts
        );
        assert_eq!(first.verdicts.rounds, 1);
        assert_eq!(first.verdicts.runs_executed, 5);
        if want == Verdict::WeakTest {
            let owed = first.verdicts.entries[0].owed.as_ref().unwrap();
            assert_eq!(owed.sup_id, "SUP-REQ-VALUE-001");
            assert!(
                owed.survivors.iter().all(|s| first
                    .store
                    .as_ref()
                    .unwrap()
                    .join(&s.patch)
                    .is_file())
            );
        }
        {
            let calls = client.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            let call = &calls[0];
            assert!(call.read_roots.is_empty() && call.write_roots.is_empty());
            assert_eq!(
                call.agent.tool_access,
                archon_workflow::llm_client_port::WorkflowAgentToolAccess::ReadOnly
            );
            assert_eq!(
                call.allowed_tools,
                vec![
                    crate::command::workflow_live::EXACT_TOOL_POLICY_MARKER,
                    "Read",
                    "Grep",
                    "Glob"
                ]
            );
            assert!(!call.task.contains(command));
            assert!(!call.task.contains(&f.source.path().display().to_string()));
            assert!(!call.cwd.as_ref().unwrap().exists());
        }
        let reused = Client::default();
        let second = prove_requirements(&p, &reused, request(&f, 0)).await;
        assert_eq!(second.verdicts.entries[0].verdict, want);
        assert_eq!(second.verdicts.runs_executed, 0);
        assert!(
            reused.calls.lock().unwrap().is_empty()
                && reused.completions.lock().unwrap().is_empty()
        );
    }
    let f = fixture("test \"$(sh v*)\" = good");
    let p = f.probe(false);
    let client = Client::default();
    let report = prove_requirements(&p, &client, request(&f, 1)).await;
    assert_eq!(report.verdicts.entries[0].verdict, Verdict::BudgetExhausted);
    assert!(report.store.is_none());
}
#[cfg(unix)]
#[tokio::test]
async fn provider_circuit_refusal_and_single_identity() {
    let f = fixture("test -d .");
    let p = f.probe(false);
    let client = Client {
        fail: true,
        ..Default::default()
    };
    let r = prove_requirements(&p, &client, request(&f, 10)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::BreakerUnavailable);
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert!(client.completions.lock().unwrap().is_empty());
    let client = Client {
        aliases: [
            ("first".into(), "one".into()),
            ("second".into(), "one".into()),
        ]
        .into(),
        ..Default::default()
    };
    let r = prove_requirements(&p, &client, request(&f, 10)).await;
    // A surviving body-gut is a real survivor even with only one model.
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::WeakTest);
    assert!(!r.verdicts.judges.equivalence_distinct);
    assert_eq!(client.completions.lock().unwrap().len(), 2);
}
