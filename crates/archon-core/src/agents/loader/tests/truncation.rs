use super::*;

#[test]
fn truncate_under_budget_preserves_all() {
    let result = truncate_agent_prompt("agent", "behavior", "context", 100);
    assert_eq!(result, "agent\n\nbehavior\n\ncontext");
}

#[test]
fn truncate_context_first_when_over_budget() {
    let agent = "A".repeat(50);
    let behavior = "B".repeat(50);
    let context = "C".repeat(500);
    // Budget: 40 tokens = 160 chars. agent(50) + sep(4) = 54, remaining = 106
    // behavior(50) fits, context budget = 106-50 = 56, context(500) > 56 → truncated
    let result = truncate_agent_prompt(&agent, &behavior, &context, 40);
    assert!(result.starts_with(&agent));
    assert!(result.contains(&behavior));
    // full context should NOT be present
    assert!(!result.contains(&context));
    // but truncated marker should be
    assert!(result.contains("... [truncated]"));
}

#[test]
fn truncate_behavior_when_context_gone_not_enough() {
    let agent = "A".repeat(50);
    let behavior = "B".repeat(500);
    let context = "C".repeat(500);
    // Budget: 20 tokens = 80 chars. agent(50) + sep(4) = 54, remaining = 26
    // context trimmed first, behavior 500 > 26, so behavior also trimmed
    let result = truncate_agent_prompt(&agent, &behavior, &context, 20);
    assert!(result.starts_with(&agent));
    assert!(result.contains("... [truncated]"));
    assert!(!result.contains(&behavior));
}

#[test]
fn truncate_agent_md_never_trimmed() {
    // agent_md bigger than total budget — still not trimmed
    let agent = "A".repeat(200);
    let behavior = "B".repeat(10);
    let context = "C".repeat(10);
    // Budget: 10 tokens = 40 chars. agent alone is 200
    let result = truncate_agent_prompt(&agent, &behavior, &context, 10);
    // agent_md is always included fully
    assert!(result.contains(&agent));
}

#[test]
fn truncate_adds_truncated_marker() {
    let agent = "A".repeat(10);
    let behavior = "B".repeat(10);
    let context = "C".repeat(500);
    // Budget: 15 tokens = 60 chars. Total = 10+10+500+4 = 524, needs truncation
    let result = truncate_agent_prompt(&agent, &behavior, &context, 15);
    assert!(result.contains("... [truncated]"));
}

#[test]
fn truncate_empty_sections_handled() {
    let result = truncate_agent_prompt("agent", "", "", 100);
    assert_eq!(result, "agent");
}

#[test]
fn truncate_wired_into_assemble() {
    // Verify assemble_system_prompt calls truncation (under budget = passthrough)
    let prompt = assemble_system_prompt("# Agent", "rules", "ctx");
    assert_eq!(prompt, "# Agent\n\nrules\n\nctx");
}
