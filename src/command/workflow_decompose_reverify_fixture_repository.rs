//! Build the committed tree the reverify fixture's check probe copies.

use std::path::Path;

pub(super) fn record(project: &Path, task_root: &Path) {
    for args in [
        vec!["add", "prds", "tasks"],
        vec![
            "-c",
            "user.name=Reverify Test",
            "-c",
            "user.email=reverify@example.invalid",
            "commit",
            "-qm",
            "freeze fixture",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(project)
                .status()
                .unwrap()
                .success()
        );
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project)
        .output()
        .unwrap();
    assert!(output.status.success());
    archon_workflow::repository_record::write_repository_record(
        task_root,
        &archon_workflow::repository_record::RepositoryRecordV1 {
            schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
            repository_root: project.to_string_lossy().into_owned(),
            base_commit: String::from_utf8(output.stdout).unwrap().trim().to_string(),
            decomposition_run_id: "reverify-fixture".into(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
}
