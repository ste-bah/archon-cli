use super::raw_evidence::raw_outcome_result;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use serde_json::{Value, json};

async fn persisted_usage_from_stream(
    call_id: &str,
    events: Vec<archon_llm::streaming::StreamEvent>,
) -> Value {
    let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
    for event in events {
        tx.send(event).await.unwrap();
    }
    drop(tx);
    let response = archon_pipeline::llm_adapter::collect_stream(rx)
        .await
        .expect("real pipeline stream accumulator");
    let result = raw_outcome_result(
        WorkflowAgentOutcome {
            content: response.content,
            tokens_in: response.tokens_in,
            tokens_out: response.tokens_out,
            stop_reason: response.stop_reason,
            ..Default::default()
        },
        "end_turn".into(),
    );
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowV2ResultStore::new(temp.path().join("v2"));
    let record = archon_workflow::WorkflowV2CallRecord::new(
        "run-stream-usage",
        archon_workflow::WorkflowV2HostCall {
            id: call_id.into(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "stream-usage-input".into(),
        result,
        Vec::new(),
    );
    store.save_call_record(&record).unwrap();
    let read_back = store.load_call_record(call_id).unwrap().unwrap();
    serde_json::to_value(read_back.result).unwrap()["data"].clone()
}

#[tokio::test]
async fn stream_usage_availability_survives_raw_outcome_persistence() {
    fn parse_sse(input: &str) -> Vec<archon_llm::streaming::StreamEvent> {
        archon_llm::streaming::split_sse_lines(input)
            .into_iter()
            .map(|(event, data)| archon_llm::streaming::parse_sse_event(event, data).unwrap())
            .collect()
    }

    let unknown = persisted_usage_from_stream(
            "unknown-stream-usage",
            parse_sse(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-unknown\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"model\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ),
        )
        .await;
    assert!(unknown["tokensIn"].is_null());
    assert_eq!(
        unknown["tokensInUnknownReason"],
        json!("provider did not report a confirmed input token count")
    );
    assert!(unknown["tokensOut"].is_null());
    assert_eq!(
        unknown["tokensOutUnknownReason"],
        json!("provider did not report an output token count")
    );

    let reported_zero = persisted_usage_from_stream(
            "reported-zero-stream-usage",
            parse_sse(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-zero\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"model\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ),
        )
        .await;
    assert_eq!(reported_zero["tokensIn"], json!(0));
    assert!(reported_zero["tokensInUnknownReason"].is_null());
    assert_eq!(reported_zero["tokensOut"], json!(0));
    assert!(reported_zero["tokensOutUnknownReason"].is_null());
    println!(
        "EVIDENCE #369 stream-to-raw-record unknown-input BEFORE: zero-only message_start; AFTER separate result-store read: {unknown}"
    );
    println!(
        "EVIDENCE #369 stream-to-raw-record reported-zero BEFORE: available zero message_delta; AFTER separate result-store read: {reported_zero}"
    );
}
