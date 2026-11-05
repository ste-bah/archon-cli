use super::super::super::check_fault::{Phase, settle_in_phase};
use super::super::super::{HostProbe, probe_tests::trees};
use super::origins::selected_tree_reader_text;
use super::rows::{candidate, pre};
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn every_p1_prerun_author_route_is_read_by_both_readers() {
    use mutation::ExclusionReason::*;
    for reasons in [
        vec![],
        vec![CheckMechanics, RepositoryRoot, WholeDirectoryTree],
        vec![
            ManifestOrSetup,
            SiblingFromRoot,
            OutsideCopiedTree,
            CheckWritesPathInText,
        ],
        vec![RepositoryAliasUnverified, ManifestOrSetup],
    ] {
        let text = pre(&reasons.into_iter().map(candidate).collect::<Vec<_>>());
        assert!(!text.contains(":;"), "{text}");
        HostProbe::assert_p1_readers(&text, true);
    }
}

#[test]
fn mutation_move_strike_survives_a_passing_baseline_after_resume() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let contract = trees.contract();
    let baseline = Baseline {
        origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    for attempt in 0..2 {
        let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
        super::super::super::silent::clear(&probe, &trees.base, &contract, "AC-X-001");
        assert!(
            super::super::routes::no_moves(
                &probe,
                &baseline,
                &contract,
                "AC-X-001",
                "base",
                &BTreeMap::from([("data/state.txt".into(), "move failed: I/O".into())]),
                "lead"
            )
            .is_none()
        );
        assert_eq!(
            probe.take_unproven_causes()["AC-X-001"].1,
            if attempt == 0 {
                UnprovenCause::Host
            } else {
                UnprovenCause::PermanentHost
            }
        );
    }
}

#[test]
fn no_status_does_not_blame_an_unmoved_input() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let contract = trees.contract();
    let why = "the mutation run did not record its exit status; input states: data/a=restored, data/b=unreported; check status/signal: signal/exit status unavailable";
    for attempt in 0..2 {
        let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
        super::super::super::silent::clear(&probe, &trees.base, &contract, "AC-X-001");
        let finding = settle_in_phase(
            &probe,
            (Some(&trees.base), &contract, "AC-X-001"),
            super::super::super::check_fault::Fault::Ambiguous(why.into()),
            ("with inputs moved aside", Phase::Mutation),
            "unproven".into(),
        );
        if attempt == 0 {
            assert!(finding.is_none());
        } else {
            let text = finding.unwrap();
            assert!(text.ends_with("make it end by its own assertion without being stopped"));
            assert!(!text.contains("remove data/b"));
            HostProbe::assert_p1_readers(&text, false);
        }
    }
}

#[test]
fn common_diagnostic_words_do_not_own_a_moved_source_file() {
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-X-001".into(),
        exit_code: Some(101),
        stdout: vec![],
        stderr: b"error[E0308]: mismatched types\n --> src/lib.rs:1:1\n".to_vec(),
        operational_error: None,
        environment_note: None,
        quota_walk_count: 0,
        fault: None,
        classification: None,
        verdict_streams: Default::default(),
    };
    for path in ["src/types.rs", "src/error.rs"] {
        assert!(!super::super::super::verdict::mutation_build_failure_owned(
            &result,
            &[path.into()]
        ));
    }
}

#[tokio::test]
async fn alias_layout_rebuild_is_attempted_only_once() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f ../repository/src/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let mut contract = trees.contract();
    // Match the real repository directory, without any project-specific alias.
    let alias = trees.repo.file_name().unwrap().to_str().unwrap();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut contract.acceptance[0].check
    {
        *command = format!("test -f ../{alias}/src/state.txt");
    }
    // The alias must exist in the source view before discovery can report
    // an unverified copy alias rather than a path outside the copied tree.
    let project = trees.outside.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let calls = std::cell::Cell::new(0);
    let discovery = super::super::routes::discover_with_rebuild(
        &contract.acceptance[0],
        &project,
        &trees.repo,
        None,
        || async {
            calls.set(calls.get() + 1);
            None
        },
    )
    .await;
    assert_eq!(calls.get(), 1, "{:?}", discovery.candidates);
    assert!(super::super::routes::alias_only(&discovery.candidates).is_some());
}

