use super::*;

impl CanaryAgentClient {
    pub(super) fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            prompts: CanaryMutex::new(Vec::new()),
        }
    }

    fn artifact_path(&self) -> PathBuf {
        self.project_root.join(CANARY_ARTIFACT_REL)
    }

    pub(super) fn artifact_exists(&self) -> bool {
        self.artifact_path().is_file()
    }

    fn write_artifact_if_instructed(&self, prompt: &str) -> bool {
        if !prompt.contains(CANARY_TASK_ID) || !prompt.contains(".archon/artifacts/") {
            return false;
        }
        let path = self.artifact_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("artifact parent dir");
        }
        std::fs::write(
            &path,
            "# TASK-TDL-001 Gap Audit\n\nEvidence written by instructed agent.\n",
        )
        .expect("artifact write");
        true
    }

    fn accepted(summary: &str, data: serde_json::Value) -> String {
        serde_json::json!({
            "status": "accepted",
            "summary": summary,
            "evidence": [
                { "kind": "inspection", "summary": summary }
            ],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [],
            "residual_gaps": [],
            "data": data
        })
        .to_string()
    }

    fn implementation_item() -> serde_json::Value {
        serde_json::json!({
            "work_type": "implementation",
            "item_id": "impl-TASK-TDL-001",
            "canonical_task_ids": [CANARY_TASK_ID],
            "dependency_ids": [],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": [
                "Gap audit implemented and evidence artifact recorded"
            ],
            "focused_verification": "check gap-audit artifact evidence exists",
            // Mirrors wf-afae6bee: the task pack declares the artifact, but the
            // inventory item never carries it, so nothing instructs the
            // implementing agent. The requirement only resurfaces at
            // verification time (host-side inference from the PRD layout).
            "artifact_requirements": []
        })
    }

    fn verification_item() -> serde_json::Value {
        serde_json::json!({
            "item_id": "verify-TASK-TDL-001-artifact",
            "canonical_task_ids": [CANARY_TASK_ID],
            "focused_verification": format!(
                "confirm artifact evidence file exists at {CANARY_ARTIFACT_REL}"
            ),
            "expected_evidence": "gap-audit artifact present under projectArtifactRoot",
            "artifact_requirements": [CANARY_ARTIFACT_REL],
            "source_item_id": "impl-TASK-TDL-001"
        })
    }

    pub(super) fn respond(&self, prompt: &str) -> String {
        // Inventory-shaped reducers: return the canonical single-task inventory.
        if prompt.contains("produce dependency-owned inventory items")
            || prompt.contains("Repair inventory shape")
            || prompt.contains("Repair dependency graph defects")
        {
            return Self::accepted(
                "Canonical inventory for TASK-TDL-001.",
                serde_json::json!({ "items": [Self::implementation_item()] }),
            );
        }

        // Verification planners and verification repair reducers: always demand
        // the artifact evidence check, exactly as the real run's reducers did.
        if prompt.contains("Plan focused verification")
            || prompt.contains("Repair an empty focused verification plan")
            || prompt.contains("Repair failed focused verification")
            || prompt.contains("Repair malformed focused verification retry output")
            || prompt.contains("Plan focused verification after write remediation")
            || prompt.contains("Repair malformed post-remediation verification output")
        {
            return Self::accepted(
                "Focused verification plan for TASK-TDL-001 artifact evidence.",
                serde_json::json!({ "items": [Self::verification_item()] }),
            );
        }

        // Focused verification agents: check the filesystem honestly. Mirror the
        // real run: the failure is NOT classified actionable_implementation_failure,
        // so triage never routes it to write remediation.
        if prompt.contains("Run focused verification")
            || prompt.contains("Run repaired focused verification")
            || prompt.contains("Run focused post-remediation verification")
        {
            if self.artifact_exists() {
                return serde_json::json!({
                    "status": "accepted",
                    "summary": "Artifact evidence present.",
                    "evidence": [
                        { "kind": "inspection", "summary": "gap-audit artifact found" }
                    ],
                    "artifacts": [{
                        "id": "gap-audit-evidence",
                        "path": CANARY_ARTIFACT_REL,
                        "description": "verified gap audit evidence artifact"
                    }],
                    "commands_run": [{
                        "kind": "inspect",
                        "command": format!("test -f {CANARY_ARTIFACT_REL}"),
                        "status": "succeeded",
                        "exit_code": 0,
                        "output_summary": "artifact evidence file present"
                    }],
                    "files_read": [
                        { "path": CANARY_ARTIFACT_REL, "purpose": "artifact evidence check" }
                    ],
                    "files_changed": [],
                    "task_coverage": [],
                    "residual_gaps": [],
                    "data": {
                        "item_id": "verify-TASK-TDL-001-artifact",
                        "canonical_task_ids": [CANARY_TASK_ID],
                        "artifacts_checked": [CANARY_ARTIFACT_REL]
                    }
                })
                .to_string();
            }
            return serde_json::json!({
                "status": "failed",
                "summary": "Required artifact evidence missing.",
                "evidence": [
                    { "kind": "inspection", "summary": "gap-audit artifact absent" }
                ],
                "artifacts": [],
                "commands_run": [{
                    "kind": "inspect",
                    "command": format!("test -f {CANARY_ARTIFACT_REL}"),
                    "status": "failed",
                    "exit_code": 1,
                    "output_summary": "artifact evidence file missing"
                }],
                "files_read": [],
                "files_changed": [],
                "task_coverage": [],
                "residual_gaps": [],
                "data": {
                    "item_id": "verify-TASK-TDL-001-artifact",
                    "canonical_task_ids": [CANARY_TASK_ID],
                    "artifacts_checked": [CANARY_ARTIFACT_REL],
                    "verification_failure_class": "artifact_evidence_missing"
                }
            })
            .to_string();
        }

        // Remediation-inventory reducers: echo the failure evidence the runtime
        // handed us into one remediation item, exactly as a real reducer would.
        if prompt.contains("Create remediation items only for non-accepted")
            || prompt.contains("Repair an empty or malformed remediation inventory")
            || prompt.contains("Repair unresolved remediation outcomes")
            || prompt.contains("Create write-capable remediation items only for actionable")
        {
            return Self::accepted(
                "Remediation inventory for TASK-TDL-001.",
                serde_json::json!({
                    "items": [{
                        "work_type": "implementation",
                        "item_id": "remediate-TASK-TDL-001-1",
                        "source_item_id": "impl-TASK-TDL-001",
                        "canonical_task_ids": [CANARY_TASK_ID],
                        "dependency_ids": [],
                        "target_files": ["src/lib.rs"],
                        "acceptance_criteria": [
                            "Gap audit implemented and evidence artifact recorded"
                        ],
                        "failure_status": "failed",
                        "failure_evidence": "implementation outcome was not accepted",
                        "required_fix": "complete the gap audit implementation for TASK-TDL-001",
                        "verification_requirements": [
                            "focused verification of TASK-TDL-001"
                        ],
                        "focused_verification": "check gap-audit artifact evidence exists",
                        "artifact_requirements": [CANARY_ARTIFACT_REL]
                    }]
                }),
            );
        }

        // Implementation / remediation agents: obey instructions literally.
        if prompt.contains("Implement only the assigned dependency-ready item")
            || prompt.contains("Remediate only the assigned unresolved item")
            || prompt.contains("Run follow-up remediation")
            || prompt.contains("Fix only the assigned focused-verification failure")
        {
            let wrote = self.write_artifact_if_instructed(prompt);
            let artifacts = if wrote {
                serde_json::json!([{
                    "id": "gap-audit-evidence",
                    "path": CANARY_ARTIFACT_REL,
                    "description": "gap audit evidence artifact"
                }])
            } else {
                serde_json::json!([])
            };
            return serde_json::json!({
                "status": "accepted",
                "summary": "Implemented TASK-TDL-001 gap audit.",
                "evidence": [
                    { "kind": "implementation", "summary": "gap audit implemented" }
                ],
                "artifacts": artifacts,
                "commands_run": [{
                    "kind": "test",
                    "command": "cargo test gap_audit",
                    "status": "succeeded",
                    "exit_code": 0,
                    "output_summary": "focused gap audit test passed"
                }],
                "files_read": [],
                "files_changed": [
                    { "path": "src/lib.rs", "purpose": "gap audit entry point" }
                ],
                "task_coverage": [
                    {
                        "task_id": CANARY_TASK_ID,
                        "status": "accepted",
                        "summary": "gap audit implemented",
                        "evidence": [
                            { "kind": "implementation", "summary": "focused gap audit evidence" }
                        ]
                    }
                ],
                "residual_gaps": [],
                "data": {
                    "item_id": "impl-TASK-TDL-001",
                    "canonical_task_ids": [CANARY_TASK_ID]
                }
            })
            .to_string();
        }

        // Artifact inventory reducer: list the one concrete evidence artifact.
        if prompt.contains("List every required generated dataset") {
            return serde_json::json!({
                "status": "accepted",
                "summary": "Artifact inventory for TASK-TDL-001.",
                "evidence": [
                    { "kind": "inspection", "summary": "collected artifact inventory" }
                ],
                "artifacts": [{
                    "id": "gap-audit-evidence",
                    "path": CANARY_ARTIFACT_REL,
                    "description": "gap audit evidence artifact"
                }],
                "commands_run": [],
                "files_read": [],
                "files_changed": [],
                "task_coverage": [],
                "residual_gaps": [],
                "data": {
                    "artifact_paths": [CANARY_ARTIFACT_REL],
                    "items": [{
                        "artifact_path": CANARY_ARTIFACT_REL,
                        "canonical_task_ids": [CANARY_TASK_ID],
                        "kind": "evidence"
                    }]
                }
            })
            .to_string();
        }

        // Read-only discovery and any other reducer: generic accepted evidence.
        Self::accepted(
            "Read-only discovery summary.",
            serde_json::json!({ "items": [] }),
        )
    }
}

#[async_trait::async_trait]
impl LlmClient for CanaryAgentClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> CanaryResult<LlmResponse> {
        let mut prompt = String::new();
        for value in system.iter().chain(messages.iter()) {
            collect_text(value, &mut prompt);
        }
        let content = self.respond(&prompt);
        self.prompts
            .lock()
            .expect("prompt log lock")
            .push(prompt.chars().take(2000).collect());
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

