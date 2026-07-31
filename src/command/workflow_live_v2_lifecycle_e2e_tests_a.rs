
struct CannedLifecycleLlm {
    scenario: CannedLifecycleScenario,
    calls: Mutex<Vec<String>>,
    deliverable_contract_executed: AtomicBool,
    parameterized_contract_executed: AtomicBool,
    inventory_calls: AtomicUsize,
    verification_failure_emitted: AtomicBool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CannedLifecycleScenario {
    #[default]
    FullLifecycle,
    TriagePreservation,
    RepairPlanPreservation,
    ZeroTestRetry,
    InventoryTombstone,
}

#[async_trait::async_trait]
impl LlmClient for CannedLifecycleLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("D64 lifecycle harness must use the agent execution seam")
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        let prompt = request
            .messages
            .iter()
            .filter_map(|message| message.get("content").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let call_id = prompt_line(&prompt, "call_id:").unwrap_or_else(|| request.task.clone());
        self.calls.lock().expect("calls lock").push(call_id.clone());
        let input = prompt_input(&prompt);

        let content = if self.scenario == CannedLifecycleScenario::TriagePreservation
            && call_id == "semantic-triage-shape-repair-1"
        {
            accepted_result(
                "shape repair accounted for more outcomes but rewrote a predicate",
                serde_json::json!({
                    "implementation_failures": [{
                        "item_id": "failed-one",
                        "source_item_id": "source-one",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "implementation_failure",
                        "failed_predicate": "mutated predicate",
                        "source_residual_gap_ids": ["gap-one"],
                    }],
                    "retry_items": [{
                        "item_id": "failed-two",
                        "source_item_id": "source-two",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "retryable_verification_shape_issue",
                        "failed_predicate": "second predicate",
                        "source_residual_gap_ids": ["gap-two"],
                    }],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if self.scenario == CannedLifecycleScenario::RepairPlanPreservation
            && call_id == "post-remediation-verification-plan-repair-1-1-1"
        {
            accepted_result(
                "shape repair dropped source gap identity",
                serde_json::json!({
                    "items": [{
                        "item_id": "retry-check",
                        "source_item_id": "source-check",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "retryable_verification_shape_issue",
                        "failed_predicate": "focused check passes",
                        "focused_verification": "cargo test focused_check -- --exact",
                    }],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if self.scenario == CannedLifecycleScenario::ZeroTestRetry
            && call_id == "zero-test-triage-shape-repair-1"
        {
            accepted_result(
                "zero-match verification routed to an informative retry",
                serde_json::json!({
                    "implementation_failures": [],
                    "retry_items": [{
                        "item_id": "zero-check",
                        "source_item_id": "source-zero-check",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "retryable_verification_shape_issue",
                        "failed_predicate": "the focused test must match at least one test",
                        "source_residual_gap_ids": ["zero_test_match_verification"],
                    }],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if self.scenario == CannedLifecycleScenario::InventoryTombstone
            && call_id == "inventory-shape-repair-1"
        {
            accepted_result(
                "inventory repair attempted to tombstone scheduled work",
                serde_json::json!({
                    "items": [{
                        "item_id": "scheduled-item",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "tombstone": true,
                    }],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "canonical-implementation-inventory" {
            if self.inventory_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                needs_review_result(
                    "malformed inventory exposes schedulable retry work",
                    serde_json::json!({
                        "items": [{"item_id": "malformed"}],
                        "retry_items": [{"item_id": "retry-inventory-generation"}],
                    }),
                )
            } else {
                accepted_result(
                    "synthetic inventory",
                    serde_json::json!({
                        "items": synthetic_inventory_items(),
                        "unresolved_issues": [],
                    }),
                    Vec::new(),
                    Vec::new(),
                )
            }
        } else if call_id.starts_with("inventory-shape-repair-") {
            needs_review_result(
                "inventory repair remains malformed with schedulable retry work",
                serde_json::json!({
                    "items": [{"item_id": "malformed"}],
                    "retry_items": [{"item_id": "retry-inventory-generation"}],
                }),
            )
        } else if call_id.starts_with("target-file-discovery-wave-") {
            let mut attempted_flip = implementation_item(
                "implementation-refuted-noop-refutable",
                "TASK-EX-002",
                "src/refuted.rs",
                "Refuted work is implemented.",
            );
            attempted_flip["work_type"] = serde_json::json!("verified_noop");
            accepted_result(
                "target discovery attempted to restore the demoted noop",
                serde_json::json!({
                    "items": [attempted_flip],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("noop-evidence-repair-") {
            accepted_result(
                "no safe noop proof repair exists",
                serde_json::json!({ "items": [], "unresolved_issues": [] }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "verification-plan-1" {
            accepted_result(
                "focused verification plan",
                serde_json::json!({
                    "items": [
                        verification_item("verify-refuted", "TASK-EX-002", "src/refuted.rs"),
                        verification_item("verify-plain", "TASK-EX-003", "src/plain.rs"),
                        verification_item("verify-contract-source", "TASK-EX-004", "src/contract.rs"),
                        verification_item("verify-artifact-only", "TASK-EX-005", "../.archon/artifacts/artifact-only.json"),
                        verification_item("verify-parameterized", "TASK-EX-006", "src/parameterized.rs"),
                    ],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("verification-failure-triage-")
            && call_id.ends_with("-shape-repair-1")
        {
            accepted_result(
                "shape repair accounts for the concrete failed outcome",
                serde_json::json!({
                    "implementation_failures": [{
                        "item_id": "verify-plain",
                        "source_item_id": "verify-plain",
                        "canonical_task_ids": ["TASK-EX-003"],
                        "classification": "implementation_failure",
                        "failure_status": "needs_review",
                        "failure_evidence": "the first focused check failed",
                        "required_fix": "re-apply the neutral implementation",
                    }],
                    "retry_items": [],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("verification-failure-triage-") {
            accepted_result(
                "malformed triage omitted every canonical route",
                serde_json::json!({
                    "implementation_failures": [],
                    "retry_items": [],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("verification-remediation-inventory-") {
            accepted_result(
                "verification remediation inventory",
                serde_json::json!({
                    "items": [verification_remediation_item()],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("post-remediation-verification-plan-") {
            accepted_result(
                "post-remediation focused plan",
                serde_json::json!({
                    "items": [
                        verification_item("verify-refuted-remediated", "TASK-EX-002", "src/refuted.rs"),
                        verification_item("verify-plain-remediated", "TASK-EX-003", "src/plain.rs"),
                        verification_item("verify-contract-remediated", "TASK-EX-004", "src/contract.rs"),
                        verification_item("verify-artifact-remediated", "TASK-EX-005", "../.archon/artifacts/artifact-only.json"),
                        verification_item("verify-parameterized-remediated", "TASK-EX-006", "src/parameterized.rs"),
                    ],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "wave-completion-evidence-repair-1" {
            accepted_result(
                "wave completion is already represented by verification evidence",
                serde_json::json!({ "items": [] }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "artifact-inventory" {
            accepted_result(
                "declared artifact inventory",
                serde_json::json!({
                    "artifacts": [{"path": ".archon/artifacts/example-contract.json"}],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("adversarial-review-") {
            accepted_result(
                "synthetic adversarial review accepted",
                serde_json::json!({ "items": [] }),
                all_task_coverage(),
                vec![test_command("true", true, "review found no residual gaps")],
            )
        } else if call_id.starts_with("final-evidence-reconciliation-") {
            accepted_result(
                "final evidence reconciled",
                serde_json::json!({ "items": [] }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "final-zero-gap-audit" {
            accepted_result(
                "zero-gap audit accepted",
                serde_json::json!({ "items": [] }),
                all_task_coverage(),
                vec![test_command("true", true, "all synthetic checks passed")],
            )
        } else if request
            .task
            .contains("Re-verify repaired dependency-ready no-op proof")
            || request
                .task
                .contains("Verify the assigned dependency-ready no-op proof")
        {
            noop_proof_result(&call_id)
        } else if request
            .task
            .contains("Implement only the assigned dependency-ready item")
            || request
                .task
                .contains("Fix only the assigned focused-verification failure")
        {
            implementation_result(&request, &input, &call_id)?
        } else if request.task.contains("Run focused verification only")
            || request
                .task
                .contains("Run focused post-remediation verification only")
        {
            verification_result(
                &request,
                &input,
                &call_id,
                &self.deliverable_contract_executed,
                &self.parameterized_contract_executed,
                &self.verification_failure_emitted,
            )?
        } else if matches!(
            call_id.as_str(),
            "prd-task-review" | "repository-implementation-audit" | "acceptance-evidence-audit"
        ) {
            accepted_result(
                "synthetic read-only discovery",
                serde_json::json!({}),
                Vec::new(),
                Vec::new(),
            )
        } else {
            anyhow::bail!("unexpected D64 lifecycle harness call: {call_id}");
        };

        Ok(LlmResponse {
            content: content.to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

