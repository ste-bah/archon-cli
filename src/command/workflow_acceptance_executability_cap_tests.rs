//! Issue 263: a freeze probe bounds each check by its site's own limit
//! (the configured `[workflow.acceptance_execution] timeout_secs`, else the
//! direct default), never a hard-coded cap; and what the host could not
//! prove under a staged freeze is resumable, never a failure of the run.

use archon_workflow::acceptance_scratch::DIRECT_DEFAULT_TIMEOUT_SECS;
use archon_workflow::task_set_contract::TrustedCwd;

use super::super::HostProbe;
use super::super::probe_tests::{Trees, trees};
use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};

/// Configure the scratch site of `trees` with `timeout_secs` per check.
pub(super) fn configure(trees: &Trees, timeout_secs: u64) {
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs={timeout_secs}\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.canonicalize().unwrap(),
            scratch
        ),
    )
    .unwrap();
}

#[test]
fn the_probe_check_cap_is_the_sites_own_limit() {
    let hermetic = trees(&[("AC-C-001", "true", TrustedCwd::RepoRoot)]);
    let probe = HostProbe::for_task_set(hermetic.set.project.path(), &hermetic.set.tasks);
    assert_eq!(probe.check_cap_secs, DIRECT_DEFAULT_TIMEOUT_SECS);

    // A check that needs a cold build longer than any fixed cap is given
    // the limit the operator configured.
    let scratch = trees(&[("AC-C-001", "true", TrustedCwd::RepoRoot)]);
    configure(&scratch, 7_200);
    let probe = HostProbe::for_task_set(scratch.set.project.path(), &scratch.set.tasks);
    assert_eq!(probe.check_cap_secs, 7_200);
}

#[tokio::test]
async fn a_staged_probe_returns_host_unproven_findings_for_freeze_orchestration() {
    let trees = trees(&[("AC-C-002", "sleep 30; true", TrustedCwd::RepoRoot)]);
    configure(&trees, 3);
    let resume = FreezeResume::saving(FreezeBudget::unlimited(), true);
    let probed = crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
        trees.set.project.path(),
        &trees.set.tasks,
        &trees.set.prd,
        &trees.contract(),
        &resume,
    )
    .await
    .expect("the probe returns findings before freeze orchestration decides to stall");
    assert!(
        probed.findings.iter().any(|finding| {
            finding.subject == "AC-C-002"
                && finding.remediation_scope == archon_workflow::RemediationScope::Operational
        }),
        "{:?}",
        probed.findings
    );
}
