//! Reflexion trajectory injection for retry agents.
//!
//! When an agent fails and is retried, this module provides formatted context
//! from prior failed attempts so the retry can learn from mistakes.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A recorded failed trajectory for reflexion injection.
#[derive(Debug, Clone)]
pub struct FailedTrajectory {
    pub agent_name: String,
    pub attempt: usize,
    pub output_summary: String,
    pub failure_reason: String,
    pub quality_score: f64,
    pub timestamp: u64,
}

/// Context injected into a retry agent's prompt.
#[derive(Debug, Clone)]
pub struct ReflexionContext {
    pub prior_attempts: Vec<FailedTrajectory>,
    pub formatted_prompt_section: String,
}

// ---------------------------------------------------------------------------
// ReflexionInjector
// ---------------------------------------------------------------------------

/// Manages failed trajectory storage and reflexion prompt injection.
pub struct ReflexionInjector {
    /// Per-agent list of failed trajectories.
    failures: HashMap<String, Vec<FailedTrajectory>>,
    /// Maximum number of failed trajectories to retain per agent.
    max_per_agent: usize,
}

impl ReflexionInjector {
    /// Create a new injector with a per-agent capacity limit.
    pub fn new(max_per_agent: usize) -> Self {
        Self {
            failures: HashMap::new(),
            max_per_agent: if max_per_agent == 0 { 5 } else { max_per_agent },
        }
    }

    /// Record a failed trajectory for an agent.
    ///
    /// If the agent already has `max_per_agent` entries, the oldest is dropped.
    pub fn record_failure(&mut self, trajectory: FailedTrajectory) {
        let entries = self
            .failures
            .entry(trajectory.agent_name.clone())
            .or_default();
        if entries.len() >= self.max_per_agent {
            entries.remove(0);
        }
        entries.push(trajectory);
    }

    /// Determine whether reflexion should be injected for this agent attempt.
    ///
    /// Returns `true` only if `attempt > 1` AND prior failures exist.
    pub fn should_inject_reflexion(&self, agent_name: &str, attempt: usize) -> bool {
        if attempt <= 1 {
            return false;
        }
        self.failures
            .get(agent_name)
            .map(|f| !f.is_empty())
            .unwrap_or(false)
    }

    /// Build reflexion context from prior failed attempts.
    ///
    /// Returns `None` if no failures are recorded for the agent.
    pub fn inject_reflexion(&self, agent_name: &str) -> Option<ReflexionContext> {
        let entries = self.failures.get(agent_name)?;
        if entries.is_empty() {
            return None;
        }

        let formatted = Self::format_failure_context(entries);

        Some(ReflexionContext {
            prior_attempts: entries.clone(),
            formatted_prompt_section: formatted,
        })
    }

    /// Format failed trajectories into a markdown prompt section.
    ///
    /// Each entry includes attempt number, quality score, failure reason,
    /// and a truncated summary (max 500 chars).
    fn format_failure_context(failures: &[FailedTrajectory]) -> String {
        let mut sections = Vec::with_capacity(failures.len());

        for f in failures {
            let summary = if f.output_summary.len() > 500 {
                format!("{}...", &f.output_summary[..500])
            } else {
                f.output_summary.clone()
            };

            sections.push(format!(
                "### Attempt #{}\n\
                 - **Quality**: {:.2}\n\
                 - **Failure reason**: {}\n\
                 - **Summary**: {}",
                f.attempt, f.quality_score, f.failure_reason, summary,
            ));
        }

        format!(
            "## Reflexion — Prior Failed Attempts\n\n\
             The following prior attempts failed. Learn from these mistakes:\n\n{}",
            sections.join("\n\n")
        )
    }

    /// Remove all recorded failures for a specific agent.
    pub fn clear_agent(&mut self, agent_name: &str) {
        self.failures.remove(agent_name);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_failure(agent: &str, attempt: usize, reason: &str) -> FailedTrajectory {
        FailedTrajectory {
            agent_name: agent.to_string(),
            attempt,
            output_summary: format!("Output from attempt {}", attempt),
            failure_reason: reason.to_string(),
            quality_score: 0.3,
            timestamp: 1000 + attempt as u64,
        }
    }

    #[test]
    fn should_inject_false_on_attempt_1() {
        let mut injector = ReflexionInjector::new(5);
        injector.record_failure(make_failure("agent-a", 1, "type error"));

        assert!(!injector.should_inject_reflexion("agent-a", 1));
    }

    #[test]
    fn should_inject_true_on_attempt_2_with_prior_failure() {
        let mut injector = ReflexionInjector::new(5);
        injector.record_failure(make_failure("agent-a", 1, "type error"));

        assert!(injector.should_inject_reflexion("agent-a", 2));
    }

    #[test]
    fn inject_reflexion_returns_none_when_no_failures() {
        let injector = ReflexionInjector::new(5);

        assert!(injector.inject_reflexion("unknown-agent").is_none());
    }

    #[test]
    fn inject_reflexion_returns_formatted_context() {
        let mut injector = ReflexionInjector::new(5);
        injector.record_failure(make_failure("agent-a", 1, "type error"));
        injector.record_failure(make_failure("agent-a", 2, "bounds check"));

        let ctx = injector.inject_reflexion("agent-a");
        assert!(ctx.is_some());

        let ctx = ctx.unwrap();
        assert_eq!(ctx.prior_attempts.len(), 2);
        assert!(ctx.formatted_prompt_section.contains("Reflexion"));
        assert!(ctx.formatted_prompt_section.contains("type error"));
        assert!(ctx.formatted_prompt_section.contains("bounds check"));
        assert!(ctx.formatted_prompt_section.contains("Attempt #1"));
        assert!(ctx.formatted_prompt_section.contains("Attempt #2"));
    }

    #[test]
    fn clear_agent_removes_only_that_agent() {
        let mut injector = ReflexionInjector::new(5);
        injector.record_failure(make_failure("agent-a", 1, "err-a"));
        injector.record_failure(make_failure("agent-b", 1, "err-b"));

        injector.clear_agent("agent-a");

        assert!(injector.inject_reflexion("agent-a").is_none());
        assert!(injector.inject_reflexion("agent-b").is_some());
    }

    #[test]
    fn format_includes_failure_reason_and_truncated_summary() {
        let long_summary = "x".repeat(600);
        let mut injector = ReflexionInjector::new(5);
        injector.record_failure(FailedTrajectory {
            agent_name: "agent-a".to_string(),
            attempt: 1,
            output_summary: long_summary,
            failure_reason: "overflow".to_string(),
            quality_score: 0.2,
            timestamp: 1000,
        });

        let ctx = injector.inject_reflexion("agent-a").unwrap();
        // Summary should be truncated to 500 chars + "..."
        assert!(ctx.formatted_prompt_section.contains("overflow"));
        assert!(ctx.formatted_prompt_section.contains("..."));
        // The full 600-char string should NOT appear
        assert!(!ctx.formatted_prompt_section.contains(&"x".repeat(600)));
    }
}
