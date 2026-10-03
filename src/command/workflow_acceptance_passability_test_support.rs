//! Test helpers shared by the freeze tests (Issue 275).

/// The evidence pass answered "the absent feature" for every check of
/// `prompt`, for test judges that script only the adversarial batch.
pub(crate) fn all_accepted(prompt: &str) -> archon_workflow::llm_client_port::WorkflowAgentOutcome {
    let checks: Vec<serde_json::Value> =
        serde_json::from_str(prompt.split_once("Checks: ").expect("checks").1).expect("JSON");
    let decisions = (checks.iter())
            .map(|check| serde_json::json!({"id": check["id"], "verdict": "accepted", "counterexample": "no rule refused the check's own setup", "reason": "the failure is the absent feature"}))
            .collect::<Vec<_>>();
    archon_workflow::llm_client_port::WorkflowAgentOutcome {
        content: serde_json::json!({ "decisions": decisions }).to_string(),
        stop_reason: Some("end_turn".into()),
        ..Default::default()
    }
}

/// Make `root` a git checkout with one commit: the pre-implementation tree
/// a freeze probes its checks on (without one, every check is unproven).
pub(crate) fn commit_project(root: &std::path::Path) {
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    ] {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
