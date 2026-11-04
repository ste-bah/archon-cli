#![allow(non_snake_case)]
use super::super::super::{
    check_fault,
    probe_tests::{git, trees},
    silent,
    verdict::Context,
};
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

fn silent_texts(origin: BaselineOrigin) {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let result = CheckResult {
        acceptance_id: "AC-X-001".into(),
        exit_code: Some(1),
        stdout: vec![],
        stderr: vec![],
        operational_error: None,
        environment_note: None,
        quota_walk_count: 0,
        fault: None,
        classification: None,
        verdict_streams: Default::default(),
    };
    let at = Context::on_host_path(&contract);
    assert!(
        silent::settle_as(
            &probe,
            origin,
            &trees.base,
            &contract,
            &result,
            ("unavailable", UnprovenCause::Host),
            &at
        )
        .is_none()
    );
    let unproven = probe.take_unproven();
    assert!(unproven["AC-X-001"].contains(&format!("on the {} at", origin.tree_name())));
    let finding = silent::settle_as(
        &probe,
        origin,
        &trees.base,
        &contract,
        &result,
        ("unavailable", UnprovenCause::Host),
        &at,
    )
    .unwrap();
    assert!(finding.contains(&format!("on the {} at", origin.tree_name())));
    for cause in [UnprovenCause::SiteLacks, UnprovenCause::ProgramAbsent] {
        let finding = silent::settle_as(
            &probe,
            origin,
            &trees.base,
            &contract,
            &result,
            ("unavailable", cause),
            &at,
        )
        .unwrap();
        assert!(finding.contains(&format!("on the {} at", origin.tree_name())));
    }
    assert!(
        silent::settle_timed_out_with_diagnostics(
            &probe,
            origin,
            &trees.base,
            &contract,
            "AC-X-001",
            ""
        )
        .is_none()
    );
    let unproven = probe.take_unproven();
    assert!(unproven["AC-X-001"].contains(&format!("on the {} at", origin.tree_name())));
    let finding = silent::settle_timed_out_with_diagnostics(
        &probe,
        origin,
        &trees.base,
        &contract,
        "AC-X-001",
        "",
    )
    .unwrap();
    assert!(finding.contains(&format!("on the {} at", origin.tree_name())));
}

#[test]
fn P1_N7_a_recorded_silent_texts() {
    silent_texts(BaselineOrigin::RecordedBase);
}
#[test]
fn P1_N7_b_head_silent_texts() {
    silent_texts(BaselineOrigin::HeadFallback);
}
#[test]
fn P1_N7_c_probed_silent_texts() {
    silent_texts(BaselineOrigin::ProbedTree);
}

fn advice(origin: BaselineOrigin) {
    let lead = super::super::diagnostics::baseline_pass_lead_for_tree("AC-X-001", "abc", origin);
    assert!(lead.contains(&format!("passed on the {} at", origin.tree_name())));
    assert_eq!(
        lead.contains("make it fail on that tree"),
        origin == BaselineOrigin::RecordedBase
    );
    if origin != BaselineOrigin::RecordedBase {
        assert!(
            lead.contains(
                "must fail on the pre-implementation tree, or be proven a regression guard"
            )
        );
    }
    let text = format!("{lead} name a file that exists there; considered inputs: none");
    HostProbe::assert_p1_readers(&text, true);
    HostProbe::assert_p1_prompt(&text);
}
#[test]
fn P1_N8_a_recorded_advice() {
    advice(BaselineOrigin::RecordedBase);
}
#[test]
fn P1_N8_b_head_advice() {
    advice(BaselineOrigin::HeadFallback);
}
#[test]
fn P1_N8_c_probed_advice() {
    advice(BaselineOrigin::ProbedTree);
}

#[test]
fn P1_N9_a_commit_kind_uses_origin_despite_changed_lead() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    for origin in [
        BaselineOrigin::RecordedBase,
        BaselineOrigin::HeadFallback,
        BaselineOrigin::ProbedTree,
    ] {
        let baseline = Baseline {
            origin,
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        };
        let text = super::super::routes::no_moves(
            &probe,
            &baseline,
            &trees.contract(),
            "AC-X-001",
            "abc",
            &BTreeMap::from([("data/input".into(), "absent at the selected root".into())]),
            "rewritten lead",
        )
        .unwrap();
        assert_eq!(
            text.contains("pre-implementation commit"),
            origin == BaselineOrigin::RecordedBase
        );
    }
}

