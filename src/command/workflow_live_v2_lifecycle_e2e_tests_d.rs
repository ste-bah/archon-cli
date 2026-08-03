use super::*;

pub(super) fn noop_proof_result(call_id: &str) -> serde_json::Value {
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

pub(super) fn implementation_result(
    request: &WorkflowAgentCall,
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
    let reported_task_id = if task_id == "TASK-EX-003" {
        "EX-003"
    } else {
        task_id.as_str()
    };
    let mut result = accepted_result(
        "implementation branch changed its declared target",
        serde_json::json!({
            "item_id": item
                .get("item_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(call_id),
            "canonical_task_ids": [reported_task_id],
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

pub(super) fn verification_result(
    request: &WorkflowAgentCall,
    input: &serde_json::Value,
    call_id: &str,
    deliverable_contract_executed: &AtomicBool,
    parameterized_contract_executed: &AtomicBool,
    verification_failure_emitted: &AtomicBool,
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
    // Fed on stdin rather than as `-c <command>`, matching the production
    // verification branch. A declared deliverable contract expands into the
    // ~29 KB generated verifier plus its embedded contract JSON, and Windows
    // caps a command line at 32,767 characters -- CreateProcess truncates past
    // that without erroring, so the shell receives a half-written script and
    // exits non-zero on a syntax error that looks like a failed verification.
    let output = run_shell_script(cwd, command)?;
    if item.get("deliverable_contract").is_some() {
        deliverable_contract_executed.store(true, Ordering::SeqCst);
    }
    if item
        .get("deliverable_contract")
        .and_then(|contract| contract.get("kind"))
        .and_then(serde_json::Value::as_str)
        == Some("instance_report")
    {
        parameterized_contract_executed.store(true, Ordering::SeqCst);
    }
    let forced_failure = task_id == "TASK-EX-003"
        && !call_id.contains("post-remediation")
        && !verification_failure_emitted.swap(true, Ordering::SeqCst);
    let succeeded = output.status.success() && !forced_failure;
    let summary = if succeeded {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        if forced_failure {
            "synthetic first-attempt verification failure".to_string()
        } else {
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        }
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

fn run_shell_script(cwd: &std::path::Path, script: &str) -> Result<std::process::Output> {
    use std::io::Write;

    let mut child = Command::new(archon_shell::resolve_posix_shell())
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes())?;
    }
    Ok(child.wait_with_output()?)
}

pub(super) fn accepted_result(
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

pub(super) fn needs_review_result(summary: &str, data: serde_json::Value) -> serde_json::Value {
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

pub(super) fn all_task_coverage() -> Vec<serde_json::Value> {
    vec![
        coverage("TASK-EX-001", "noop"),
        coverage("TASK-EX-002", "accepted"),
        coverage("TASK-EX-003", "accepted"),
        coverage("TASK-EX-004", "accepted"),
        coverage("TASK-EX-005", "accepted"),
        coverage("TASK-EX-006", "accepted"),
    ]
}

pub(super) fn test_command(command: &str, succeeded: bool, summary: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "test",
        "command": command,
        "status": if succeeded { "succeeded" } else { "failed" },
        "exit_code": if succeeded { 0 } else { 1 },
        "output_summary": if summary.is_empty() { "command completed" } else { summary },
    })
}

pub(super) fn prompt_line(prompt: &str, prefix: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix).map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn prompt_input(prompt: &str) -> serde_json::Value {
    let Some(after) = prompt.split("## Input\n```json\n").nth(1) else {
        return serde_json::Value::Null;
    };
    let Some(raw) = after.split("\n```").next() else {
        return serde_json::Value::Null;
    };
    serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
}

fn find_item(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("canonical_task_ids")
                && (object.contains_key("target_files")
                    || object.contains_key("focused_verification"))
            {
                return Some(value);
            }
            object.values().find_map(find_item)
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_item),
        _ => None,
    }
}

fn first_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn find_string_key(value: &serde_json::Value, target: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => object
            .get(target)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_string_key(value, target))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_string_key(value, target)),
        _ => None,
    }
}

pub(super) fn init_git_repo(repo: &std::path::Path) {
    run_git(repo, &["init"]);
    // Line endings pinned: Git for Windows defaults to core.autocrlf=true, so a
    // file committed with LF is checked back out with CRLF, the lifecycle sees
    // every seeded source as modified, and the run latches into verification
    // remediation instead of reaching a terminal state.
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);
    run_git(repo, &["config", "user.name", "archon-test"]);
    run_git(
        repo,
        &["config", "user.email", "archon-test@example.invalid"],
    );
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
