use archon_workflow::*;
use serde_json::{Value, json};
use std::sync::Mutex;

fn request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall { id: "audit-fixture".into(), method: WorkflowV2HostMethod::Agent,
            write_mode: None, options: WorkflowV2HostOptions { extra: [("repository_audit_contract".into(),
                json!({"schema_version":1,"snapshot":"sealed-1","declared_paths":["src/new.txt","src/new.txt"]}))].into(), ..Default::default() } },
        role: "critic".into(), task: "Audit the sealed source".into(), constraints: vec![],
        input: json!({}), repository_root: None, project_artifacts: Default::default(),
        target_files: vec![], target_ownership_scopes: vec![],
    }
}
fn envelope(records: Value) -> String {
    json!({"status":"accepted","summary":"repository audited","evidence":[{"kind":"inspection","summary":"read sealed source"}],
        "data":{"repository_audit":{"schema_version":1,"snapshot":"sealed-1","records":records}}}).to_string()
}
fn valid() -> Value {
    json!([{"declared_path":"src/new.txt","verdict":"exists_elsewhere","equivalents":["legacy/implementation.txt"],"required_action":"wire_or_migrate","reason":"Existing implementation has the same behavior."}])
}

#[test]
fn malformed_audit_record_is_not_accepted_by_envelope_parser() {
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(), &envelope(json!([{"nonsense":1}])))
        .unwrap_err();
    assert!(error.to_string().contains("declared_path"), "{error}");
}
#[test]
fn shared_declared_path_requires_one_complete_record() {
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(), &envelope(valid()))
        .unwrap();
}
#[test]
fn incomplete_duplicate_escaping_and_wrong_snapshot_records_are_rejected() {
    let adapter = WorkflowV2AgentAdapter::new();
    for records in [
        json!([]),
        json!([valid()[0], valid()[0]]),
        json!([{"declared_path":"../escape","verdict":"absent","equivalents":[],"required_action":"deliver","reason":"absent"}]),
        json!([{"declared_path":"src/new.txt","verdict":"exists_elsewhere","equivalents":["../escape"],"required_action":"wire_or_migrate","reason":"equivalent"}]),
        json!([{"declared_path":"src/new.txt","verdict":"exists_elsewhere","equivalents":[],"required_action":"none","reason":"equivalent"}]),
    ] {
        assert!(
            adapter
                .parse_agent_output(&request(), &envelope(records))
                .is_err()
        );
    }
    assert!(
        adapter
            .parse_agent_output(&request(), &envelope(valid()).replace("sealed-1", "other"))
            .is_err()
    );
}
struct Scripted(Mutex<Vec<String>>);
#[async_trait::async_trait]
impl WorkflowV2AgentClient for Scripted {
    async fn run_agent(&self, _: String) -> Result<String, WorkflowV2AgentError> {
        unreachable!("request-aware dispatch")
    }
    async fn run_agent_request(
        &self,
        _: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> Result<String, WorkflowV2AgentError> {
        let mut prompts = self.0.lock().unwrap();
        prompts.push(prompt);
        Ok(envelope(if prompts.len() == 1 {
            json!([{"nonsense":1}])
        } else {
            valid()
        }))
    }
}
#[tokio::test]
async fn malformed_audit_earns_existing_bounded_repair() {
    let client = Scripted(Mutex::new(vec![]));
    let result = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request())
        .await
        .unwrap();
    assert_eq!(result.data["repository_audit"]["records"], valid());
    let prompts = client.0.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("declared_path"));
}

#[tokio::test]
async fn filesystem_inconsistent_audit_earns_bounded_repair() {
    struct IncorrectThenCorrect(Mutex<usize>);
    #[async_trait::async_trait]
    impl WorkflowV2AgentClient for IncorrectThenCorrect {
        async fn run_agent(&self, _: String) -> Result<String, WorkflowV2AgentError> {
            let mut calls = self.0.lock().unwrap();
            *calls += 1;
            let records = if *calls == 1 {
                json!([{"declared_path":"src/new.txt","verdict":"exists_as_declared","equivalents":[],"required_action":"none","reason":"claimed existing"}])
            } else {
                json!([{"declared_path":"src/new.txt","verdict":"absent","equivalents":[],"required_action":"deliver","reason":"sealed file is absent"}])
            };
            Ok(envelope(records))
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let mut request = request();
    request.repository_root = Some(temp.path().display().to_string());
    let client = IncorrectThenCorrect(Mutex::new(0));
    let result = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request)
        .await
        .unwrap();
    assert_eq!(
        result.data["repository_audit"]["records"][0]["verdict"], "absent",
        "a typed but false filesystem claim bypassed the bounded repair boundary"
    );
    assert_eq!(*client.0.lock().unwrap(), 2);
}
