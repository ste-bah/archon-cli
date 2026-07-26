use std::time::Duration;

use archon_pipeline::audit::types::BundleStatus;
use archon_pipeline::audit::{PipelineAuditRun, PipelineEvent, PipelineEventLine};
use archon_pipeline::audit::{PipelineBundleStore, rewind_bundle, verify_bundle};
use archon_pipeline::runner::{AgentInfo, AgentResult, PipelineType, ToolAccessLevel};

fn agent(key: &str) -> AgentInfo {
    AgentInfo {
        key: key.to_string(),
        display_name: key.to_string(),
        model: "sonnet".to_string(),
        phase: 1,
        critical: true,
        parallelizable: false,
        quality_threshold: 0.5,
        tool_access_level: ToolAccessLevel::ReadOnly,
    }
}

fn result(output: &str) -> AgentResult {
    AgentResult {
        output: output.to_string(),
        tool_use_log: Vec::new(),
        tokens_in: 10,
        tokens_out: 20,
        cost_usd: 0.01,
        duration: Duration::from_millis(1),
        quality: None,
    }
}

#[test]
fn rewind_quarantines_stale_agents_and_keeps_bundle_verifiable() {
    let temp = tempfile::tempdir().unwrap();
    let store = PipelineBundleStore::new(temp.path());
    store
        .create("session-1", PipelineType::Research, "research")
        .unwrap();
    let mut audit =
        archon_pipeline::audit::PipelineAuditRun::resume(temp.path(), "session-1").unwrap();

    for (ordinal, key) in ["a", "b", "c"].iter().enumerate() {
        let agent = agent(key);
        let result = result(&format!("output {key}"));
        let prompt = audit.record_prompt(ordinal, &agent, &[], &[], &[]).unwrap();
        audit
            .record_agent_completed(ordinal, &agent, &result, Vec::new(), prompt)
            .unwrap();
    }
    audit.fail("bad downstream output").unwrap();
    let export_dir = store.bundle_dir("session-1").join("exports");
    std::fs::create_dir_all(&export_dir).unwrap();
    std::fs::write(export_dir.join("final-paper.md"), "stale final paper").unwrap();

    let report = rewind_bundle(&store, "session-1", 2, "bad downstream output").unwrap();
    assert_eq!(report.from_completed_agent_count, 3);
    assert_eq!(report.to_completed_agent_count, 2);
    assert_eq!(report.quarantined_agent_records, 1);

    let state = store.load_state("session-1").unwrap();
    assert_eq!(state.completed_agent_count, 2);
    assert_eq!(state.total_tokens_in, 20);
    assert_eq!(state.total_tokens_out, 40);
    assert!(state.final_output_hash.is_none());
    assert!(state.last_error.unwrap().contains("bad downstream output"));

    let agents = store.list_agent_records("session-1").unwrap();
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().all(|record| record.ordinal < 2));

    let bundle_dir = store.bundle_dir("session-1");
    assert!(!bundle_dir.join("agents/002-c.json").exists());
    assert!(!bundle_dir.join("prompts/002-c.json").exists());
    assert!(!bundle_dir.join("outputs/002-c.txt").exists());
    assert!(!bundle_dir.join("exports/final-paper.md").exists());
    assert!(
        std::fs::read_dir(bundle_dir.join("rewound"))
            .unwrap()
            .any(|entry| entry.unwrap().path().join("outputs/002-c.txt").exists())
    );
    assert!(
        std::fs::read_dir(bundle_dir.join("rewound"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .join("exports/final-paper.md")
                .exists())
    );

    let verification = verify_bundle(&store, "session-1", false).unwrap();
    assert!(verification.valid, "{:?}", verification.findings);
}

#[test]
fn audit_append_syncs_data_before_returning() {
    let source = include_str!("../src/audit/store.rs");
    let append_event = source
        .split("pub fn append_event")
        .nth(1)
        .and_then(|body| body.split("pub fn write_prompt").next())
        .expect("append_event source");
    let write = append_event.find("file.write_all").expect("audit write");
    let sync = append_event.find("file.sync_data").expect("audit sync");
    let returned = append_event.rfind("Ok(())").expect("append return");
    assert!(write < sync && sync < returned);
}

#[test]
fn terminal_audit_event_is_visible_before_terminal_state() {
    for outcome in ["completed", "failed", "aborted"] {
        let temp = tempfile::tempdir().unwrap();
        let session_id = outcome;
        let mut audit = PipelineAuditRun::start(
            temp.path(),
            session_id,
            PipelineType::Coding,
            "terminal ordering",
        )
        .unwrap();

        match outcome {
            "failed" => audit.fail("boom").unwrap(),
            "completed" => audit.complete("done").unwrap(),
            "aborted" => audit.abort("user aborted run").unwrap(),
            _ => unreachable!(),
        }

        let store = PipelineBundleStore::new(temp.path());
        let state_modified = std::fs::metadata(store.bundle_dir(session_id).join("state.json"))
            .unwrap()
            .modified()
            .unwrap();
        let audit_modified = std::fs::metadata(store.bundle_dir(session_id).join("audit.log"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(
            audit_modified <= state_modified,
            "terminal audit event must be durable before terminal state"
        );

        let events = std::fs::read_to_string(store.bundle_dir(session_id).join("audit.log"))
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<PipelineEventLine>(line)
                    .unwrap()
                    .event
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            events.last(),
            Some(
                PipelineEvent::RunFailed { .. }
                    | PipelineEvent::RunCompleted { .. }
                    | PipelineEvent::RunAborted { .. }
            )
        ));
    }
}

#[test]
fn rewind_refuses_running_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let store = PipelineBundleStore::new(temp.path());
    store
        .create("session-1", PipelineType::Research, "research")
        .unwrap();

    let state = store.load_state("session-1").unwrap();
    assert_eq!(state.status, BundleStatus::Running);

    let error = rewind_bundle(&store, "session-1", 0, "should not race")
        .expect_err("running rewind must fail");
    assert!(
        error
            .to_string()
            .contains("cannot rewind a running pipeline")
    );
}
