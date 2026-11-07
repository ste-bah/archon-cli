use super::super::*;
use super::*;
struct Port;
#[async_trait::async_trait]
impl WorkflowLlmClient for Port {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(
        &self,
        _: archon_workflow::WorkflowAgentCall,
    ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let records = archon_workflow::v2::record_landing::current()
            .expect("actual dispatch installed record scope");
        assert_eq!(
            archon_tools::audit_landing::current().unwrap().tool_name(),
            "land-review-record"
        );
        records.land(json!({"subject":"UNIT-1","findings":[{"id":"F1","claim":"counterexample"}],"evidence":[{"kind":"inspection","summary":"inspected source"}]}))?;
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: json!({"status":"accepted","data":{"records_landed":1}}).to_string(),
            tool_uses: vec![],
            tokens_in: Some(0),
            tokens_out: Some(0),
            stop_reason: Some("end_turn".into()),
        })
    }
}
#[tokio::test]
async fn typed_landing_actual_dispatch_reconstructs_review_without_changing_scope() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(Port),
        sink,
        vec![],
        "fixture".into(),
        None,
        Some(30),
    );
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "review".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions {
                extra: [("reviewContract".into(), json!({"stage":"map"}))].into(),
                ..Default::default()
            },
        },
        input: json!({"canonical_task_ids":["UNIT-1"]}),
        depends_on: vec![],
    };
    let request =
        archon_workflow::v2::call_data::v2_agent_request("review", None, &execution, None);
    let result = run_v2_agent_call_with_rejected_output_log(
        &WorkflowV2AgentAdapter::new(),
        &client,
        &request,
        Some(&store),
    )
    .await
    .unwrap();
    assert_eq!(result.data["findings"][0]["id"], "F1");
    assert_eq!(
        result.data["findings"][0]["canonical_task_ids"],
        json!(["UNIT-1"])
    );
    assert!(archon_tools::audit_landing::current().is_none());
}
fn schema_for(kind: RecordKind) -> Value {
    use archon_tools::audit_landing::LandingHost;
    let temp = tempfile::tempdir().unwrap();
    Bridge(Arc::new(
        RecordLanding::open(temp.path().into(), "id".into(), kind, vec![], false).unwrap(),
    ))
    .schema()
    .unwrap()
}
#[test]
fn skeleton_tool_schema_spells_out_the_task_entry() {
    let schema = schema_for(RecordKind::Skeleton);
    let task = &schema["properties"]["task"];
    assert_eq!(task["required"], json!(["task_id", "file_name"]));
    for field in [
        "task_id",
        "file_name",
        "depends_on",
        "blocks",
        "implements",
        "deliverable_contracts",
    ] {
        assert!(
            task["properties"].get(field).is_some(),
            "{field} missing from {task}"
        );
    }
    assert_eq!(task["properties"]["implements"]["items"]["type"], "string");
    assert_eq!(
        task["properties"]["deliverable_contracts"]["items"]["required"],
        json!(["kind", "artifact_path"])
    );
    assert_eq!(
        task["properties"]["depends_on"]["items"]["properties"]["consumes"]["items"]["required"],
        json!(["artifact_path"])
    );
    assert!(
        task.get("additionalProperties").is_none(),
        "FrozenTask accepts unknown keys, so the schema must too"
    );
    assert!(
        schema["description"]
            .as_str()
            .unwrap()
            .contains("deliverable_contracts")
    );
}
// Issue-39: the schema the model sees is built per kind and states what `validate` enforces.
#[test]
fn tool_schema_required_lists_and_enums_follow_the_record_kind() {
    let names = archon_workflow::v2::record_landing::enum_names();
    for (kind, required) in [
        (
            RecordKind::Review,
            json!(["subject", "evidence", "summary"]),
        ),
        (
            RecordKind::Verify,
            json!(["subject", "evidence", "summary", "status"]),
        ),
        (
            RecordKind::Skeleton,
            json!(["subject", "evidence", "summary"]),
        ),
    ] {
        let schema = schema_for(kind);
        assert_eq!(schema["required"], required, "{kind:?}");
        assert_eq!(schema["additionalProperties"], json!(false));
        let evidence = &schema["properties"]["evidence"]["items"];
        assert_eq!(evidence["required"], json!(["kind", "summary"]), "{kind:?}");
        assert_eq!(
            evidence["properties"]["kind"]["enum"],
            json!(names.evidence_kinds),
            "{kind:?}"
        );
        assert!(
            !evidence["properties"]["kind"]["enum"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let command = &schema["properties"]["commands_run"]["items"];
        assert_eq!(
            command["required"],
            json!(["kind", "command", "status", "output_summary"]),
            "{kind:?}"
        );
        assert_eq!(
            command["properties"]["kind"]["enum"],
            json!(names.command_kinds)
        );
        assert_eq!(
            command["properties"]["status"]["enum"],
            json!(names.command_statuses)
        );
        assert_eq!(
            schema["properties"]["status"]["enum"],
            json!(names.verify_verdicts),
            "{kind:?}"
        );
        assert_eq!(
            schema["properties"]["status"]["enum"],
            json!(["accepted", "noop", "failed", "blocked", "needs_review"])
        );
        for key in schema["required"].as_array().unwrap() {
            assert!(
                schema["properties"].get(key.as_str().unwrap()).is_some(),
                "{kind:?} requires {key} but does not describe it"
            );
        }
    }
    let (review, verify, skeleton) = (
        schema_for(RecordKind::Review),
        schema_for(RecordKind::Verify),
        schema_for(RecordKind::Skeleton),
    );
    assert!(
        review["properties"].get("task").is_none() && verify["properties"].get("task").is_none(),
        "task is a skeleton field"
    );
    assert!(
        skeleton["properties"].get("findings").is_none(),
        "findings are not skeleton fields"
    );
    let finding = &review["properties"]["findings"]["items"];
    assert_eq!(
        finding["anyOf"],
        json!([{"required":["claim"]},{"required":["summary"]},{"required":["finding"]},{"required":["title"]}])
    );
    assert_eq!(
        verify["properties"]["findings"]["items"]["anyOf"],
        finding["anyOf"]
    );
    assert!(
        verify["properties"]["status"]["description"]
            .as_str()
            .unwrap()
            .contains("succeeded commands_run entry")
    );
    assert_eq!(
        skeleton["properties"]["task"]["properties"]["deliverable_contracts"]["items"]["properties"]
            ["artifact_path"]["minLength"],
        json!(1)
    );
}

// Every author class uses the same completion rule, through preparation and raw composition.
fn assert_author_domain(class: &str, seeded: bool, findings: bool, directive: bool) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let task = json!({"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"]});
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: class.into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions {
                extra: [
                    ("recordLanding".into(), json!("skeleton")),
                    (
                        "openFindings".into(),
                        if findings {
                            json!(["repair task"])
                        } else {
                            json!([])
                        },
                    ),
                    (
                        "previousSkeleton".into(),
                        if seeded {
                            json!({"schema_version":1,"acceptance_digest":"","tasks":[task]})
                        } else {
                            Value::Null
                        },
                    ),
                ]
                .into(),
                ..Default::default()
            },
        },
        input: json!({}),
        depends_on: vec![],
    };
    let mut request =
        archon_workflow::v2::call_data::v2_agent_request(class, None, &execution, None);
    if directive {
        request.task.push_str("\nThe frozen predecessor was repaired; rebind the skeleton while retaining every unaffected task entry.");
    }
    let records = prepare(&mut request, Some(&store)).unwrap().unwrap();
    assert_eq!(
        records
            .retained_completion()
            .unwrap()
            .map_or(0, |value| value["records_landed"].as_u64().unwrap()
                as usize),
        usize::from(seeded),
        "{class}: retained seed"
    );
    let zero = compose_raw_reply(&records, r#"{"records_landed":0}"#);
    assert_eq!(zero.is_ok(), seeded && !findings, "{class}: {zero:?}");
    if let Ok(artifact) = zero {
        assert_eq!(
            serde_json::from_str::<Value>(&artifact).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    } else if !seeded {
        assert!(
            zero.unwrap_err()
                .to_string()
                .contains("task store is empty")
        );
    }
    let mut changed = task;
    changed["source_sections"] = json!(["2"]);
    records.land(json!({"subject":"TASK-X-001","task":changed,"summary":"checked","evidence":[{"kind":"inspection","summary":"read"}]})).unwrap();
    let artifact: Value =
        serde_json::from_str(&compose_raw_reply(&records, r#"{"records_landed":1}"#).unwrap())
            .unwrap();
    assert_eq!(
        artifact["tasks"][0]["source_sections"],
        json!(["2"]),
        "{class}"
    );
}

#[test]
fn stage_landing_author_first_draft() {
    assert_author_domain("first_draft", false, false, false);
}

#[test]
fn stage_landing_author_fresh_retry() {
    assert_author_domain("fresh_retry", false, false, false);
}

#[test]
fn stage_landing_author_repair() {
    assert_author_domain("repair", true, true, false);
}

#[test]
fn stage_landing_author_rebind() {
    assert_author_domain("rebind", true, false, true);
}

#[test]
fn stage_landing_author_seeded() {
    assert_author_domain("seeded", true, false, false);
}

#[test]
fn stage_landing_author_rebind_with_findings() {
    assert_author_domain("rebind_with_findings", true, true, true);
}
