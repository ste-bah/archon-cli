//! Recorded outcomes exclude every path without a product verdict.
use super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[tokio::test]
async fn s4a_probe_records_only_baseline_passes_and_failures() {
    let trees = trees(&[
        ("AC-P", "test -f src/state.txt", TrustedCwd::RepoRoot),
        ("AC-F", "test -f feature.txt", TrustedCwd::RepoRoot),
        (
            "AC-S",
            "s4a_absent_silent 2>/dev/null",
            TrustedCwd::RepoRoot,
        ),
        ("AC-T", "sleep 3; test -f feature.txt", TrustedCwd::RepoRoot),
        ("AC-U", "absent_program_s4a_123", TrustedCwd::RepoRoot),
        (
            "AC-E",
            "cd ../absent_setup_s4a && test -f feature.txt",
            TrustedCwd::RepoRoot,
        ),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf())
        .with_check_cap(1);
    let _findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert_eq!(
        probe.take_baseline_passes(),
        BTreeSet::from(["AC-P".to_string()])
    );
    assert!(
        probe.take_baseline_passes().is_empty(),
        "passes are drained"
    );
    let runs = probe.take_baseline_runs().unwrap();
    assert_eq!(
        runs.failures.keys().map(String::as_str).collect::<Vec<_>>(),
        ["AC-F"]
    );
    trees.assert_live_untouched(copies.path());
}
