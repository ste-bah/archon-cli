//! Prompt builder for game-theory specialists.
//!
//! Assembles prompts from agent templates, the situation string, Tier 1
//! classification outputs, and dependency specialist outputs.

/// Assemble a specialist prompt from constituent parts.
///
/// The prompt follows the 5-part structure:
/// 1. Role/identity (from agent template)
/// 2. Situation context
/// 3. Tier 1 fingerprint summary
/// 4. Dependency specialist outputs (if any)
/// 5. Task instruction
pub fn build_specialist_prompt(
    agent_key: &str,
    agent_display_name: &str,
    situation: &str,
    fingerprint_summary: &str,
    dependency_outputs: &[(&str, &str)], // (dep_agent_key, dep_output)
) -> String {
    build_specialist_prompt_with_prior_context(
        agent_key,
        agent_display_name,
        situation,
        fingerprint_summary,
        "",
        dependency_outputs,
    )
}

/// Assemble a specialist prompt with recalled prior context.
pub fn build_specialist_prompt_with_prior_context(
    agent_key: &str,
    agent_display_name: &str,
    situation: &str,
    fingerprint_summary: &str,
    prior_context: &str,
    dependency_outputs: &[(&str, &str)], // (dep_agent_key, dep_output)
) -> String {
    let mut parts = Vec::new();

    // Part 1: Role
    parts.push(format!(
        "You are the game-theory specialist **{display}** (`{key}`).",
        display = agent_display_name,
        key = agent_key,
    ));

    // Part 2: Situation
    parts.push(format!("## Situation\n\n{situation}"));

    // Part 3: Tier 1 classification
    parts.push(format!(
        "## Strategic Classification\n\n{fingerprint_summary}"
    ));

    // Part 4: Recalled memory context
    if !prior_context.trim().is_empty() {
        parts.push(format!(
            "## Prior Context\n\n{}",
            safe_truncate(prior_context, 10_000)
        ));
    }

    // Part 4: Dependency outputs
    if !dependency_outputs.is_empty() {
        let mut dep_section = String::from("## Preceding Analysis\n\n");
        for (dep_key, dep_output) in dependency_outputs {
            dep_section.push_str(&format!("### {dep_key}\n\n{dep_output}\n\n---\n\n"));
        }
        parts.push(dep_section);
    }

    // Part 5: Task
    parts.push(format!(
        "## Task\n\nAnalyze the situation above through your specialist lens. \
         Produce a structured analysis with:\n\
         1. Key findings\n\
         2. Evidence and reasoning\n\
         3. Confidence assessment\n\
         4. Recommendations (if applicable)"
    ));

    parts.join("\n\n")
}

fn safe_truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("\n\n[truncated]");
    out
}

/// Generate a fingerprint summary string for use in prompts.
pub fn fingerprint_summary_text(fp: &super::fingerprint::GameTheoryFingerprint) -> String {
    format!(
        "Primary family: {family}\n\
         Cooperation: {coop} ({coop_conf})\n\
         Payoff sum: {payoff} ({payoff_conf})\n\
         Symmetry: {sym} ({sym_conf})\n\
         Timing: {timing} ({timing_conf})\n\
         Information - Perfect: {perf} ({perf_conf}), Complete: {comp} ({comp_conf})\n\
         Players: {card} ({card_conf})\n\
         Strategy space: {strat} ({strat_conf})\n\
         Horizon: {horiz} ({horiz_conf})",
        family = fp.primary_family,
        coop = fp.cooperation.value,
        coop_conf = fp.cooperation.confidence,
        payoff = fp.payoff_sum.value,
        payoff_conf = fp.payoff_sum.confidence,
        sym = fp.symmetry.value,
        sym_conf = fp.symmetry.confidence,
        timing = fp.timing.value,
        timing_conf = fp.timing.confidence,
        perf = fp.perfect_info.value,
        perf_conf = fp.perfect_info.confidence,
        comp = fp.complete_info.value,
        comp_conf = fp.complete_info.confidence,
        card = fp.cardinality.value,
        card_conf = fp.cardinality.confidence,
        strat = fp.strategy_space.value,
        strat_conf = fp.strategy_space.confidence,
        horiz = fp.horizon.value,
        horiz_conf = fp.horizon.confidence,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_specialist_prompt_has_all_sections() {
        let prompt = build_specialist_prompt(
            "gt-nash",
            "Nash Equilibrium Finder",
            "Two firms set quantities.",
            "Primary family: Cournot competition\nCooperation: non-cooperative",
            &[("gt-payoff", "Payoff matrix: [[5,3],[3,5]]")],
        );

        assert!(prompt.contains("gt-nash"));
        assert!(prompt.contains("Nash Equilibrium Finder"));
        assert!(prompt.contains("Two firms set quantities"));
        assert!(prompt.contains("Cournot competition"));
        assert!(prompt.contains("gt-payoff"));
        assert!(prompt.contains("Payoff matrix"));
        assert!(prompt.contains("## Task"));
    }

    #[test]
    fn test_build_specialist_prompt_no_dependencies() {
        let prompt = build_specialist_prompt(
            "gt-standalone",
            "Standalone Analyst",
            "A simple game.",
            "Primary family: Strategic interaction",
            &[],
        );

        assert!(prompt.contains("gt-standalone"));
        assert!(!prompt.contains("## Preceding Analysis"));
    }

    #[test]
    fn test_build_specialist_prompt_includes_prior_context() {
        let prompt = build_specialist_prompt_with_prior_context(
            "nash-equilibrium-finder",
            "Nash Equilibrium Finder",
            "Two firms set prices.",
            "Primary family: Bertrand competition",
            "Prior payoff evidence from memory",
            &[],
        );

        assert!(prompt.contains("## Prior Context"));
        assert!(prompt.contains("Prior payoff evidence from memory"));
    }

    #[test]
    fn test_fingerprint_summary_text_includes_all_axes() {
        use crate::gametheory::fingerprint::{AxisVerdict, GameTheoryFingerprint};

        let fp = GameTheoryFingerprint {
            run_id: "test".into(),
            cooperation: AxisVerdict::new("cooperative", "high", ""),
            payoff_sum: AxisVerdict::new("positive-sum", "medium", ""),
            symmetry: AxisVerdict::new("symmetric", "high", ""),
            timing: AxisVerdict::new("sequential", "medium", ""),
            perfect_info: AxisVerdict::new("perfect", "high", ""),
            complete_info: AxisVerdict::new("complete", "high", ""),
            cardinality: AxisVerdict::new("n-player", "medium", ""),
            strategy_space: AxisVerdict::new("discrete", "medium", ""),
            horizon: AxisVerdict::new("repeated", "high", ""),
            primary_family: "Coordination game".into(),
            nearest_classic: Some("Battle of the Sexes".into()),
            shadow_games: vec![],
            hidden_game_scan: None,
            ambiguities: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let summary = fingerprint_summary_text(&fp);
        assert!(summary.contains("Coordination game"));
        assert!(summary.contains("cooperative"));
        assert!(summary.contains("positive-sum"));
        assert!(summary.contains("symmetric"));
        assert!(summary.contains("sequential"));
        assert!(summary.contains("perfect"));
        assert!(summary.contains("complete"));
        assert!(summary.contains("n-player"));
        assert!(summary.contains("discrete"));
        assert!(summary.contains("repeated"));
    }
}
