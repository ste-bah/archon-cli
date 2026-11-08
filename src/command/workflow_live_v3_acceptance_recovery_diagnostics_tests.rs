use super::*;
use std::path::PathBuf;

fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".decompose.log" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    walk(root, root, &mut files);
    files
}

#[test]
fn recovery_refreeze_diagnostics_leave_files_and_publication_unchanged() {
    let run = run_fixture_with(&[("AC-F-001", "test -f missing/input.json", true)]);
    pre_implementation_head(&run);
    let launch = run.set.pin().identity();
    super::round_six::metadata(&run, &run.run_id, &launch);
    super::round_six::recover(&run);
    let prepare = || {
        crate::command::workflow_task_set::prepare_from_judged(
            run.set.project.path(),
            &run.set.tasks,
            &run.set.prd,
            archon_core::config::GateMode::Observe,
            &run.set.contract(),
            None,
            crate::command::workflow_task_set::capture_live_contract_digest(&run.set.tasks)
                .unwrap(),
        )
        .unwrap()
    };
    let mut before = prepare();
    let mut after = before.clone();
    let files = snapshot(run.set.project.path());
    fn stamp(
        mode: FreezeGateMode,
        findings: &[crate::command::workflow_gate::GateFinding],
    ) -> archon_workflow::task_set_contract::FreezeGateStamp {
        archon_workflow::task_set_contract::FreezeGateStamp {
            mode,
            finding_count: findings.len(),
            findings_digest: content_digest(format!("{findings:?}").as_bytes()),
            binary_commit: "fixture".into(),
            evaluated_at: "2026-11-08T00:00:00+00:00".into(),
        }
    }
    before.record_with_observations(false, stamp).unwrap();
    after.record_with_observations(true, stamp).unwrap();
    assert_eq!(files, snapshot(run.set.project.path()));
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
    let log = run.set.tasks.join(".decompose.log");
    let first = std::fs::read_to_string(&log).unwrap();
    assert!(first.contains("unowned_check_input"), "{first}");
    let mut repeat = prepare();
    repeat.record_recovery_refreeze().unwrap();
    assert_eq!(first, std::fs::read_to_string(log).unwrap());
    assert_eq!(files, snapshot(run.set.project.path()));
}
