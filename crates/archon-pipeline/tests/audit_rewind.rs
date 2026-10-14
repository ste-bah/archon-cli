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
        tokens_in: Some(10),
        tokens_out: Some(20),
        cost_usd: 0.01,
        duration: Duration::from_millis(1),
        quality: None,
    }
}

#[test]
fn pipeline_audit_persists_unknown_usage_as_null_and_keeps_reported_zero() {
    let temp = tempfile::tempdir().unwrap();
    let mut audit = PipelineAuditRun::start(
        temp.path(),
        "session-usage",
        PipelineType::Research,
        "usage persistence",
    )
    .unwrap();

    let unknown_agent = agent("unknown");
    let unknown_prompt = audit
        .record_prompt(0, &unknown_agent, &[], &[], &[])
        .unwrap();
    let unknown = AgentResult {
        tokens_in: None,
        tokens_out: None,
        ..result("unknown")
    };
    audit
        .record_agent_completed(0, &unknown_agent, &unknown, Vec::new(), unknown_prompt)
        .unwrap();

    let zero_agent = agent("zero");
    let zero_prompt = audit.record_prompt(1, &zero_agent, &[], &[], &[]).unwrap();
    let zero = AgentResult {
        tokens_in: Some(0),
        tokens_out: Some(0),
        ..result("zero")
    };
    audit
        .record_agent_completed(1, &zero_agent, &zero, Vec::new(), zero_prompt)
        .unwrap();

    let unknown_record = audit.store().list_agent_records("session-usage").unwrap()[0].clone();
    let zero_record = audit.store().list_agent_records("session-usage").unwrap()[1].clone();
    let unknown_json = serde_json::to_value(unknown_record).unwrap();
    let zero_json = serde_json::to_value(zero_record).unwrap();
    assert!(unknown_json["tokens_in"].is_null());
    assert!(unknown_json["tokens_out"].is_null());
    assert_eq!(zero_json["tokens_in"], 0);
    assert_eq!(zero_json["tokens_out"], 0);
}

#[test]
fn bundle_totals_keep_unknown_agent_counts_separate_from_known_sums() {
    // Expected: known sums stay numeric, while unknown usage increments the
    // direction-specific count so consumers can label each sum incomplete.
    let temp = tempfile::tempdir().unwrap();
    let mut audit = PipelineAuditRun::start(
        temp.path(),
        "session-incomplete-totals",
        PipelineType::Research,
        "usage totals",
    )
    .unwrap();
    let mut before_unknown = None;
    for (ordinal, key, tokens_in, tokens_out) in
        [(0, "known", Some(7), Some(3)), (1, "unknown", None, None)]
    {
        let info = agent(key);
        let prompt = audit.record_prompt(ordinal, &info, &[], &[], &[]).unwrap();
        let usage = AgentResult {
            tokens_in,
            tokens_out,
            ..result(key)
        };
        audit
            .record_agent_completed(ordinal, &info, &usage, Vec::new(), prompt)
            .unwrap();
        if ordinal == 0 {
            let known_only = audit
                .store()
                .load_state("session-incomplete-totals")
                .unwrap();
            assert_eq!(known_only.total_tokens_in, 7);
            assert_eq!(known_only.total_tokens_out, 3);
            assert_eq!(known_only.tokens_in_unknown_agents, 0);
            assert_eq!(known_only.tokens_out_unknown_agents, 0);
            before_unknown = Some(known_only);
        }
    }

    let state = audit
        .store()
        .load_state("session-incomplete-totals")
        .unwrap();
    assert_eq!(state.total_tokens_in, 7);
    assert_eq!(state.total_tokens_out, 3);
    assert_eq!(state.tokens_in_unknown_agents, 1);
    assert_eq!(state.tokens_out_unknown_agents, 1);
    let persisted = std::fs::read_to_string(
        audit
            .store()
            .bundle_dir("session-incomplete-totals")
            .join("state.json"),
    )
    .unwrap();
    let before_unknown = before_unknown.unwrap();
    eprintln!(
        "EVIDENCE #369 total BEFORE: persisted known sums in={}/out={} with unknown counts in={}/out={}; ACTION: completed an agent whose counts are unknown; EXPECTED: sums unchanged and both unknown counts increase; AFTER: persisted in={} ({} unknown), out={} ({} unknown). SOURCE: state.json={persisted}",
        before_unknown.total_tokens_in,
        before_unknown.total_tokens_out,
        before_unknown.tokens_in_unknown_agents,
        before_unknown.tokens_out_unknown_agents,
        state.total_tokens_in,
        state.tokens_in_unknown_agents,
        state.total_tokens_out,
        state.tokens_out_unknown_agents
    );
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
fn rewind_recomputes_known_sums_and_unknown_counts_from_kept_records() {
    // Expected: after keeping records 0 and 1, each known sum and each
    // direction-specific unknown count matches those kept records exactly.
    let temp = tempfile::tempdir().unwrap();
    let store = PipelineBundleStore::new(temp.path());
    store
        .create(
            "session-unknown-rewind",
            PipelineType::Research,
            "rewind usage",
        )
        .unwrap();
    let mut audit = PipelineAuditRun::resume(temp.path(), "session-unknown-rewind").unwrap();

    for (ordinal, key, tokens_in, tokens_out) in [
        (0, "in-unknown", None, Some(5)),
        (1, "out-unknown", Some(7), None),
        (2, "stale", Some(100), Some(200)),
    ] {
        let info = agent(key);
        let prompt = audit.record_prompt(ordinal, &info, &[], &[], &[]).unwrap();
        let usage = AgentResult {
            tokens_in,
            tokens_out,
            ..result(key)
        };
        audit
            .record_agent_completed(ordinal, &info, &usage, Vec::new(), prompt)
            .unwrap();
    }
    audit.fail("rewind after partial usage").unwrap();

    rewind_bundle(&store, "session-unknown-rewind", 2, "keep partial usage").unwrap();

    let kept = store.list_agent_records("session-unknown-rewind").unwrap();
    let state = store.load_state("session-unknown-rewind").unwrap();
    let expected_tokens_in: u64 = kept.iter().filter_map(|record| record.tokens_in).sum();
    let expected_tokens_out: u64 = kept.iter().filter_map(|record| record.tokens_out).sum();
    let expected_unknown_in = kept
        .iter()
        .filter(|record| record.tokens_in.is_none())
        .count() as u64;
    let expected_unknown_out = kept
        .iter()
        .filter(|record| record.tokens_out.is_none())
        .count() as u64;
    assert_eq!(state.total_tokens_in, expected_tokens_in);
    assert_eq!(state.total_tokens_out, expected_tokens_out);
    assert_eq!(state.tokens_in_unknown_agents, expected_unknown_in);
    assert_eq!(state.tokens_out_unknown_agents, expected_unknown_out);
    eprintln!(
        "EVIDENCE #369 rewind ACTION: kept {} records; EXPECTED sums in={} out={} and unknown counts in={} out={}; ACTUAL state sums in={} out={} and unknown counts in={} out={}; SOURCE: kept agent records",
        kept.len(),
        expected_tokens_in,
        expected_tokens_out,
        expected_unknown_in,
        expected_unknown_out,
        state.total_tokens_in,
        state.total_tokens_out,
        state.tokens_in_unknown_agents,
        state.tokens_out_unknown_agents
    );
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
