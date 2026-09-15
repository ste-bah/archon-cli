use super::*;
use crate::subagent_adapter::tests::{NoopClient, request};

fn client() -> SubagentPipelineClient {
    SubagentPipelineClient::new(Arc::new(NoopClient), ToolContext::default())
}

#[test]
fn continuation_retains_raw_history_identity_and_read_budget() {
    let client = client();
    let mut request = request(ToolAccessLevel::Full);
    request.pipeline_type = PipelineType::Workflow;
    let mut first = SessionLease::begin(&client, &request, false).unwrap();
    let messages = vec![
        serde_json::json!({"role":"assistant","content":[{"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"src/lib.rs"}}]}),
        serde_json::json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"read-1","content":"source bytes"}]}),
        serde_json::json!({"role":"assistant","content":"previous invalid answer"}),
    ];
    for message in &messages {
        first.history.append(message);
    }
    let id = first.id.clone();
    let guard = first.read_guard.clone().unwrap();
    first.complete().unwrap();
    drop(first);
    request.messages = vec![serde_json::json!({"role":"user","content":"repair feedback"})];
    let next = SessionLease::begin(&client, &request, true).unwrap();
    assert_eq!(next.id, id);
    assert_eq!(next.history.messages(), messages);
    assert!(Arc::ptr_eq(next.read_guard.as_ref().unwrap(), &guard));
}

#[test]
fn fresh_generation_never_reuses_history_and_parallel_agent_is_rejected() {
    let client = client();
    let request = request(ToolAccessLevel::ReadOnly);
    let mut first = SessionLease::begin(&client, &request, false).unwrap();
    assert!(SessionLease::begin(&client, &request, false).is_err());
    first
        .history
        .append(&serde_json::json!({"role":"assistant","content":"old"}));
    let old_id = first.id.clone();
    first.complete().unwrap();
    drop(first);
    let next = SessionLease::begin(&client, &request, false).unwrap();
    assert_ne!(next.id, old_id);
    assert!(next.history.messages().is_empty());
}

#[test]
fn missing_or_changed_identity_cannot_resume() {
    let client = client();
    let mut request = request(ToolAccessLevel::ReadOnly);
    assert!(SessionLease::begin(&client, &request, true).is_err());
    let mut first = SessionLease::begin(&client, &request, false).unwrap();
    first.complete().unwrap();
    drop(first);
    request.allowed_tools.push("Bash".into());
    assert!(SessionLease::begin(&client, &request, true).is_err());
    request.allowed_tools.clear();
    request.agent.key.push_str("-other-agent");
    assert!(SessionLease::begin(&client, &request, true).is_err());
}

/// Issue-21: a verifier's Bash runs in the canonical checkout too, so it gets
/// the shell admissions — but never the read budget a coder gets.
#[test]
fn bash_only_verification_gets_the_shell_guard_without_the_read_budget() {
    use archon_tools::workflow_read_guard::GuardMode;
    let client = client();
    let mut request = request(ToolAccessLevel::Full);
    request.pipeline_type = PipelineType::Workflow;
    request.allowed_tools = vec!["Read".into(), "Bash".into()];
    let verification = SessionLease::begin(&client, &request, false).unwrap();
    let guard = verification.read_guard.clone().expect("read-only workflow call gets a guard");
    assert_eq!(guard.mode(), GuardMode::ReadOnly);
    let refusal = guard
        .before_tool("Bash", &serde_json::json!({"command": "cargo build --release --bin archon"}))
        .expect("release build refused for the verifier");
    assert!(refusal.starts_with("Release builds are disabled for workflow calls."), "{refusal}");
    assert!(guard.before_tool("Bash", &serde_json::json!({"command": "git stash"})).is_some());
    assert!(guard.before_tool("Bash", &serde_json::json!({"command": "cargo fmt --all"})).is_some());
    for _ in 0..200 {
        assert!(guard.before_tool("Read", &serde_json::json!({"file_path": "src/lib.rs"})).is_none());
    }
    drop(verification);
    request.allowed_tools.push("Edit".into());
    let writer = SessionLease::begin(&client, &request, false).unwrap();
    assert_eq!(writer.read_guard.as_ref().unwrap().mode(), GuardMode::WriteCapable);
}

#[test]
fn guard_is_workflow_only_and_needs_a_shell_or_a_write_tool() {
    let client = client();
    let mut request = request(ToolAccessLevel::Full);
    request.allowed_tools = vec!["Read".into(), "Bash".into(), "Edit".into()];
    for pipeline in [PipelineType::Coding, PipelineType::Workflow] {
        request.pipeline_type = pipeline.clone();
        let lease = SessionLease::begin(&client, &request, false).unwrap();
        assert_eq!(lease.read_guard.is_some(), pipeline == PipelineType::Workflow, "{pipeline:?}");
    }
    request.allowed_tools = vec!["Read".into(), "Grep".into(), "Glob".into()];
    let inspect_only = SessionLease::begin(&client, &request, false).unwrap();
    assert!(inspect_only.read_guard.is_none(), "nothing to admit without Bash or a write tool");
}

#[tokio::test]
async fn audit_tool_contract_lists_only_host_granted_landing_tool() {
    struct Host;
    impl archon_tools::audit_landing::LandingHost for Host {
        fn land(&self,_:serde_json::Value)->Result<String,String>{Ok(String::new())}
        fn hint(&self)->Result<String,String>{Ok(String::new())}
        fn complete(&self,_:&serde_json::Value)->Result<(),String>{Ok(())}
    }
    let mut request=request(ToolAccessLevel::ReadOnly);
    request.allowed_tools=vec!["Read".into(),"Grep".into(),"Glob".into()];
    let landing=Arc::new(archon_tools::audit_landing::AuditLanding::new(Arc::new(Host),None));
    archon_tools::audit_landing::scope(landing,async {
        assert!(SubagentPipelineClient::prompt_for_request(&request).prompt.contains("Glob, land-audit-record"));
        assert!(!SubagentPipelineClient::allowed_tools(&request).contains(&"Bash".into()));
    }).await;
    assert!(!SubagentPipelineClient::allowed_tools(&request).contains(&"land-audit-record".into()));
}
