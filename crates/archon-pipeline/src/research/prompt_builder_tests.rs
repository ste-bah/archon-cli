use super::*;
use crate::research::agents::RESEARCH_AGENTS;
use std::sync::OnceLock;

fn builder() -> ResearchPromptBuilder {
    ResearchPromptBuilder::new().with_project_root(research_test_project_root())
}

fn research_test_project_root() -> &'static std::path::Path {
    static ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!(
            "archon-research-prompt-test-project-{}",
            uuid::Uuid::new_v4()
        ));
        for agent in RESEARCH_AGENTS {
            let path = root.join(agent.prompt_source_path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let body = match agent.key {
                "step-back-analyzer" => "Step-Back Analysis Excellence Framework",
                "evidence-synthesizer" => "Evidence Synthesizer research test prompt.",
                _ => "Research test project prompt body.",
            };
            std::fs::write(path, format!("---\nname: {}\n---\n{body}", agent.key)).unwrap();
        }
        root
    })
}

#[test]
fn missing_runtime_prompt_is_an_actionable_error() {
    let mut agent = RESEARCH_AGENTS[0].clone();
    agent.prompt_source_path = ".archon/agents/research/missing-runtime-prompt.md";
    let error = builder()
        .build(&agent, 0, RESEARCH_AGENTS.len(), "task", "", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Research pipeline"));
    assert!(error.contains(agent.key));
    assert!(error.contains(agent.prompt_source_path));
    assert!(error.contains("archon agent sync"));
}

#[test]
fn prompt_has_all_5_sections_phase4() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[14];
    assert_eq!(agent.key, "evidence-synthesizer");

    let prompt = b
        .build(
            agent,
            14,
            47,
            "AI in healthcare",
            "some prior context",
            None,
        )
        .unwrap();

    assert!(
        prompt.contains("## Workflow Context"),
        "missing workflow context"
    );
    assert!(prompt.contains("## Prior Context"), "missing prior context");
    assert!(
        prompt.contains("## Output Expectations"),
        "missing output expectations"
    );
    assert!(
        prompt.contains("## Task Completion"),
        "missing task completion"
    );
    assert!(
        prompt.contains("Evidence Synthesizer") || prompt.contains("evidence-synthesizer"),
        "missing agent instructions reference"
    );
}

#[test]
fn workflow_context_correct_position() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[14];
    let prompt = b.build(agent, 14, 47, "test query", "", None).unwrap();

    assert!(
        prompt.contains("Agent #15 of 47"),
        "should show 1-based index: Agent #15 of 47"
    );
    assert!(
        prompt.contains("Phase: 4 - Synthesis"),
        "should show phase name"
    );
}

#[test]
fn prior_context_truncation() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[0];
    let long_context: String = "x".repeat(15_000);
    let prompt = b.build(agent, 0, 47, "test", &long_context, None).unwrap();

    assert!(
        prompt.contains("... [truncated]"),
        "should have truncation marker"
    );
    let prior_section = prompt.split("## Prior Context").nth(1).unwrap_or("");
    assert!(
        prior_section.chars().count() < 12_000,
        "truncated section should be well under 15000 chars"
    );
}

#[test]
fn empty_prior_context_omitted() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[0];
    let prompt = b.build(agent, 0, 47, "test", "", None).unwrap();

    assert!(
        !prompt.contains("## Prior Context"),
        "empty prior context should be omitted"
    );
}

#[test]
fn backtick_escaping() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[0];
    let context_with_backticks = "Here is code: ```python\nprint('hello')\n```";
    let prompt = b
        .build(agent, 0, 47, "test", context_with_backticks, None)
        .unwrap();

    assert!(
        !prompt.contains("```python"),
        "triple backticks should be escaped"
    );
    assert!(
        prompt.contains("\\`\\`\\`python"),
        "should contain escaped backticks"
    );
}

