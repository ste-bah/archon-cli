//! N1/N2: one check's fault at the scratch site is that check's own
//! finding; the checks beside it keep their own verdicts (a process per
//! test, as a test runner gives each test its own process), and the staged freeze routes the
//! authored finding even while another check is unproven (per-target
//! results, as a keep-going build keeps them).

use super::super::probe_tests::{Trees, trees};
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

const REPO: TrustedCwd = TrustedCwd::RepoRoot;
/// Fails on the pre-implementation tree, passes once the feature lands.
const SOUND: &str = "test -f feature.txt";
/// Removes its own site's build-output link, then asserts as `SOUND` does.
const UNLINKS: &str = "rm -f target && test -f feature.txt";
/// Idle well past the site window below: the host's, unproven (timed out).
const STALLS: &str = "sleep 30; test -f feature.txt";

/// A scratch-site policy for `trees` whose per-check window is `window`.
fn scratch(trees: &Trees, window: u64) {
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs={window}\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch.display().to_string(),
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn a_check_that_breaks_its_own_site_is_its_finding_and_its_siblings_are_proven() {
    let trees = trees(&[
        ("AC-I-001", SOUND, REPO),
        ("AC-I-002", UNLINKS, REPO),
        (
            "AC-I-003",
            "test -f feature.txt && test -f src/state.txt",
            REPO,
        ),
    ]);
    scratch(&trees, 60);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    assert!(matches!(probe.site, Site::Scratch(_)));
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    let diagnostics = probe.take_diagnostics();
    assert!(
        unproven.is_empty(),
        "no sibling is left without a verdict: {unproven:?} {diagnostics:?}"
    );
    assert_eq!(
        findings.keys().collect::<Vec<_>>(),
        vec!["AC-I-002"],
        "{findings:?} {diagnostics:?}"
    );
    let finding = &findings["AC-I-002"];
    assert!(
        finding.contains("its own run") && finding.contains("target links"),
        "the author is told what its run changed: {finding}"
    );
}

/// H1 (renamed from `a_staged_freeze_routes_an_authored_finding_while_another_check_is_unproven`,
/// whose result held the authored finding alone): the check the host could
/// not prove stays an operational finding beside the authored one, so
/// nothing is published in any gate mode, observe included.
#[tokio::test]
async fn a_staged_freeze_routes_an_authored_finding_and_keeps_the_unproven_check_operational() {
    let trees = trees(&[
        ("AC-I-001", STALLS, REPO),
        ("AC-I-002", UNLINKS, REPO),
        ("AC-I-003", SOUND, REPO),
    ]);
    scratch(&trees, 3);
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probed = crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
        trees.set.project.path(),
        &trees.set.tasks,
        &trees.set.project.path().join("prd.md"),
        &trees.contract(),
        &resume,
    )
    .await
    .unwrap_or_else(|error| panic!("the authored finding is routed, not paused: {error:#}"));
    let probed_again =
        |text: &str| text.contains("AC-I-001") && text.contains("on the base commit");
    let routed: Vec<(&str, archon_workflow::RemediationScope)> = (probed.findings.iter())
        .map(|finding| (finding.subject.as_str(), finding.remediation_scope))
        .collect();
    assert_eq!(
        routed,
        vec![
            (
                "AC-I-002",
                archon_workflow::RemediationScope::CandidateArtifact
            ),
            ("AC-I-001", archon_workflow::RemediationScope::Operational),
        ],
        "{:?}",
        probed.findings.iter().map(|f| &f.text).collect::<Vec<_>>()
    );
    let text = &probed.findings[1].text;
    assert!(
        text.starts_with("check 'AC-I-001': operational: the host could not prove it"),
        "the host's, named as such: {text}"
    );
    let envelope = crate::command::workflow_gate::GateEvaluation::new("freeze", probed.findings)
        .into_envelope()
        .unwrap();
    for mode in [
        archon_core::config::GateMode::Observe,
        archon_core::config::GateMode::Enforce,
    ] {
        assert!(
            crate::command::workflow_host_command_decision::candidate_findings_prevent_publication(
                "freeze-acceptance",
                mode,
                &envelope
            ),
            "a contract with a never-proven check is not published under {mode:?}"
        );
    }
    // R5-4: the freeze after the repair probes the unproven check again --
    // no saved verdict, saved progress or stored judgment counts it as
    // passed -- and it is not accepted without that probe.
    let mut repaired = trees.contract();
    for entry in repaired.acceptance.iter_mut() {
        if entry.id == "AC-I-002" {
            entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
                command: "test -f feature.txt && test -f src/state.txt".into(),
                cwd: REPO,
            };
        }
    }
    let again = crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
        trees.set.project.path(),
        &trees.set.tasks,
        &trees.set.project.path().join("prd.md"),
        &repaired,
        &resume,
    )
    .await;
    let named = match &again {
        Ok(probed) => (probed.findings.iter())
            .map(|finding| finding.text.clone())
            .collect::<Vec<_>>(),
        Err(error) => vec![format!("{error:#}")],
    };
    assert!(
        named.iter().any(|text| probed_again(text)),
        "probed again and still not accepted: {named:?}"
    );
    assert!(
        !named.iter().any(|text| text.contains("AC-I-002")),
        "the repair is sound: {named:?}"
    );
}
