use crate::learning::sona::{FeedbackInput, SonaEngine};

// ---------------------------------------------------------------------------
// SherlockLearningIntegration
// ---------------------------------------------------------------------------

/// Sherlock verdict classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherlockVerdict {
    Approved,
    Rejected,
    NeedsRevision,
}

/// Feeds Sherlock verdicts into the learning subsystems.
pub struct SherlockLearningIntegration {
    sona: Option<SonaEngine>,
    verdicts: Vec<(String, SherlockVerdict)>,
    failed_patterns_list: Vec<String>,
}

impl SherlockLearningIntegration {
    pub fn new(sona: Option<SonaEngine>) -> Self {
        Self {
            sona,
            verdicts: Vec::new(),
            failed_patterns_list: Vec::new(),
        }
    }

    /// Record a Sherlock verdict for an agent.
    ///
    /// Maps Approved->0.9, NeedsRevision->0.5, Rejected->0.2 quality.
    /// If SONA is available and has a trajectory for this agent, records feedback.
    /// Stores failed patterns for Rejected verdicts.
    pub fn record_verdict(&mut self, agent_name: &str, verdict: SherlockVerdict) {
        let quality = match verdict {
            SherlockVerdict::Approved => 0.9,
            SherlockVerdict::NeedsRevision => 0.5,
            SherlockVerdict::Rejected => 0.2,
        };

        self.verdicts.push((agent_name.to_string(), verdict));

        if verdict == SherlockVerdict::Rejected {
            self.failed_patterns_list
                .push(format!("rejected:{}", agent_name));
        }

        // If SONA available, attempt to provide feedback.
        // We create a synthetic trajectory for the verdict since we don't
        // necessarily have an active trajectory ID.
        if let Some(ref mut sona) = self.sona {
            let route = format!("sherlock/{}", agent_name);
            let traj = sona.create_trajectory(&route, agent_name, "sherlock");
            let input = FeedbackInput {
                trajectory_id: traj.trajectory_id,
                quality,
                l_score: quality,
                success_rate: if verdict == SherlockVerdict::Approved {
                    1.0
                } else {
                    quality
                },
            };
            let _ = sona.provide_feedback(&input);
        }
    }

    /// Calculate the pass rate (Approved / total).
    pub fn pass_rate(&self) -> f64 {
        if self.verdicts.is_empty() {
            return 0.0;
        }
        let approved = self
            .verdicts
            .iter()
            .filter(|(_, v)| *v == SherlockVerdict::Approved)
            .count();
        approved as f64 / self.verdicts.len() as f64
    }

    /// Return all recorded failed patterns.
    pub fn failed_patterns(&self) -> &[String] {
        &self.failed_patterns_list
    }
}