#[test]
fn output_expectations_memory_key() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[0];
    let prompt = b.build(agent, 0, 47, "test", "", None).unwrap();

    assert!(
        prompt.contains("Store output at: research/foundation/framing"),
        "should reference memory_keys[0]"
    );
    assert!(
        prompt.contains("Expected artifacts: high-level-framing.md, abstraction-analysis.md"),
        "should list output artifacts"
    );
}

#[test]
fn phase6_style_injection() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[28];
    assert_eq!(agent.key, "introduction-writer");
    assert_eq!(agent.phase, 6);

    let prompt = b
        .build(
            agent,
            28,
            47,
            "test",
            "",
            Some("Use American English spelling conventions"),
        )
        .unwrap();

    assert!(
        prompt.contains("## STYLE GUIDELINES"),
        "Phase 6 should have style guidelines"
    );
    assert!(
        prompt.contains("American English"),
        "should include style content"
    );
}

#[test]
fn final_assembly_gets_larger_prior_context_budget() {
    let b = builder();
    let agent = RESEARCH_AGENTS.last().unwrap();
    assert_eq!(agent.key, "chapter-synthesizer");
    assert_eq!(agent.phase, 8);

    let long_context: String = "x".repeat(20_000);
    let prompt = b.build(agent, 46, 47, "test", &long_context, None).unwrap();

    assert!(
        !prompt.contains("... [truncated]"),
        "final assembly should be able to see chapter-scale context"
    );
}

#[test]
fn final_assembly_gets_style_injection() {
    let b = builder();
    let agent = RESEARCH_AGENTS.last().unwrap();
    assert_eq!(agent.key, "chapter-synthesizer");

    let prompt = b
        .build(agent, 46, 47, "test", "", Some("Use APA 7 formatting"))
        .unwrap();

    assert!(
        prompt.contains("## STYLE GUIDELINES"),
        "final assembly should receive style guidelines"
    );
    assert!(prompt.contains("APA 7"));
}

#[test]
fn declared_instruction_file_is_loaded() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[0];
    let prompt = b.build(agent, 0, 47, "test", "", None).unwrap();

    assert!(prompt.contains("Step-Back Analysis Excellence Framework"));
    assert!(
        !prompt.contains("You are the Step-Back Analyzer agent for the PhD research pipeline.")
    );
}

#[test]
fn safe_truncate_char_boundary() {
    let text = "🎉🎊🎈🎆🎇";
    let result = safe_truncate(text, 3);
    assert!(result.starts_with("🎉🎊🎈"));
    assert!(result.contains("... [truncated]"));
}

#[test]
fn safe_truncate_no_op() {
    let text = "short";
    let result = safe_truncate(text, 100);
    assert_eq!(result, "short");
}

#[test]
fn escape_backticks_works() {
    let input = "before ``` middle ``` after";
    let result = escape_backticks(input);
    assert_eq!(result, "before \\`\\`\\` middle \\`\\`\\` after");
}

#[test]
fn phase6_no_style_prompt_no_injection() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[28];
    let prompt = b.build(agent, 28, 47, "test", "", None).unwrap();
    assert!(
        !prompt.contains("## STYLE GUIDELINES"),
        "no style_prompt means no STYLE GUIDELINES section"
    );
}

#[test]
fn workflow_context_prev_next() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[1];
    let prompt = b.build(agent, 1, 47, "test", "", None).unwrap();

    assert!(
        prompt.contains("Previous agent: step-back-analyzer"),
        "should show previous agent key"
    );
    assert!(
        prompt.contains("Next agent: ambiguity-clarifier"),
        "should show next agent key"
    );
}

#[test]
fn first_agent_prev_none() {
    let b = builder();
    let agent = &RESEARCH_AGENTS[0];
    let prompt = b.build(agent, 0, 47, "test", "", None).unwrap();
    assert!(prompt.contains("Previous agent: none (first agent)"));
}

#[test]
fn last_agent_next_none() {
    let b = builder();
    let last_idx = RESEARCH_AGENTS.len() - 1;
    let agent = &RESEARCH_AGENTS[last_idx];
    let prompt = b.build(agent, last_idx, 47, "test", "", None).unwrap();
    assert!(prompt.contains("Next agent: none (final agent)"));
}
