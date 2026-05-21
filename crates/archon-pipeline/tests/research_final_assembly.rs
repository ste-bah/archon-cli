use std::time::{Duration, Instant};

use archon_pipeline::research::final_assembly::assemble_result;
use archon_pipeline::runner::{
    AgentInfo, AgentResult, PipelineSession, PipelineType, ToolAccessLevel,
};

fn agent(key: &str) -> AgentInfo {
    AgentInfo {
        key: key.to_string(),
        display_name: key.to_string(),
        model: "test".to_string(),
        phase: 8,
        critical: true,
        parallelizable: false,
        quality_threshold: 0.5,
        tool_access_level: ToolAccessLevel::Full,
    }
}

fn result(output: &str) -> AgentResult {
    AgentResult {
        output: output.to_string(),
        tool_use_log: Vec::new(),
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0.0,
        duration: Duration::ZERO,
        quality: None,
    }
}

#[test]
fn final_result_preserves_dynamic_chapters_over_short_combiner() {
    let architect = r#"### Chapter 1: Introduction
**Expected Word Count**: 3,000
**Content Outline**:
- Background

### Chapter 2: Architecture
**Expected Word Count**: 3,000
**Content Outline**:
- Components
"#;
    let mut session = PipelineSession {
        id: "test".to_string(),
        pipeline_type: PipelineType::Research,
        task: "Write a research paper about GKB match scoring.".to_string(),
        started_at: Instant::now(),
        agent_results: Vec::new(),
        leann_context: String::new(),
    };
    session
        .agent_results
        .push((agent("dissertation-architect"), result(architect)));
    session.agent_results.push((
        agent("abstract-writer"),
        result(
            "# Abstract Draft\n\n## Abstract\n\nREAL ABSTRACT MUST SURVIVE.\n\n## Abstract Quality Check\n\nQUALITY JUNK MUST NOT SURVIVE.",
        ),
    ));
    session.agent_results.push((
        agent("citation-reconciler"),
        result(
            "## Master Reference List\n\nSmith, J. (2024). Screening systems.\n\nAdams, R. (2023). Architecture controls.\n\n## Removed or Downgraded Citations\n\nBad source.",
        ),
    ));
    session.agent_results.push((
        agent("citation-validator"),
        result("## References\n\n**Final verdict**: NEEDS REVISION BEFORE PUBLICATION."),
    ));
    session.agent_results.push((
        agent("chapter-writer-001-introduction"),
        result("# Chapter 1: Introduction\n\nFULL INTRODUCTION PROSE MUST SURVIVE."),
    ));
    session.agent_results.push((
        agent("chapter-writer-002-architecture"),
        result("# Chapter 2: Architecture\n\nFULL ARCHITECTURE PROSE MUST SURVIVE."),
    ));
    session.agent_results.push((
        agent("final-paper-combiner"),
        result("# Short Paper\n\n## Abstract\n\nTiny summary.\n\n## References\n\nSmith."),
    ));

    let output = assemble_result(session).unwrap().final_output;
    assert!(output.contains("REAL ABSTRACT MUST SURVIVE"));
    assert!(!output.contains("QUALITY JUNK MUST NOT SURVIVE"));
    assert!(output.contains("FULL INTRODUCTION PROSE MUST SURVIVE"));
    assert!(output.contains("FULL ARCHITECTURE PROSE MUST SURVIVE"));
    assert!(output.contains("Adams, R. (2023). Architecture controls."));
    assert!(output.contains("Smith, J. (2024). Screening systems."));
    assert!(!output.contains("Bad source."));
    assert!(!output.contains("Tiny summary"));
    assert!(output.contains("## Appendix A: Primary Architecture Source Register"));
    assert!(output.contains("## Appendix B: Locked Chapter Architecture"));
    assert!(output.contains("| 2 | Architecture | 3000 | Components |"));
}
