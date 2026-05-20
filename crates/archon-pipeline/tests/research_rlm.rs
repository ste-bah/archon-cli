use std::time::Instant;

use archon_pipeline::research::agents::{get_agent_by_key, get_all_agents};
use archon_pipeline::research::rlm::ResearchRlm;
use archon_pipeline::runner::{PipelineSession, PipelineType};

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
