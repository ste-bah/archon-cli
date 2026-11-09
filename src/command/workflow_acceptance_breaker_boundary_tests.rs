use super::tests::fixture;
use super::*;
use test_client::Client;
#[cfg(unix)]
use test_client::corpus;
#[test]
fn floor_excludes_nothing_and_cache_is_view_stable_private_and_schema_checked() {
    let f = fixture("cmd value");
    let keys = [(
        "value".into(),
        archon_workflow::base_snapshot::SnapshotKind::Blob,
    )];
    let first =
        materialise_view(f.parent.path(), f.source.path(), f.snapshot(), &keys, &[]).unwrap();
    let second =
        materialise_view(f.parent.path(), f.source.path(), f.snapshot(), &keys, &[]).unwrap();
    let mut entry = f.contract.acceptance[0].clone();
    entry.check=serde_json::from_value(serde_json::json!({"kind":"floor","contract":{"kind":"output","artifact_path":"value","min_instances":1}})).unwrap();
    assert!(named::check_named_keys(&entry, f.source.path(), f.source.path(), &first).is_empty());
    assert_eq!(first.files.len(), 1);
    for command in [
        "cmd ./value",
        "cmd nested/../value",
        "cmd --project=./value",
    ] {
        let mut check = f.contract.acceptance[0].clone();
        check.check = serde_json::from_value(
            serde_json::json!({"kind":"command","command":command,"cwd":"repo_root"}),
        )
        .unwrap();
        assert_eq!(
            named::check_named_keys(&check, f.source.path(), f.source.path(), &first),
            vec!["value"]
        );
    }
    let client = Client::default();
    let a = cache::key(
        &client,
        "breaker",
        &prompt::prompt("item", "keeps the good value", &first, ""),
        &first,
        "author",
    );
    let b = cache::key(
        &client,
        "breaker",
        &prompt::prompt("item", "keeps the good value", &second, ""),
        &second,
        "author",
    );
    assert_eq!(a, b);
    assert_ne!(
        a,
        cache::key(&client, "breaker", "different", &first, "author")
    );
    let mut probe = HostProbe::new(
        f.source.path().into(),
        f.source.path().into(),
        Site::Hermetic,
    );
    probe.resume.persist = true;
    let root = tempfile::tempdir().unwrap();
    let mut refusals = vec![];
    cache::save(
        &probe,
        Some(root.path()),
        &a,
        serde_json::json!("reply"),
        &mut refusals,
    );
    assert!(refusals.is_empty());
    assert_eq!(
        cache::read(&probe, Some(root.path()), &a, &mut refusals),
        Some(serde_json::json!("reply"))
    );
    let path = root.path().join("llm").join(format!("{a}.json"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    std::fs::write(&path, "{\"schema\":0,\"key\":\"bad\",\"answer\":{}}").unwrap();
    assert!(cache::read(&probe, Some(root.path()), &a, &mut refusals).is_none());
    assert_eq!(refusals, vec![format!("llm_cache_unreadable: {a}")]);
    probe.resume.persist = false;
    cache::save(
        &probe,
        Some(root.path()),
        "not-written",
        serde_json::json!("reply"),
        &mut refusals,
    );
    assert!(!root.path().join("llm/not-written.json").exists());
}
#[cfg(unix)]
#[tokio::test]
async fn prompt_contains_only_one_requirement_and_no_check_context() {
    use super::tests::request;
    for command in [
        "test \"$(sh v*)\" = good # distinctive_check_secret",
        "test -d . # distinctive_check_secret",
        "test -s v* # distinctive_check_secret",
    ] {
        let f = fixture(command);
        let mut p = f.probe(false);
        p.task_root = Some(f.parent.path().join("distinctive-task-root"));
        let c = Client::default();
        let mut req = request(&f, 20);
        let text = format!(
            "{}\n- REQ-OTHER-002: distinctive_other_requirement_secret\n",
            super::tests::TEXT
        );
        req.prd_text = &text;
        let r = prove_requirements(&p, &c, req).await;
        assert!(!r.verdicts.entries.is_empty());
        assert!(!c.calls.lock().unwrap().is_empty());
        for call in c.calls.lock().unwrap().iter() {
            assert_eq!(
                call.agent.key,
                archon_core::agents::harness::REQUIREMENT_BREAKER_AGENT
            );
            assert!(call.read_roots.is_empty() && call.write_roots.is_empty());
            for forbidden in [
                "distinctive_check_secret",
                "REQ-OTHER-002",
                "distinctive_other_requirement_secret",
                "distinctive-task-root",
                "check",
            ] {
                assert!(!call.task.contains(forbidden), "{forbidden}: {}", call.task);
            }
            assert!(!call.task.contains(&f.source.path().display().to_string()));
        }
    }
    let mut f = fixture("test -d .");
    f.classes.entries[0].deliverables[0].shared = true;
    let p = f.probe(false);
    let c = Client::default();
    let r = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(
        r.verdicts.entries[0].verdict,
        Verdict::NoCountableDeliverable
    );
    assert!(c.calls.lock().unwrap().is_empty());
}
#[cfg(unix)]
#[tokio::test]
async fn unavailable_judge_is_not_saved_and_is_retried_next_call() {
    use super::tests::request;
    let f = fixture("test \"$(sh v*)\" = good");
    let p = f.probe(true);
    let mut c = Client::default();
    c.answers.insert("relevant".into(), corpus("equivalent"));
    let first = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(first.verdicts.entries[0].verdict, Verdict::JudgeUnavailable);
    let cached: Vec<_> = std::fs::read_dir(first.store.as_ref().unwrap().join("llm"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(cached.len(), 1);
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cached[0]).unwrap()).unwrap();
    assert_eq!(value["answer"], corpus("breaker_gut"));
    let c = Client::default();
    let second = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(second.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    assert_eq!(second.verdicts.runs_executed, 0);
    assert_eq!(c.calls.lock().unwrap().len(), 0);
    assert_eq!(c.completions.lock().unwrap().len(), 1);
    assert!(!corpus("breaker").is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn failed_evidence_write_never_persists_inline_survivor_text() {
    use super::tests::request;
    let f = fixture("test -d .");
    let p = f.probe(true);
    let c = Client::default();
    let root =
        super::super::store::root(f.source.path(), f.lock.baseline_commit.as_deref().unwrap())
            .unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("evidence"), b"blocked").unwrap();
    let report = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(report.verdicts.entries[0].verdict, Verdict::WeakTest);
    assert!(
        report
            .verdicts
            .refusals
            .contains(&"verdicts_not_saved".into())
    );
    let path = super::super::store::summary_path(&root, &f.lock.digest)
        .unwrap()
        .with_file_name("verdicts.json");
    assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn survivor_evidence_belongs_to_a_surviving_check() {
    use super::tests::request;
    use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
    for (failure, expected) in [
        (
            "printf 'value:3:5: error invalid\\n'; exit 1",
            Verdict::NoViableBehaviourMutant,
        ),
        ("printf interrupted; kill -TERM $$", Verdict::WeakTest),
    ] {
        let mut f = fixture("printf observed_ok; test -d .");
        f.contract.acceptance[0].id = "b_pass".into();
        let mut other = f.contract.acceptance[0].clone();
        other.id = "a_no_status".into();
        other.check=serde_json::from_value(serde_json::json!({"kind":"command","command":format!("if test \"$(sh v*)\" != good; then {failure}; fi"),"cwd":"repo_root"})).unwrap();
        f.contract.acceptance.push(other);
        f.lock.baseline_results = BaselineResults::observed(
            f.lock.baseline_commit.clone(),
            &[
                ("a_no_status".into(), BaselineOutcome::Passed),
                ("b_pass".into(), BaselineOutcome::Passed),
            ]
            .into(),
            &f.contract,
        )
        .outcomes;
        f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
        f.classes.entries[0].checks = f
            .lock
            .baseline_results
            .iter()
            .map(|(id, result)| (id.clone(), Some(result.outcome)))
            .collect();
        let p = f.probe(true);
        let c = Client::default();
        let report = prove_requirements(&p, &c, request(&f, 20)).await;
        assert_eq!(report.verdicts.entries[0].verdict, expected);
        if expected == Verdict::NoViableBehaviourMutant {
            assert_eq!(
                c.calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.task.contains("Pick one function or block"))
                    .count(),
                2
            );
            assert!(
                report.verdicts.entries[0]
                    .attempts
                    .iter()
                    .flat_map(|a| &a.refusals)
                    .any(|r| r == "value:3")
            );
        }
        assert_eq!(
            report.verdicts.entries[0].survivors.len(),
            if expected == Verdict::WeakTest { 3 } else { 0 }
        );
        for evidence in &report.verdicts.entries[0].survivors {
            assert_eq!(evidence.checks, vec!["b_pass"]);
            let output = std::fs::read_to_string(
                report
                    .store
                    .as_ref()
                    .unwrap()
                    .join(evidence.output.as_ref().unwrap()),
            )
            .unwrap();
            assert!(output.contains("observed_ok"));
            assert!(!output.contains("interrupted"));
            assert!(!output.contains("error invalid"));
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn named_deliverables_still_consume_the_view_cap() {
    use super::tests::request;
    for excluded in [true, false] {
        let mut f = fixture(if excluded {
            "test -f bundle/huge_input"
        } else {
            "test -d ."
        });
        let path = if excluded {
            "bundle/huge_input"
        } else {
            "value"
        };
        if excluded {
            std::fs::create_dir(f.source.path().join("bundle")).unwrap();
        }
        std::fs::write(
            f.source.path().join(path),
            vec![b'x'; super::super::super::breaker_view::MAX_VIEW_BYTES + 1],
        )
        .unwrap();
        let snapshot = super::view_tests::snapshot(
            &f,
            if excluded {
                &["value", "bundle"]
            } else {
                &["value"]
            },
        );
        f.lock.baseline_commit = Some(snapshot.commit.clone());
        f.classes.baseline_commit = Some(snapshot.commit.clone());
        f.classes.base_snapshot = Some(snapshot);
        if excluded {
            let mut d = f.classes.entries[0].deliverables[0].clone();
            d.artifact_path = "bundle".into();
            f.classes.entries[0].deliverables.push(d);
        }
        let p = f.probe(false);
        let c = Client::default();
        let r = prove_requirements(&p, &c, request(&f, 20)).await;
        assert_eq!(
            r.verdicts.entries[0].verdict,
            Verdict::Unmutable,
            "{:?}",
            r.verdicts
        );
        assert!(c.calls.lock().unwrap().is_empty());
        assert!(
            r.verdicts.entries[0]
                .attempts
                .iter()
                .all(|a| a.refusals == vec!["view_too_large"])
        );
    }
}