#[test]
fn P1_N9_b_mutation_strike_phase_survives_changed_text() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let settle = |phase, at| {
        check_fault::settle_in_phase(
            &probe,
            (Some(&trees.base), &contract, "AC-X-001"),
            check_fault::Fault::Ambiguous("signal TERM".into()),
            (at, phase),
            "unproven".into(),
        )
    };
    assert!(settle(check_fault::Phase::Baseline, "with inputs moved aside").is_none());
    assert!(settle(check_fault::Phase::Mutation, "with inputs moved aside").is_none());
    assert!(settle(check_fault::Phase::Mutation, "another location").is_some());
}

// The selected tree may be a recorded base or HEAD when no record exists.
// Exercise the producer and both routing readers, rather than a copied lead.
pub(super) async fn selected_tree_reader_text(recorded: bool) -> String {
    // Judge a real manifest: unlike an ordinary input, it is never moved.
    // Keep it in both selected commits so each real baseline run passes.
    let mut trees = trees(&[("AC-X-001", "test -f Cargo.toml", TrustedCwd::RepoRoot)]);
    std::fs::write(trees.repo.join("Cargo.toml"), "[workspace]\n").unwrap();
    git(&trees.repo, &["add", "Cargo.toml"]);
    git(&trees.repo, &["commit", "-qm", "manifest baseline"]);
    trees.base = git(&trees.repo, &["rev-parse", "HEAD"]);
    std::fs::write(trees.repo.join("feature.txt"), "updated feature").unwrap();
    git(&trees.repo, &["add", "feature.txt"]);
    git(&trees.repo, &["commit", "-qm", "after baseline"]);
    trees.head = git(&trees.repo, &["rev-parse", "HEAD"]);
    let empty_tasks = trees.repo.join("tasks-without-lock");
    std::fs::create_dir(&empty_tasks).unwrap();
    let tasks: &Path = if recorded {
        &trees.set.tasks
    } else {
        &empty_tasks
    };
    if recorded {
        let path = tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
        let mut lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        lock["baseline_commit"] = trees.base.clone().into();
        std::fs::write(path, serde_json::to_vec(&lock).unwrap()).unwrap();
    }
    let baseline = Baseline::for_task_set(&trees.repo, tasks).unwrap();
    assert_eq!(
        baseline.origin,
        if recorded {
            BaselineOrigin::RecordedBase
        } else {
            BaselineOrigin::HeadFallback
        }
    );
    assert_eq!(
        baseline.commit,
        if recorded { &trees.base } else { &trees.head }.as_str()
    );
    // Without a repository record the Git project itself supplies HEAD.
    let project = if recorded {
        trees.set.project.path()
    } else {
        &trees.repo
    };
    let probe = HostProbe::for_task_set(project, tasks);
    let contract = trees.contract();
    let discovery = mutation::input_discovery(&contract.acceptance[0], project, &trees.repo);
    assert!(discovery.eligible.is_empty());
    assert!(discovery.candidates.iter().any(|candidate| {
        candidate.path == "Cargo.toml"
            && candidate.exclusion == Some(mutation::ExclusionReason::ManifestOrSetup)
    }));
    let outcome = probe.prove_can_fail(&contract, &trees.ids()).await;
    assert!(outcome.unproven.is_empty(), "{:?}", outcome.unproven);
    assert!(outcome.proven.is_empty(), "{:?}", outcome.proven);
    let text = outcome
        .findings
        .get("AC-X-001")
        .unwrap_or_else(|| panic!("missing author finding: {:?}", outcome.findings))
        .clone();
    assert!(text.contains(&baseline.commit[..12]), "{text}");
    assert!(text.contains(super::super::super::CANNOT_FAIL), "{text}");
    if recorded {
        assert!(
            text.contains("passed on the pre-implementation tree at "),
            "{text}"
        );
    } else {
        assert!(
            text.contains("passed on the HEAD tree (no recorded base exists) at "),
            "{text}"
        );
        let (lead, _) = text.split_once(super::super::super::CANNOT_FAIL).unwrap();
        assert!(!lead.contains("pre-implementation"), "{text}");
        assert!(text.contains("The check must fail on the pre-implementation tree, or be proven a regression guard"), "{text}");
    }
    HostProbe::assert_p1_readers(&text, true);
    HostProbe::assert_p1_prompt(&text);
    text
}
