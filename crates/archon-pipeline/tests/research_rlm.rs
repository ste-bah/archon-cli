use std::time::{Duration, Instant};

use archon_pipeline::research::agents::{get_agent_by_key, get_all_agents};
use archon_pipeline::research::rlm::ResearchRlm;
use archon_pipeline::runner::{
    AgentInfo, AgentResult, PipelineSession, PipelineType, ToolAccessLevel,
};

fn session() -> PipelineSession {
    PipelineSession {
        id: "test-session".to_string(),
        pipeline_type: PipelineType::Research,
        task: "write a research paper".to_string(),
        started_at: Instant::now(),
        agent_results: Vec::new(),
        leann_context: String::new(),
    }
}

fn agent_info(key: &str, phase: u32) -> AgentInfo {
    AgentInfo {
        key: key.to_string(),
        display_name: key.to_string(),
        model: "sonnet".to_string(),
        phase,
        critical: true,
        parallelizable: false,
        quality_threshold: 0.5,
        tool_access_level: ToolAccessLevel::ReadOnly,
    }
}

fn agent_result(output: &str) -> AgentResult {
    AgentResult {
        output: output.to_string(),
        tool_use_log: Vec::new(),
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0.0,
        duration: Duration::from_millis(1),
        quality: None,
    }
}

#[test]
fn research_rlm_injects_pinned_structure_and_rolling_outputs() {
    let mut rlm = ResearchRlm::new();
    let architect = get_agent_by_key("dissertation-architect").unwrap();
    let intro = get_agent_by_key("introduction-writer").unwrap();
    let validator = get_agent_by_key("consistency-validator").unwrap();

    rlm.write_agent_output(architect, 5, "# Structure\n\n**Total Chapters**: 8\n");
    rlm.write_agent_output(intro, 31, "Chapter 1 introduces the system.");

    let context = rlm.build_context(&session(), validator);

    assert!(context.contains("Research RLM Identity"));
    assert!(context.contains("Logical `research/...` namespaces are memory keys"));
    assert!(
        context
            .contains(".archon/pipelines/test-session/outputs/markdown/031-introduction-writer.md")
    );
    assert!(!context.contains(".md.md"));
    assert!(!context.contains("/Volumes/Externalwork/archon-cli/project-1/research/writing"));
    assert!(context.contains("Pinned Output `005-dissertation-architect`"));
    assert!(context.contains("Rolling Output `031-introduction-writer`"));
    assert!(context.contains("Deterministic Consistency Pre-Scan"));
    assert!(context.contains("Locked chapter count detected: 8"));
}

#[test]
fn final_assembly_receives_writer_outputs() {
    let mut rlm = ResearchRlm::new();
    for (ordinal, key) in [
        "dissertation-architect",
        "introduction-writer",
        "literature-review-writer",
        "methodology-writer",
        "results-writer",
        "discussion-writer",
        "conclusion-writer",
        "abstract-writer",
    ]
    .iter()
    .enumerate()
    {
        let agent = get_agent_by_key(key).unwrap();
        rlm.write_agent_output(agent, ordinal, &format!("output from {key}"));
    }

    let final_agent = get_agent_by_key("chapter-synthesizer").unwrap();
    let context = rlm.build_context(&session(), final_agent);

    assert!(context.contains("output from introduction-writer"));
    assert!(context.contains("output from literature-review-writer"));
    assert!(context.contains("output from abstract-writer"));
}

#[test]
fn consistency_validator_does_not_overwrite_writer_namespaces() {
    let mut rlm = ResearchRlm::new();
    let intro = get_agent_by_key("introduction-writer").unwrap();
    let validator = get_agent_by_key("consistency-validator").unwrap();
    let final_agent = get_agent_by_key("chapter-synthesizer").unwrap();

    rlm.write_agent_output(intro, 28, "real introduction chapter");
    rlm.write_agent_output(validator, 42, "validator status report");

    let context = rlm.build_context(&session(), final_agent);

    assert!(context.contains("real introduction chapter"));
    assert!(!context.contains("RLM Namespace `research/writing/introduction`\n\nvalidator"));
}

#[test]
fn resumed_context_prefers_restored_session_when_rlm_is_partial() {
    let mut rlm = ResearchRlm::new();
    let quality = get_agent_by_key("quality-assessor").unwrap();
    rlm.write_agent_output(quality, 43, "partial resumed quality output");

    let mut session = session();
    session.agent_results.push((
        agent_info("introduction-writer", 6),
        agent_result("restored introduction from audit bundle"),
    ));
    session.agent_results.push((
        agent_info("literature-review-writer", 6),
        agent_result("restored literature from audit bundle"),
    ));

    let final_agent = get_agent_by_key("chapter-synthesizer").unwrap();
    let context = rlm.build_context(&session, final_agent);

    assert!(context.contains("Completed agents: 2"));
    assert!(context.contains("restored introduction from audit bundle"));
    assert!(context.contains("restored literature from audit bundle"));
    assert!(!context.contains("Completed agents: 1"));
}

#[test]
fn every_agent_can_build_research_rlm_context() {
    let mut rlm = ResearchRlm::new();
    let first = get_agent_by_key("step-back-analyzer").unwrap();
    rlm.write_agent_output(first, 0, "framing output");

    for agent in get_all_agents() {
        let context = rlm.build_context(&session(), agent);
        assert!(
            context.contains("Accepted Output Manifest"),
            "missing manifest for {}",
            agent.key
        );
    }
}