#[tokio::test]
async fn every_p1_author_text_is_classified_by_both_readers() {
    // Rows without author text (host faults and guard verdicts) deliberately
    // produce nothing for either reader. Cover every author-text row here.
    every_p1_prerun_author_route_is_read_by_both_readers(); // 1, 2, 3, 5
    for (command, code, moved, state, stderr, generated, recreated) in [
        (
            "test -f data/state.txt",
            0,
            false,
            "absent at the selected root",
            "",
            false,
            false,
        ), // 6
        (
            "test -f data/state.txt",
            0,
            false,
            "refused: symlink component",
            "",
            false,
            false,
        ), // 8
        (
            "cat data/state.txt; true",
            0,
            true,
            "restored",
            "",
            false,
            false,
        ), // 10
        (
            "test -f data/state.txt",
            0,
            true,
            "original-recreated",
            "",
            true,
            true,
        ), // 20
        (
            "test -f data/state.txt",
            0,
            true,
            "original-recreated",
            "",
            false,
            true,
        ), // 21
        (
            "test -f data/state.txt; python3 -c 'undefined_name()'",
            1,
            true,
            "restored",
            "Traceback (most recent call last):\n  File \"<string>\", line 1, in <module>\nNameError: name 'undefined_name' is not defined",
            false,
            false,
        ), // 28
        (
            "test -f data/state.txt; archon_p1_absent_program",
            127,
            true,
            "restored",
            "sh: archon_p1_absent_program: command not found",
            false,
            false,
        ), // 29
    ] {
        let (findings, _, _) =
            super::rows::injected(command, code, moved, state, stderr, generated, recreated).await;
        assert_eq!(findings.len(), 1);
        for text in findings.values() {
            HostProbe::assert_p1_readers(text, true);
        }
    }
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let baseline = Baseline {
        origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    let text = super::super::routes::no_moves(
        &probe,
        &baseline,
        &contract,
        "AC-X-001",
        "base",
        &BTreeMap::from([
            ("a".into(), "absent at the selected root".into()),
            ("b".into(), "refused: symlink component".into()),
        ]),
        &super::super::diagnostics::baseline_pass_lead("AC-X-001", "base"),
    )
    .unwrap();
    HostProbe::assert_p1_readers(&format!("{text}; considered inputs: a, b"), true); // 9
    for _ in 0..2 {
        let _ = super::super::super::silent::settle_timed_out(
            &probe,
            &trees.base,
            &contract,
            "AC-X-001",
        );
    }
    let text =
        super::super::super::silent::settle_timed_out(&probe, &trees.base, &contract, "AC-X-001")
            .unwrap();
    HostProbe::assert_p1_readers(&text, false); // 12
    for memo in [true, false] {
        // 13, 14
        let mut probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
        probe.memo = memo;
        let mut finding = None;
        for _ in 0..2 {
            finding = super::super::super::silent::settle_mutation_timeout(
                &probe,
                &trees.base,
                &contract,
                "AC-X-001",
                &["a".into()],
                "states",
            );
        }
        HostProbe::assert_p1_readers(&finding.unwrap(), false);
    }
    use super::super::super::check_fault::{Fault, settle_in_phase as settle};
    let text = settle(
        &probe,
        (Some(&trees.base), &contract, "AC-X-001"),
        Fault::Check {
            trigger: "output limit",
            remedy: "bound output",
            error: "overflow".into(),
        },
        ("with inputs moved aside", Phase::Mutation),
        "unproven".into(),
    )
    .unwrap();
    HostProbe::assert_p1_readers(&text, false); // 30
    for why in [
        "signal TERM",
        "the mutation run left input state unproven; input states: a=aside-missing; check status/signal: exit status 1",
    ] {
        // 31, 32
        let mut finding = None;
        for _ in 0..2 {
            finding = settle(
                &probe,
                (Some(&trees.base), &contract, "AC-X-001"),
                Fault::Ambiguous(why.into()),
                ("with inputs moved aside", Phase::Mutation),
                "unproven".into(),
            );
        }
        HostProbe::assert_p1_readers(&finding.unwrap(), false);
    }
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P1_F1_a_recorded_base_lead_matches_every_reader() {
    selected_tree_reader_text(true).await;
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P1_F1_b_head_fallback_lead_is_true_and_matches_every_reader() {
    selected_tree_reader_text(false).await;
}

#[test]
#[allow(non_snake_case)]
fn P1_F1_c_readers_accept_new_and_persisted_text_but_reject_unrelated_findings() {
    let text = pre(&[]);
    HostProbe::assert_p1_readers(&text, true);
    for legacy in [
        "passed on the pre-implementation tree",
        "passed on the selected tree",
    ] {
        let legacy_text = text
            .replace("passed on the pre-implementation tree", legacy)
            .replace(
                super::super::super::CANNOT_FAIL,
                "it cannot yet be shown to fail",
            );
        HostProbe::assert_p1_readers(&legacy_text, true);
    }
    HostProbe::assert_p1_readers("check 'AC-X-001': an unrelated finding", false);
}

#[test]
fn relabeling_the_lead_preserves_quoted_check_output() {
    let lead = super::super::diagnostics::baseline_pass_lead("AC-X-001", "base");
    let output = "quoted: pre-implementation tree; pre-implementation commit";
    let mut findings = BTreeMap::from([("AC-X-001".into(), format!("{lead} {output}"))]);
    super::super::diagnostics::label_findings(
        &mut findings,
        "base",
        super::super::super::BaselineOrigin::HeadFallback,
    );
    assert!(findings["AC-X-001"].ends_with(output));
    assert!(findings["AC-X-001"].contains("HEAD tree (no usable recorded base)"));
}

#[test]
fn mutation_program_remedies_appear_once() {
    for cause in [UnprovenCause::SiteLacks, UnprovenCause::ProgramAbsent] {
        let why = "specific cause; check the criterion with what the site provides";
        let text = super::super::helpers::mutation_defect_reason(
            &Some((why.into(), cause)),
            String::new(),
        );
        assert_eq!(
            text.matches("check the criterion with what the site provides")
                .count(),
            1,
            "{text}"
        );
        assert!(!text.contains("; Check"), "{text}");
    }
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P1_R7_b_task_set_lock_preserves_recorded_base_origin() {
    selected_tree_reader_text(true).await;
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P1_R7_c_task_set_without_lock_uses_head_fallback_origin() {
    selected_tree_reader_text(false).await;
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P1_R8_no_repository_has_no_lazy_baseline_or_author_lead() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let tasks = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), tasks.path());
    assert!(probe.baseline.is_none());
    let outcome = probe.prove_can_fail(&trees.contract(), &trees.ids()).await;
    assert!(outcome.findings.is_empty(), "{:?}", outcome.findings);
    assert!(outcome.proven.is_empty());
    let text = &outcome.unproven["AC-X-001"];
    assert_eq!(
        text,
        "the run has no pre-implementation tree to run it on, so it is not proven able to fail"
    );
    assert!(probe.baseline.is_none());
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P1_R7_d_with_baseline_head_preserves_explicit_head_fallback_origin() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    // head_of explicitly selects HEAD as a fallback; with_baseline must
    // preserve that choice rather than infer provenance from the probe.
    let tasks = tempfile::tempdir().unwrap();
    let baseline = Baseline::head_of(&trees.repo).unwrap();
    assert_eq!(baseline.origin, BaselineOrigin::HeadFallback);
    let probe =
        HostProbe::for_task_set(trees.set.project.path(), tasks.path()).with_baseline(baseline);
    let baseline = probe.baseline.as_ref().unwrap();
    assert_eq!(baseline.origin, BaselineOrigin::HeadFallback);
    let contract = trees.contract();
    let findings = super::super::prove(
        &probe,
        baseline,
        &contract,
        &super::super::super::contract_digest(&contract).unwrap(),
        &[],
        &["AC-X-001".into()],
        &BTreeMap::new(),
    )
    .await;
    let text = &findings["AC-X-001"];
    assert!(
        text.contains("passed on the HEAD tree (no usable recorded base) at "),
        "{text}"
    );
    let (lead, _) = text.split_once(super::super::super::CANNOT_FAIL).unwrap();
    assert!(!lead.contains("pre-implementation"), "{text}");
    assert!(
        text.contains(
            "The check must fail on the pre-implementation tree, or be proven a regression guard"
        ),
        "{text}"
    );
    HostProbe::assert_p1_readers(text, true);
    HostProbe::assert_p1_prompt(text);
}
