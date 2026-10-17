//! Reverify observation persistence across an operational judge failure.

use super::*;

#[tokio::test]
async fn reverify_retry_reuses_every_observation_after_judge_failure() {
    let trees = trees(&[("AC-R-VERIFY", "test -f feature.txt", REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let resume = FreezeResume::staged("freeze-acceptance");
    let run_record = trees.set.project.path().join("run-record.json");
    let original_record = br#"{ "status": "Completed", "record": "unchanged" }"#;
    std::fs::write(&run_record, original_record).unwrap();

    let first = HostProbe::for_reverify_with_limits(
        trees.set.project.path(),
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    first
        .run_reverify_copy(&trees.contract(), &trees.ids())
        .await
        .expect("first reverify observations");
    let observed_copies = first.copies_made.load(SeqCst);
    assert!(
        observed_copies > 0,
        "first attempt runs each accepted check"
    );
    assert_eq!(first.take_reuse_audit().len(), trees.ids().len());

    // Stand for the passability judge failing after the probe finished. Its
    // FreezeIncomplete is operational and must leave the run record alone.
    let judge_error = crate::command::workflow_freeze_budget::FreezeIncomplete::awaiting_judge(
        &resume.budget,
        &resume.progress,
        vec!["AC-R-VERIFY".into()],
        "operational: provider authentication_error: invalid credential",
    );
    assert!(judge_error.to_string().contains("authentication_error"));

    let retry = HostProbe::for_reverify_with_limits(
        trees.set.project.path(),
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    retry
        .run_reverify_copy(&trees.contract(), &trees.ids())
        .await
        .expect("retry reuses saved observations");
    assert_eq!(retry.copies_made.load(SeqCst), 0, "no checks rerun");
    assert_eq!(retry.take_reuse_audit().len(), trees.ids().len());
    assert_eq!(std::fs::read(&run_record).unwrap(), original_record);
}
