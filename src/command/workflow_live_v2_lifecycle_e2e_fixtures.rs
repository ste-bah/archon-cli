fn synthetic_task_universe(root: &std::path::Path) -> WorkflowV2TaskUniverse {
    let task = |id: &str, criterion: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: root
            .join("tasks")
            .join(format!("{id}.md"))
            .display()
            .to_string(),
        acceptance_criteria: vec![criterion.to_string()],
        ..Default::default()
    };
    let mut contract_task = task("TASK-EX-004", "Declared artifact verification passes.");
    contract_task.artifact_requirements =
        vec![".archon/artifacts/example-contract.json".to_string()];
    contract_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "example-record".to_string(),
        artifact_path: ".archon/artifacts/example-contract.json".to_string(),
        required_universe: false,
        ..Default::default()
    }];
    let mut artifact_only_task = task("TASK-EX-005", "Artifact-only output is produced.");
    artifact_only_task.artifact_requirements =
        vec![".archon/artifacts/artifact-only.json".to_string()];
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec![root.join("tasks").display().to_string()],
        tasks: vec![
            task("TASK-EX-001", "Existing evidence is sufficient."),
            task("TASK-EX-002", "Refuted work is implemented."),
            task("TASK-EX-003", "Plain implementation is present."),
            contract_task,
            artifact_only_task,
        ],
    }
}

fn synthetic_inventory_items() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "item_id": "noop-legit",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-001"],
            "dependency_ids": [],
            "acceptance_criteria": ["Existing evidence is sufficient."],
            "noop_proof": "existing neutral fixture evidence",
            "noop_proof_refs": ["fixture:existing-evidence"],
            "artifact_requirements": [],
        }),
        serde_json::json!({
            "item_id": "noop-refutable",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-002"],
            "dependency_ids": [],
            "acceptance_criteria": ["Refuted work is implemented."],
            "noop_proof": "unsupported inherited claim",
            "noop_proof_refs": ["fixture:missing-evidence"],
            "artifact_requirements": [],
        }),
        serde_json::json!({
            "item_id": "noop-artifact-only",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-005"],
            "dependency_ids": [],
            "acceptance_criteria": ["Artifact-only output is produced."],
            "noop_proof": "unsupported inherited artifact claim",
            "noop_proof_refs": ["fixture:missing-artifact"],
            "artifact_requirements": [],
        }),
        implementation_item(
            "implementation-plain",
            "TASK-EX-003",
            "src/plain.rs",
            "Plain implementation is present.",
        ),
        implementation_item(
            "implementation-contract",
            "TASK-EX-004",
            "src/contract.rs",
            "Declared artifact verification passes.",
        ),
    ]
}

fn implementation_item(
    item_id: &str,
    task_id: &str,
    target_file: &str,
    criterion: &str,
) -> serde_json::Value {
    serde_json::json!({
        "item_id": item_id,
        "work_type": "implementation",
        "canonical_task_ids": [task_id],
        "dependency_ids": [],
        "target_files": [target_file],
        "acceptance_criteria": [criterion],
        "focused_verification": format!("test -f {target_file}"),
        "artifact_requirements": if task_id == "TASK-EX-004" {
            serde_json::json!([".archon/artifacts/example-contract.json"])
        } else {
            serde_json::json!([])
        },
    })
}

fn verification_item(item_id: &str, task_id: &str, target_file: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": item_id,
        "source_item_id": item_id.replace("verify-", "implementation-"),
        "canonical_task_ids": [task_id],
        "focused_verification": format!("test -f {target_file}"),
        "expected_evidence": format!("{target_file} exists"),
        "artifact_requirements": [],
    })
}

