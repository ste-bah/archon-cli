//! Regressions for the configured Scratch execution site.

use super::*;
use crate::command::workflow_task_set::executability::probe_tests::trees;
use archon_workflow::task_set_contract::TrustedCwd;

#[tokio::test]
async fn scratch_layout_unions_all_checks_and_keeps_each_check_siblings() {
    let trees = trees(&[
        (
            "AC-S-001",
            "grep -q ready ../sibling-a/fixture.txt",
            TrustedCwd::RepoRoot,
        ),
        (
            "AC-S-002",
            "grep -q ready ../sibling-b/fixture.txt && false",
            TrustedCwd::RepoRoot,
        ),
        ("AC-S-003", "test -f src/state.txt", TrustedCwd::RepoRoot),
    ]);
    for name in ["sibling-a", "sibling-b"] {
        let root = trees.outside.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("fixture.txt"), "ready\n").unwrap();
    }
    let scratch_parent = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch_parent.display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_reverify_with_limits(
        trees.set.project.path(),
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    assert!(matches!(probe.site, Site::Scratch(_)));

    let contract = trees.contract();
    let (evidence, _, missing, _) = probe
        .run_reverify_copy(&contract, &trees.ids())
        .await
        .expect("Scratch reverify observation");
    assert!(missing.is_empty(), "{missing:?}");
    let second = evidence
        .iter()
        .find(|item| item.acceptance_id == "AC-S-002")
        .expect("second sibling check ran");
    let layout = second.copy_layout.as_ref().expect("Scratch layout");
    assert!(
        layout
            .siblings
            .iter()
            .any(|sibling| sibling.path == Path::new("../sibling-a"))
    );
    assert!(
        layout
            .siblings
            .iter()
            .any(|sibling| sibling.path == Path::new("../sibling-b"))
    );
    assert_eq!(
        layout.check_siblings["AC-S-001"].len(),
        1,
        "layout siblings: {:?}; check map: {:?}",
        layout.siblings,
        layout.check_siblings
    );
    assert_eq!(
        layout.check_siblings["AC-S-001"][0].path,
        Path::new("../sibling-a")
    );
    assert_eq!(layout.check_siblings["AC-S-002"].len(), 1);
    assert_eq!(
        layout.check_siblings["AC-S-002"][0].path,
        Path::new("../sibling-b")
    );
    assert!(layout.check_siblings["AC-S-003"].is_empty());
    assert_eq!(second.exit_code, Some(1));
    assert!(second.operational_error.is_none());
    let at = super::super::silent::baseline_context(&probe, &contract, Some(layout));
    assert_eq!(
        super::super::silent::silent_failure(&contract, &second.result, &at),
        None,
        "the second check's behavioral failure is not discarded as setup"
    );
}
