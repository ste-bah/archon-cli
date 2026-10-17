use std::path::Path;

pub(super) fn run_second_reverify(
    root: &Path,
    run_id: &str,
    run_dir: &Path,
    previous: &serde_json::Value,
) -> (std::path::PathBuf, serde_json::Value) {
    println!(
        "EVIDENCE reverify paused_second BEFORE: first_report={} run=Paused ledger_unconsumed=true",
        previous["id"]
    );
    let output = super::run(root, &["workflow", "reverify", run_id]);
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let progress = String::from_utf8_lossy(&output.stderr);
    for (subject, capability) in [
        ("TASK-X-010", "land-task-body"),
        ("TASK-X-020", "land-task-body"),
        ("task-set", "task-set-lint"),
        ("task-set", "requirements-trace"),
        ("frozen-task-set", "verify-frozen-acceptance"),
        ("frozen-task-set", "verify-frozen-skeleton"),
    ] {
        assert!(
            progress.contains(&format!(
                "reverify gate started subject={subject} capability={capability}"
            )),
            "paused reverify did not start {subject}/{capability}: {progress}"
        );
        assert!(
            progress.contains(&format!(
                "reverify gate finished subject={subject} capability={capability} verdict="
            )),
            "paused reverify did not finish {subject}/{capability}: {progress}"
        );
    }
    let (path, report) = super::newest_report(run_dir);
    assert_eq!(report["outcome"], "needs_remediation");
    for gate in report["set_gates"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report["frozen_gates"].as_array().unwrap())
    {
        assert!(
            gate["verdict"] == "accepted" || gate["verdict"] == "needs_remediation",
            "{gate:#}"
        );
    }
    let ledger = run_dir.join(format!(
        "decomposition/reverify/remediation-{}.json",
        report["id"].as_str().unwrap()
    ));
    assert!(ledger.is_file());
    println!(
        "EVIDENCE reverify paused_second AFTER: outcome={} set_gates={} frozen_gates={} all_gates_policy_verdicts=true run=Paused ledger_unconsumed=true",
        report["outcome"],
        report["set_gates"].as_array().unwrap().len(),
        report["frozen_gates"].as_array().unwrap().len()
    );
    (path, report)
}
