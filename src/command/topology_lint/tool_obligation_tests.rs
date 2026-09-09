use super::*;
use archon_core::config::GateMode;

fn fixture(kind: &str, tools: &str) -> (tempfile::TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers":{"sample":{"command":"unused","toolPolicy":{"toolPermissions":{"fetch_records":"safe","inspect_state":"safe","erase_records":"dangerous"}}}}}"#).unwrap();
    let tasks = dir.path().join("tasks/PRD-EXAMPLE");
    std::fs::create_dir_all(&tasks).unwrap();
    let path = tasks.join("TASK-EXAMPLE-001.md");
    let raw = format!("```yaml\ntask_id: TASK-EXAMPLE-001\ntitle: Adapter\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: {tools}\ndeliverable_contracts: [{{kind: {kind}, artifact_path: adapter.txt, min_instances: 1, typed_verifier_command: 'grep -q required {{artifact_path}}'}}]\n```\n\n## Focused Tests\n- `grep -q required adapter.txt`\n");
    std::fs::write(&path, &raw).unwrap();
    (dir, path, raw)
}

#[test]
fn mcp_obligation_body_candidate_and_set_gate_reject_missing_declarations() {
    let (dir, path, raw) = fixture("sample-mcp-native-ingest", "[]");
    for mode in [GateMode::Observe, GateMode::Enforce] {
        let candidate = evaluate_task_file_candidate(dir.path(), &path, raw.as_bytes(), mode).unwrap();
        assert!(candidate.findings.iter().any(|f| f.text.contains("required_tools") && f.remediation_scope == archon_workflow::RemediationScope::Body), "{:?}", candidate.findings);
        let set = evaluate_lint(dir.path(), &LintSource::Tasks(path.parent().unwrap().into()), mode).unwrap();
        assert!(set.findings.iter().any(|f| f.text.contains("required_tools") && f.remediation_scope == archon_workflow::RemediationScope::Body), "{:?}", set.findings);
    }
}

#[test]
fn mcp_obligation_wrong_server_or_ambient_runner_cannot_satisfy_contract() {
    for tools in ["[Bash]", "[mcp__other__fetch_records]", "[mcp__sample__erase_records]"] {
        let (dir, path, raw) = fixture("sample-mcp-native-ingest", tools);
        let gate = evaluate_task_file_candidate(dir.path(), &path, raw.as_bytes(), GateMode::Enforce).unwrap();
        assert!(gate.findings.iter().any(|f| f.text.contains("required_tools")), "{tools}: {:?}", gate.findings);
    }
}

#[test]
fn mcp_obligation_http_and_non_tool_contracts_do_not_require_mcp() {
    for kind in ["http-native-ingest-or-unavailable", "source-library"] {
        let (dir, path, raw) = fixture(kind, "[]");
        let gate = evaluate_task_file_candidate(dir.path(), &path, raw.as_bytes(), GateMode::Enforce).unwrap();
        assert!(!gate.findings.iter().any(|f| f.text.contains("required_tools")), "{:?}", gate.findings);
    }
}

#[test]
fn mcp_obligation_repaired_task_reaches_actual_stage_allowlist() {
    let (dir, path, raw) = fixture("sample-mcp-native-ingest", "[mcp__sample__fetch_records]");
    let gate = evaluate_task_file_candidate(dir.path(), &path, raw.as_bytes(), GateMode::Enforce).unwrap();
    assert!(!gate.findings.iter().any(|f| f.text.contains("required_tools")), "{:?}", gate.findings);
    let task = archon_workflow::task_universe::parsing::parse_task_file(&path, &raw).unwrap();
    let universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {schema_version: "v1".into(), source_roots: vec![], tasks: vec![task]};
    let item = archon_workflow::generated_contract::normalize_generated_item_value(&serde_json::json!({"item_id":"item", "canonical_task_ids":["TASK-EXAMPLE-001"], "work_type":"implementation", "target_files":["adapter.txt"]}), Some(&universe)).value;
    let request = archon_workflow::StageRunRequest {run_id:"test".into(), stage_id:"write".into(), stage_kind:archon_workflow::StageKind::Implementation, agent:None, task:"Implement".into(), attempt:1, provider_tier:archon_workflow::ProviderTier::Coder, depends_on:vec![], input:serde_json::json!({"project_artifact_root":dir.path(), "item":item})};
    let tools = crate::command::workflow_live::workflow_live_runner::allowed_tools(&request);
    assert!(tools.contains(&"mcp__sample__fetch_records".into()), "{tools:?}");
    assert!(!tools.contains(&"mcp__sample__inspect_state".into()));
    assert!(!tools.contains(&"mcp__sample__erase_records".into()));
}

#[test]
fn mcp_obligation_exact_focused_call_must_be_declared_even_with_another_grant() {
    let (dir, path, raw) = fixture("sample-mcp-native-ingest", "[mcp__sample__inspect_state]");
    let raw = format!("{raw}\n- `mcp__sample__fetch_records` must be exercised.\n");
    let gate = evaluate_task_file_candidate(dir.path(), &path, raw.as_bytes(), GateMode::Enforce).unwrap();
    assert!(gate.findings.iter().any(|f| f.text.contains("fetch_records") && f.text.contains("required_tools")), "{:?}", gate.findings);
}
