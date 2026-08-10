//! The live emitter for `attribution_followup_evaluated`.
//!
//! An attribution row says what the engine thought caused a correction. It says
//! nothing about whether attributing it helped, and that — not accuracy — is
//! what the R2 promotion gate is about: accepted attributions must be followed
//! by fewer repeated verified failures than abstained or unattributed ones.
//!
//! This runs at the end of every turn that executed a tool, because that is what
//! an opportunity for the mistake to recur looks like. It reads the session's
//! prior attribution rows back out of the metric store and writes one row per
//! (correction, opportunity) pair still inside its follow-up window.
//!
//! Pure telemetry: unlike the attribution itself, nothing downstream reads the
//! result, so this fails open and is dispatched to the blocking pool without
//! the turn waiting for it.

use archon_cognitive::attribution::followup::{
    AttributedCorrection, FollowupOpportunity, attributed_corrections, followup_event,
    followup_window,
};

/// Wall-clock budget for one turn's follow-up pass.
const FOLLOWUP_BUDGET_MS: u64 = 1_000;

/// Everything the pass needs, gathered on the turn thread.
pub(super) struct FollowupObservation {
    pub session_id: String,
    pub task_class: String,
    pub model_id: String,
    pub opportunity: FollowupOpportunity,
}

/// Write one row per repeated opportunity this turn presented.
///
/// Returns how many rows were newly written, which is what a caller would log
/// and what the tests assert on.
pub(super) fn record_followup_opportunities(
    store: &archon_cognitive::PersistentCognitiveStore,
    observation: &FollowupObservation,
) -> Result<usize, archon_cognitive::CognitiveError> {
    let event_store = archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())?;
    let events = event_store.events()?;
    let attributions = attributed_corrections(&events, &observation.session_id);

    let window = followup_window(observation.opportunity.observed_at);
    event_store.declare_window(&window)?;

    let mut written = 0usize;
    for attribution in &attributions {
        if !attribution.covers(
            &observation.opportunity.session_id,
            observation.opportunity.turn_number,
        ) {
            continue;
        }
        let cohort = followup_cohort(&observation.task_class, &observation.model_id, attribution);
        let event = followup_event(attribution, &observation.opportunity, cohort, &window);
        match event_store.record(&event) {
            Ok(archon_cognitive::MetricWriteOutcome::Written) => written += 1,
            Ok(archon_cognitive::MetricWriteOutcome::DuplicateIgnored) => {}
            Err(error) => {
                // One bad row must not lose the rest of the turn's
                // opportunities, and a silently dropped opportunity biases the
                // cohort rate towards whichever arm happened to write cleanly.
                tracing::warn!(
                    %error,
                    correction_id = %attribution.correction_id,
                    "follow-up opportunity row rejected"
                );
            }
        }
    }
    Ok(written)
}

/// Cohort for a follow-up row.
///
/// Segmented by the ATTRIBUTION's policy version rather than the current turn's,
/// so a correction attributed under one procedure keeps its opportunities in
/// that procedure's population. Task class and model come from the turn that
/// presented the opportunity, which is what the stratum is about.
fn followup_cohort(
    task_class: &str,
    model_id: &str,
    attribution: &AttributedCorrection,
) -> archon_cognitive::MetricCohort {
    archon_cognitive::MetricCohort::new(task_class, model_id, attribution.policy_version.as_str())
}

impl super::Agent {
    /// Record this turn as a repeated opportunity for the session's prior
    /// corrections, if it ran anything.
    ///
    /// A turn that executed no tool is not an opportunity: nothing could have
    /// recurred, and counting it would dilute both cohorts equally while making
    /// the denominators describe conversation length rather than exposure.
    ///
    /// Runs after the turn has already been reported complete, off the async
    /// runtime and under a hard budget. Awaited rather than fired and forgotten
    /// so "the opportunity was recorded" is an observation a test can make
    /// rather than a race it has to poll for.
    ///
    /// `None` means the pass did not complete; `Some(n)` is the number of new
    /// rows. Nothing downstream reads either, which is why this may fail open
    /// where the attribution itself may not.
    pub(super) async fn record_attribution_followup(&self, user_input: &str) -> Option<usize> {
        let store = self.cognitive_store.as_ref().map(std::sync::Arc::clone)?;
        let (tool_names, failures) =
            super::cognitive_gate::turn_tool_activity(&self.state.messages, user_input);
        if tool_names.is_empty() {
            return Some(0);
        }

        let session_id = self.config.session_id.clone();
        let observation = FollowupObservation {
            task_class: self
                .current_situation
                .as_ref()
                .map_or("unclassified", |situation| situation.kind.as_str())
                .to_string(),
            model_id: self.config.model.clone(),
            opportunity: FollowupOpportunity {
                session_id: session_id.clone(),
                turn_number: self.turn_number,
                verified_failure: failures > 0,
                observed_at: chrono::Utc::now(),
            },
            session_id,
        };

        super::cognitive_gate::bounded_cognitive_observation(
            FOLLOWUP_BUDGET_MS,
            "record-attribution-followup",
            move || {
                let store = store
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                record_followup_opportunities(&store, &observation)
                    .inspect_err(|error| tracing::warn!(%error, "R2 follow-up pass failed"))
                    .ok()
            },
        )
        .await
        .flatten()
    }
}

#[cfg(test)]
#[path = "attribution_followup_tests.rs"]
mod tests;
