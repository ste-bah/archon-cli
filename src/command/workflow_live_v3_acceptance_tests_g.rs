//! Batch G (Issue-128): a project input that diverged from the repository's
//! tracked copy is the host's to repair, and a scratch site that still
//! cannot be built is ONE round-level operational error that routes nothing.
use super::*;

const TRACKED: &str = "{\"datasets\":[\"a\"]}";
const DIVERGED: &str = "{\"datasets\":[\"a\",\"regenerated\"]}";

/// The stage fixture in the scratch site's combined view, with a tracked
/// project input whose project copy a read-only verifier regenerated.
fn diverged(scratch: &std::path::Path) -> Fixture {
    let fixture = fixture(true);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(fixture.repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    std::fs::create_dir_all(fixture.repo.path().join("data")).unwrap();
    std::fs::write(fixture.repo.path().join("data/spec.json"), TRACKED).unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "tracked input"]);
    let head = git(&["rev-parse", "HEAD"]);
    std::fs::create_dir_all(fixture.project.path().join("data")).unwrap();
    std::fs::write(fixture.project.path().join("data/spec.json"), DIVERGED).unwrap();
    let policy = serde_json::json!({
        "repository": fixture.repo.path().canonicalize().unwrap(),
        "project": fixture.project.path().canonicalize().unwrap(),
        "task_root": fixture.task_root.canonicalize().unwrap(),
        "scratch_parent": scratch, "project_inputs": ["data"], "project_input_excludes": [],
        "combined": true, "toolchain_path": "/usr/bin:/bin", "environment": {},
        "environment_allowlist": [], "cargo_seed": null, "timeout_secs": 60,
        "output_bytes": 2048, "scratch_bytes": 16777216u64,
    });
    let metadata = serde_json::json!({
        "schema_version": "test",
        "observer_snapshot": {
            "schema_version": 1,
            "canonical_task_root_identity": fixture.task_root.canonicalize().unwrap(),
            "expected_artifact_paths": [],
            "native_execution": {"policy": policy, "source_commit": head},
        },
    });
    let path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
    fixture
}

fn status_of(
    record: &archon_workflow::v2::acceptance_stage::AcceptanceRoundRecordV1,
    id: &str,
) -> AcceptanceCheckStatus {
    record
        .checks
        .iter()
        .find(|check| check.check_id == id)
        .unwrap_or_else(|| panic!("{id} recorded: {record:#?}"))
        .status
}

/// The live collision, repaired: the host puts the tracked copy back (the
/// diverged one kept and logged), the scratch builds and the round runs —
/// REQ-1 passes and REQ-2 is an ordinary FAILED check for its owner.
#[tokio::test]
async fn a_diverged_tracked_input_is_restored_and_the_round_runs() {
    let scratch = tempfile::tempdir().unwrap();
    let fixture = diverged(scratch.path());
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let (record, _) = latest_round_record(&run_dir).unwrap().unwrap();
    assert!(
        record.operational_errors.is_empty(),
        "{:#?}",
        record.operational_errors
    );
    assert_eq!(status_of(&record, "REQ-1"), AcceptanceCheckStatus::Passed);
    assert_eq!(status_of(&record, "REQ-2"), AcceptanceCheckStatus::Failed);
    assert!(record.has_remediable_failures());
    assert_eq!(failing_ids(&result), vec!["REQ-2", "REQ-9"]);
    assert_eq!(
        std::fs::read_to_string(fixture.project.path().join("data/spec.json")).unwrap(),
        TRACKED
    );
    let log =
        std::fs::read_to_string(run_dir.join("write-coordination/project-inputs-restored.jsonl"))
            .unwrap();
    assert!(
        log.contains("data/spec.json") && log.contains("\"restored\":true"),
        "{log}"
    );
}

/// A divergence the host must not undo (a recorded landing put that copy
/// there) leaves the scratch unbuildable: one round-level operational error
/// naming both sources, no per-check results, a final round, nothing sent
/// to a task.
#[tokio::test]
async fn an_unrepairable_site_is_one_operational_error_that_routes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let fixture = diverged(scratch.path());
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let ledger = run_dir.join("write-coordination/project-inputs.jsonl");
    std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    let state = archon_workflow::write_coordinator::project_inputs::file_state(
        &fixture.project.path().join("data/spec.json"),
    );
    std::fs::write(
        &ledger,
        format!(
            "{}\n",
            serde_json::json!({"stage_id":"s","item_id":"i","task_ids":["TASK-F-002"],
                "path":"data/spec.json","outcome":"applied","before":"x","after":state,"at":1})
        ),
    )
    .unwrap();
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let (record, _) = latest_round_record(&run_dir).unwrap().unwrap();
    assert!(
        record.checks.is_empty(),
        "no per-check results: {:#?}",
        record.checks
    );
    assert!(record.final_round);
    assert!(!record.has_remediable_failures());
    let errors = record.operational_errors.join("\n");
    assert!(
        errors.contains("nonidentical scratch path collision")
            && errors.contains("tracked copy at commit")
            && errors.contains("data/spec.json"),
        "{errors}"
    );
    assert!(failing_ids(&result).is_empty());
    assert_eq!(result.data["final"], true);
    assert_eq!(
        std::fs::read_to_string(fixture.project.path().join("data/spec.json")).unwrap(),
        DIVERGED,
        "a landing's copy is not the host's to undo"
    );
}
