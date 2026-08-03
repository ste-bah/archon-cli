use super::*;

pub(super) fn verification_scheduled_ids(
    contract: &LifecycleContract<'_>,
    inputs: &Value,
) -> BTreeSet<String> {
    let evidence = inputs
        .get("verificationEvidence")
        .or_else(|| inputs.get("verification_evidence"));
    let mut ids = BTreeSet::new();
    for event in support::array(evidence) {
        let plan_items = event
            .get("verificationPlan")
            .or_else(|| event.get("verification_plan"))
            .and_then(|plan| plan.get("items"));
        for item in support::array(plan_items) {
            ids.extend(contract.canonical_ids_for(&item));
        }
    }
    ids
}

pub(super) fn item_id(item: &Value) -> Option<String> {
    item.get("item_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub(super) fn compact_work_item(item: &Value) -> Value {
    serde_json::json!({
        "item_id": item.get("item_id").or_else(|| item.get("id")),
        "source_item_id": item.get("source_item_id"),
        "canonical_task_ids": item.get("canonical_task_ids"),
        "source_residual_gap_ids": item.get("source_residual_gap_ids"),
        "work_type": item.get("work_type"),
        "classification": item.get("classification"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    fn universe() -> WorkflowV2TaskUniverse {
        WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: Vec::new(),
            tasks: vec![WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-EX-001".to_string(),
                source_path: "tasks/TASK-EX-001.md".to_string(),
                acceptance_criteria: vec!["Evidence exists.".to_string()],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn blocked_report_reroutes_refuted_noop_without_targets() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let inputs = serde_json::json!({
            "readyNoopItems": [{
                "item_id": "noop-example",
                "work_type": "verified_noop",
                "canonical_task_ids": ["TASK-EX-001"],
                "dependency_ids": [],
                "acceptance_criteria": ["Evidence exists."],
                "target_files": [],
                "artifact_requirements": [],
            }],
            "failedNoopProof": [{
                "status": "needs_review",
                "canonical_task_ids": ["TASK-EX-001"],
                "residual_gaps": [{"id": "gap", "description": "evidence absent"}],
            }],
        });
        let mut state = TerminalGateState::default();

        assert!(matches!(
            decide(
                &contract,
                "blocked-noop-proof-failed-1",
                &inputs,
                &mut state
            ),
            TerminalGateDecision::Reroute(_)
        ));
        assert_eq!(state.pending_implementation_items.len(), 1);
    }

    #[test]
    fn terminal_gate_is_bounded_and_emits_when_no_work_exists() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let retry_inputs = serde_json::json!({
            "routing": {
                "retry_items": [{"item_id": "retry-one"}],
            },
        });
        let mut state = TerminalGateState::default();
        for _ in 0..MAX_REROUTES_PER_BLOCKED_ID {
            assert!(matches!(
                decide(
                    &contract,
                    "blocked-verification-failed-1",
                    &retry_inputs,
                    &mut state
                ),
                TerminalGateDecision::Reroute(_)
            ));
        }
        assert_eq!(
            decide(
                &contract,
                "blocked-verification-failed-1",
                &retry_inputs,
                &mut state
            ),
            TerminalGateDecision::Emit
        );
        assert_eq!(
            decide(
                &contract,
                "blocked-verification-failed-2",
                &serde_json::json!({}),
                &mut state
            ),
            TerminalGateDecision::Emit
        );
    }

    #[test]
    fn terminal_gate_evidence_compacts_schedulable_work() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let inputs = serde_json::json!({
            "routing": {
                "retry_items": [{
                    "item_id": "retry-one",
                    "canonical_task_ids": ["TASK-EX-001"],
                    "private_payload": "must-not-persist",
                }],
            },
        });
        let mut state = TerminalGateState::default();

        let TerminalGateDecision::Reroute(event) = decide(
            &contract,
            "blocked-verification-failed-1",
            &inputs,
            &mut state,
        ) else {
            panic!("retry work must reroute");
        };

        assert_eq!(event["work"][0]["item_id"], "retry-one");
        assert!(event.to_string().find("must-not-persist").is_none());
    }

    #[test]
    fn no_completion_reroutes_accepted_implementation_with_bare_unverified_id() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let inputs = serde_json::json!({
            "readyImplementationItems": [{
                "item_id": "implementation-example",
                "work_type": "implementation",
                "canonical_task_ids": ["TASK-EX-001"],
            }],
            "wave": {
                "outcomes": [{
                    "item_id": "implementation-wave-implementation-example",
                    "canonical_task_ids": ["EX-001"],
                    "status": "accepted",
                    "evidence": [{"kind": "inspection", "summary": "implemented"}],
                }],
            },
            "verificationEvidence": [],
        });
        let mut state = TerminalGateState::default();

        let TerminalGateDecision::Reroute(event) =
            decide(&contract, "blocked-no-completion-1", &inputs, &mut state)
        else {
            panic!("accepted unverified implementation must reroute");
        };

        assert_eq!(
            event["schedulable_work_kinds"],
            serde_json::json!(["accepted_unverified_implementation"])
        );
    }
}
