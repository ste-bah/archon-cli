use super::*;

#[tokio::test]
async fn corpus_review_verify_wrapped_receipts_use_the_envelope_normalizer() {
    for kind in [RecordKind::Review, RecordKind::Verify] {
        for text in [
            "```json\n{\"status\":\"failed\",\"data\":{\"records_landed\":1}}\n```",
            "Reviewed tasks [A, B]. {\"status\":\"failed\",\"data\":{\"records_landed\":1}} Done.",
            "{\"status\":\"failed\",\"data\":{\"records_landed\":1,},}",
        ] {
            let run = replay(
                vec![events(&[json!({"type":"text","text":text})], "end_turn")],
                0,
            );
            let options = if kind == RecordKind::Review {
                WorkflowV2HostOptions {
                    extra: [("reviewContract".into(), json!({"stage":"map"}))].into(),
                    ..Default::default()
                }
            } else {
                WorkflowV2HostOptions {
                    item_kind: Some("focused_verification".into()),
                    ..Default::default()
                }
            };
            let execution = WorkflowV2CallExecution {
                call: WorkflowV2HostCall {
                    id: "verdict".into(),
                    method: WorkflowV2HostMethod::Agent,
                    write_mode: None,
                    options,
                },
                input: json!({"canonical_task_ids":["UNIT-1"]}),
                depends_on: vec![],
            };
            let mut request = archon_workflow::v2::call_data::v2_agent_request(
                "Return a verdict",
                None,
                &execution,
                None,
            );
            let store = WorkflowV2ResultStore::new(run._temp.path().join("verdict"));
            let records = prepare(&mut request, Some(&store)).unwrap().unwrap();
            assert_eq!(records.kind(), kind);
            records
                .land(json!({"subject":"UNIT-1","status":"failed",
                "evidence":[{"kind":"inspection","summary":"inspected source"}]}))
                .unwrap();
            let landing = Arc::new(AuditLanding::new(Arc::new(Bridge(records.clone())), None));
            let root = run._temp.path().to_path_buf();
            let registry = Arc::new(archon_core::dispatch::create_default_registry(
                root.clone(),
                None,
            ));
            let runner = SubagentRunner::new(
                run.provider.clone(),
                "Return a verdict".into(),
                registry.tool_definitions(),
                registry,
                ToolContext {
                    working_dir: root,
                    session_id: uuid::Uuid::new_v4().to_string(),
                    audit_landing: Some(landing),
                    ..Default::default()
                },
                "corpus".into(),
                4,
                10,
                Arc::new(AgentConfig::default()),
                Arc::new(IdentityProvider::new(
                    IdentityMode::Clean,
                    "test".into(),
                    String::new(),
                    String::new(),
                )),
            );
            let answer = runner.run("Return the verdict receipt").await.unwrap();
            let value: Value = serde_json::from_str(&answer).unwrap();
            assert_eq!(value["status"], "failed");
            assert_eq!(value["data"]["records_landed"], 1);
            let composed: Value =
                serde_json::from_str(&compose_raw_reply(&records, &answer).unwrap()).unwrap();
            assert!(composed["data"].get("records_landed").is_none());
            assert!(composed["data"]["findings"].is_array());
            assert_eq!(run.provider.requests.lock().unwrap().len(), 1);
        }
    }
}
