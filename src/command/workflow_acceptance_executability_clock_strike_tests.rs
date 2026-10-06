//! Old total-clock strikes are not evidence against a check's author.
use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn issue356_legacy_total_clock_strikes_do_not_authorize_a_finding() {
    let trees = trees(&[("AC-B-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let contract = trees.contract();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let path = strike(&probe, &trees.base, &contract, "AC-B-001");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for bound in [1_800, 7_200, 20_000] {
        std::fs::write(
            &path,
            format!("it ran past the probe's per-check bound of {bound}s"),
        )
        .unwrap();
        let finding = strike_or_unproven(
            &probe,
            &trees.base,
            &contract,
            "AC-B-001",
            "a tool could not run",
            || ("author finding".into(), "host unproven".into()),
        );
        assert!(
            finding.is_none(),
            "the old clock alone cannot authorize an author finding"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a tool could not run",
            "only current non-clock evidence gets a strike"
        );
    }
}