fn noop_proof_result(call_id: &str) -> serde_json::Value {
    if call_id.ends_with("noop-legit") {
        serde_json::json!({
            "status": "noop",
            "summary": "authoritative noop evidence exists",
            "evidence": [{"kind": "inspection", "summary": "fixture evidence checked"}],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [{
                "task_id": "TASK-EX-001",
                "status": "noop",
                "summary": "existing evidence satisfies the criterion",
                "evidence": [{"kind": "inspection", "summary": "fixture:existing-evidence"}],
            }],
            "residual_gaps": [],
            "data": {
                "item_id": "noop-legit",
                "canonical_task_ids": ["TASK-EX-001"],
                "acceptance_criteria_results": [{
                    "task_id": "TASK-EX-001",
                    "criterion": "Existing evidence is sufficient.",
                    "status": "passed",
                    "evidence_refs": ["fixture:existing-evidence"],
                }],
            },
        })
    } else if call_id.contains("artifact-only") {
        serde_json::json!({
            "status": "needs_review",
            "summary": "artifact-only noop claim is refuted",
            "evidence": [{"kind": "inspection", "summary": "required artifact is absent"}],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [{
                "task_id": "TASK-EX-005",
                "status": "missing",
                "summary": "artifact is absent",
                "evidence": [],
            }],
            "residual_gaps": [{
                "id": "gap-refuted-artifact-noop",
                "description": "the declared artifact does not exist",
                "severity": "blocking",
            }],
            "data": {
                "item_id": call_id,
                "canonical_task_ids": ["TASK-EX-005"],
                "proof_gap": true,
            },
        })
    } else {
        serde_json::json!({
            "status": "needs_review",
            "summary": "noop claim is refuted",
            "evidence": [{"kind": "inspection", "summary": "required evidence is absent"}],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [{
                "task_id": "TASK-EX-002",
                "status": "missing",
                "summary": "implementation evidence is absent",
                "evidence": [],
            }],
            "residual_gaps": [{
                "id": "gap-refuted-noop",
                "description": "the exact acceptance criterion is not satisfied",
                "severity": "blocking",
            }],
            "data": {
                "item_id": call_id,
                "canonical_task_ids": ["TASK-EX-002"],
                "proof_gap": true,
            },
        })
    }
}

fn implementation_result(
    request: &AgentExecutionRequest,
    input: &serde_json::Value,
    call_id: &str,
) -> Result<serde_json::Value> {
    let item = find_item(input).ok_or_else(|| anyhow::anyhow!("implementation item missing"))?;
    let task_id = first_string(item.get("canonical_task_ids"))
        .ok_or_else(|| anyhow::anyhow!("canonical task id missing"))?;
    let target_file = first_string(item.get("target_files"));
    let cwd = request
        .cwd
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("cwd missing"))?;
    if let Some(target_file) = target_file.as_deref() {
        let target = cwd.join(target_file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            &target,
            format!(
                "pub fn implemented_{}() -> bool {{ true }}\n",
                task_id.replace('-', "_")
            ),
        )?;
    } else if task_id == "TASK-EX-005" {
        let project_root = find_string_key(input, "project_artifact_root")
            .or_else(|| find_string_key(input, "project_root"))
            .ok_or_else(|| anyhow::anyhow!("project artifact root missing"))?;
        std::fs::write(
            std::path::Path::new(&project_root).join(".archon/artifacts/artifact-only.json"),
            "{\"status\":\"produced\"}\n",
        )?;
    } else {
        anyhow::bail!("target file missing")
    }
    let mut result = accepted_result(
        "implementation branch changed its declared target",
        serde_json::json!({
            "item_id": item
                .get("item_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(call_id),
            "canonical_task_ids": [task_id],
        }),
        vec![coverage(&task_id, "accepted")],
        vec![test_command("true", true, "implementation fixture passed")],
    );
    if let Some(target_file) = target_file {
        result = result.with_files_changed(vec![target_file]);
    }
    if task_id == "TASK-EX-004" {
        let project_root = find_string_key(input, "project_artifact_root")
            .or_else(|| find_string_key(input, "project_root"))
            .ok_or_else(|| anyhow::anyhow!("project artifact root missing"))?;
        result["artifacts"] = serde_json::json!([{
            "id": "example-contract",
            "path": std::path::Path::new(&project_root)
                .join(".archon/artifacts/example-contract.json")
                .display()
                .to_string(),
            "description": "pre-existing declared contract fixture",
        }]);
    }
    Ok(result)
}

