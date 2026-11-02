use super::*;

#[test]
fn host_command_stalled_critic_carries_transient_envelope() {
    let budget = FreezeBudget::unlimited();
    let progress = FreezeProgress::default();
    let error = anyhow::Error::new(LintIncomplete::new(
        &budget,
        &progress,
        vec!["AC-X-001".into()],
    ));
    let envelope =
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error,
        )
        .into_envelope()
        .unwrap();
    assert_eq!(
        envelope.operational_error.unwrap().kind,
        "transient_host_fault"
    );
}

#[tokio::test(start_paused = true)]
async fn host_command_stalled_critic_preserves_existing_body_findings() {
    let temp = corpus();
    let source = LintSource::Tasks(
        temp.path()
            .join("tasks")
            .read_dir()
            .unwrap()
            .flatten()
            .find(|entry| entry.path().is_dir())
            .unwrap()
            .path(),
    );
    let LintSource::Tasks(root) = &source else {
        unreachable!()
    };
    let file = root
        .read_dir()
        .unwrap()
        .flatten()
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("## Focused Tests", "## Notes");
    std::fs::write(&file, text).unwrap();
    let deterministic = crate::command::topology_lint::evaluate_lint(
        temp.path(),
        &source,
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(
        !deterministic.findings.is_empty(),
        "fixture must have an actionable defect"
    );
    let critic = Critic::jumping_after(0, hand_clock().0);
    let error = tokio::time::timeout(
        Duration::from_secs(10000),
        evaluate_lint_with_fidelity_resumable(
            temp.path(),
            &source,
            archon_core::config::GateMode::Enforce,
            Ok(critic),
            &[],
            &staged(hand_clock().1),
        ),
    )
    .await
    .expect("critic must end")
    .expect_err("a stalled critic keeps its resumable error surface");
    let evaluation =
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error,
        );
    assert_eq!(evaluation.findings, deterministic.findings);
    assert_eq!(
        evaluation
            .into_envelope()
            .unwrap()
            .operational_error
            .unwrap()
            .kind,
        "transient_host_fault"
    );
}
