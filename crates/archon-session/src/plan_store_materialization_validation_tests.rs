use super::*;

#[test]
fn canonical_generation_validator_rejects_invalid_input_before_claim() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "canonical-generation-validation";
    let mut plan = PlanDocument::new("canonical-generation-plan", "Canonical generation");
    plan.status = PlanStatus::Approved;
    plan.steps.push(PlanStep {
        task_id: Some("canonical-task".into()),
        ..step(1, "canonical step", PlanStepStatus::Pending)
    });
    let task = PersistedPlanTask {
        task_id: "canonical-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "canonical step".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };

    approve_for_materialization(&store, session_id, &mut plan);
    let invalid = [
        ("empty", Vec::new()),
        (
            "cross-plan",
            vec![PersistedPlanTask {
                plan_id: "other-plan".into(),
                ..task.clone()
            }],
        ),
        (
            "task-id",
            vec![PersistedPlanTask {
                task_id: "other-task".into(),
                ..task.clone()
            }],
        ),
        (
            "description",
            vec![PersistedPlanTask {
                description: "other description".into(),
                ..task.clone()
            }],
        ),
        (
            "status",
            vec![PersistedPlanTask {
                status: "Running".into(),
                ..task.clone()
            }],
        ),
        (
            "extra",
            vec![
                task.clone(),
                PersistedPlanTask {
                    task_id: "extra-task".into(),
                    plan_step: 2,
                    ..task.clone()
                },
            ],
        ),
        ("duplicate", vec![task.clone(), task.clone()]),
    ];
    for (label, tasks) in invalid {
        let error = store
            .claim_plan_materialization_with_tasks(
                &test_authority(&store, session_id),
                session_id,
                &plan,
                &tasks,
            )
            .expect_err(label);
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidInput,
            "{label}: {error}"
        );
        assert_eq!(
            store
                .load_plan(session_id, &plan.id)
                .unwrap()
                .expect("durable approved plan")
                .to_json(),
            plan.to_json(),
            "{label}"
        );
        assert!(
            store.load_plan_tasks(session_id).unwrap().is_empty(),
            "{label}"
        );
    }

    store
        .claim_plan_materialization_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            &[task],
        )
        .expect("valid canonical generation");
}