fn verification_result(
    request: &AgentExecutionRequest,
    input: &serde_json::Value,
    call_id: &str,
    deliverable_contract_executed: &AtomicBool,
) -> Result<serde_json::Value> {
    let item = find_item(input).ok_or_else(|| anyhow::anyhow!("verification item missing"))?;
    let task_id = first_string(item.get("canonical_task_ids"))
        .ok_or_else(|| anyhow::anyhow!("verification task id missing"))?;
    let command = item
        .get("focused_verification")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("true");
    let cwd = request
        .cwd
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("cwd missing"))?;
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()?;
    if item.get("deliverable_contract").is_some() {
        deliverable_contract_executed.store(true, Ordering::SeqCst);
    }
    let succeeded = output.status.success();
    let summary = if succeeded {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    };
    let mut result = accepted_result(
        "focused verification executed",
        serde_json::json!({
            "item_id": item
                .get("item_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(call_id),
            "source_item_id": item.get("source_item_id"),
            "canonical_task_ids": [task_id],
            "focused_verification": command,
            "pass_fail_count": {
                "intended_target_passed": usize::from(succeeded),
                "intended_target_failed": usize::from(!succeeded),
            },
            "matched_test_check_names": {
                "passed": if succeeded { vec![call_id] } else { Vec::new() },
                "failed": if succeeded { Vec::new() } else { vec![call_id] },
            },
        }),
        vec![coverage(
            &task_id,
            if succeeded { "accepted" } else { "blocked" },
        )],
        vec![test_command(command, succeeded, &summary)],
    );
    if !succeeded {
        result["status"] = serde_json::json!("needs_review");
        result["residual_gaps"] = serde_json::json!([{
            "id": format!("verification-failed-{call_id}"),
            "description": summary,
            "severity": "blocking",
        }]);
    }
    Ok(result)
}

fn accepted_result(
    summary: &str,
    data: serde_json::Value,
    task_coverage: Vec<serde_json::Value>,
    commands_run: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "accepted",
        "summary": summary,
        "evidence": [{"kind": "inspection", "summary": summary}],
        "artifacts": [],
        "commands_run": commands_run,
        "files_read": [],
        "files_changed": [],
        "task_coverage": task_coverage,
        "residual_gaps": [],
        "data": data,
    })
}

fn needs_review_result(summary: &str, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "status": "needs_review",
        "summary": summary,
        "evidence": [{"kind": "inspection", "summary": summary}],
        "artifacts": [],
        "commands_run": [],
        "files_read": [],
        "files_changed": [],
        "task_coverage": [],
        "residual_gaps": [{
            "id": "synthetic-inventory-shape",
            "description": summary,
            "severity": "blocking",
        }],
        "data": data,
    })
}

trait ResultValueExt {
    fn with_files_changed(self, files: Vec<String>) -> serde_json::Value;
}

impl ResultValueExt for serde_json::Value {
    fn with_files_changed(mut self, files: Vec<String>) -> serde_json::Value {
        self["files_changed"] = serde_json::Value::Array(
            files
                .into_iter()
                .map(|path| serde_json::json!({"path": path, "purpose": "declared target edit"}))
                .collect(),
        );
        self["evidence"] = serde_json::json!([{
            "kind": "implementation",
            "summary": "declared target changed in isolated worktree",
        }]);
        self
    }
}

fn coverage(task_id: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "status": status,
        "summary": format!("{task_id} {status}"),
        "evidence": [{"kind": "test", "summary": format!("{task_id} evidence")}],
    })
}

fn all_task_coverage() -> Vec<serde_json::Value> {
    vec![
        coverage("TASK-EX-001", "noop"),
        coverage("TASK-EX-002", "accepted"),
        coverage("TASK-EX-003", "accepted"),
        coverage("TASK-EX-004", "accepted"),
        coverage("TASK-EX-005", "accepted"),
    ]
}

fn test_command(command: &str, succeeded: bool, summary: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "test",
        "command": command,
        "status": if succeeded { "succeeded" } else { "failed" },
        "exit_code": if succeeded { 0 } else { 1 },
        "output_summary": if summary.is_empty() { "command completed" } else { summary },
    })
}

include!("workflow_live_v2_lifecycle_e2e_fixture_utils.rs");
