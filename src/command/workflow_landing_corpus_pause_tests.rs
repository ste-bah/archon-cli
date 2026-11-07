use super::*;

pub(super) fn corpus_config() -> AgentConfig {
    AgentConfig {
        subagent_stream_idle_timeout_secs: 1,
        ..Default::default()
    }
}

#[tokio::test(start_paused = true)]
async fn corpus_unanswered_streams_preserve_landings_and_resumable_pause() {
    for failure in [
        vec![],
        vec![StreamEvent::Error {
            error_type: "connection_reset".into(),
            message: "connection reset".into(),
        }],
    ] {
        let all = blocks("landings.json");
        let mut rounds = vec![events(&all[17..], "tool_use")];
        rounds.extend(vec![failure; 100]);
        let run = replay(rounds, 17);
        // Exercise the runner's Unanswered resend path directly; the outer
        // recovery scope routes operational HTTP faults to its own retry layer.
        let error = run.runner.run("Process records").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
            "{error}"
        );
        assert_eq!(run.records.new_landing_count().unwrap(), 2);
        let artifact = run.records.retained_artifact().unwrap().unwrap();
        let expected: Vec<_> = all.iter().map(|b| b["input"]["task"].clone()).collect();
        assert_tasks(&run.records, &artifact, &expected);
        assert!(run.provider.requests.lock().unwrap().len() > 4);
    }
}
