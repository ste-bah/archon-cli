//! Recording a correction and reinforcing the rule it implies are two acts.
//!
//! They used to be one call. That is the shape the R2 roadmap slice forbids:
//! "reinforce or propose a rule only after attribution; never infer ownership
//! from lexical similarity alone". Under the joined version, a correction the
//! system could not explain still raised a rule's score — the score went up
//! because a phrase matched, not because anything was shown to have caused the
//! complaint. Every such boost is a vote cast by a detector, counted as if it
//! were evidence.
//!
//! So the write is split at its natural seam. [`CorrectionTracker::record_claim`]
//! stores the correction row, resolves the rule, and creates the `CausedBy`
//! edge; none of that changes a score. The score change is
//! [`CorrectionTracker::reinforce_from_correction`], which a caller invokes only
//! once it has something to justify it with.
//!
//! Two properties make the deferral safe rather than merely later:
//!
//! * The boost's idempotency key is the correction id, so a reinforcement
//!   applied minutes after the record is still exactly-once. This is the same
//!   key that already made a retried `record_correction_with_id` safe.
//! * A deferred reinforcement has no rollback window. `record_claim`'s
//!   compensation depends on knowing that THIS call claimed the row, and that
//!   knowledge expires when the call returns. So the deferred path cannot undo
//!   the correction if the boost fails — which is correct: the correction
//!   happened and is worth keeping whether or not the score moved.
//!
//! [`CorrectionTracker::record_correction_with_id`] keeps the joined behaviour
//! by calling both halves in order, so the existing callers, tests and the
//! compensation semantics they pin are unchanged.

use super::{Correction, CorrectionError, CorrectionTracker, CorrectionType};

impl CorrectionTracker<'_> {
    /// Record a correction without touching any rule score.
    ///
    /// The correction row, the derived-or-explicit rule, and the
    /// `Correction -> CausedBy -> Rule` edge are all written. What is withheld
    /// is the reinforcement, which the caller applies through
    /// [`Self::reinforce_from_correction`] once it can justify it.
    pub fn record_correction_unreinforced(
        &self,
        correction_type: CorrectionType,
        content: &str,
        context: &str,
        rule_id: Option<&str>,
    ) -> Result<Correction, CorrectionError> {
        self.record_correction_unreinforced_with_id(
            &uuid::Uuid::new_v4().to_string(),
            correction_type,
            content,
            context,
            rule_id,
        )
    }

    /// [`Self::record_correction_unreinforced`] with a caller-stable id.
    pub fn record_correction_unreinforced_with_id(
        &self,
        correction_id: &str,
        correction_type: CorrectionType,
        content: &str,
        context: &str,
        rule_id: Option<&str>,
    ) -> Result<Correction, CorrectionError> {
        self.record_claim(correction_id, correction_type, content, context, rule_id)
            .map(|(correction, _newly_claimed)| correction)
    }

    /// Apply the rule reinforcement this correction implies.
    ///
    /// Separate from recording so the caller decides whether it is warranted.
    /// Exactly-once for a given correction: the correction id is the
    /// idempotency key, so calling this twice for one correction raises the
    /// score once.
    ///
    /// Returns `Ok(false)` when the correction carries no rule to reinforce,
    /// which is a real outcome rather than an error — a correction recorded
    /// against no rule is still a correction.
    pub fn reinforce_from_correction(
        &self,
        correction: &Correction,
    ) -> Result<bool, CorrectionError> {
        let Some(rule_id) = correction.rule_id.as_deref() else {
            return Ok(false);
        };
        self.reinforce_once(rule_id, correction.severity, &correction.id)?;
        Ok(true)
    }

    /// The boost, with the retry and provenance check the joined path uses.
    ///
    /// Retried once, then reconciled against the stored provenance: a boost
    /// whose response was lost has already been applied, and re-applying it
    /// would double-count the correction.
    fn reinforce_once(
        &self,
        rule_id: &str,
        severity: f64,
        correction_id: &str,
    ) -> Result<(), CorrectionError> {
        if let Err(first_error) = self.boost_rule(rule_id, severity, correction_id)
            && let Err(retry_error) = self.boost_rule(rule_id, severity, correction_id)
        {
            return match self
                .graph
                .has_importance_application(rule_id, correction_id)
            {
                Ok(true) => Ok(()),
                Ok(false) => Err(CorrectionError::Memory(
                    archon_memory::MemoryError::Database(format!(
                        "initial boost failed: {first_error}; retry failed: {retry_error}"
                    )),
                )),
                Err(status_error) => Err(CorrectionError::BoostOutcomeUnknown(format!(
                    "initial boost failed: {first_error}; retry failed: {retry_error}; \
                     provenance status read failed: {status_error}"
                ))),
            };
        }
        Ok(())
    }

    /// The joined path's boost: [`Self::reinforce_once`] plus the rollback that
    /// only a caller still holding the claim token can perform.
    pub(super) fn boost_with_compensation(
        &self,
        correction: &Correction,
        newly_claimed: bool,
    ) -> Result<(), CorrectionError> {
        let Some(rule_id) = correction.rule_id.as_deref() else {
            return Ok(());
        };
        match self.reinforce_once(rule_id, correction.severity, &correction.id) {
            Ok(()) => Ok(()),
            // `BoostOutcomeUnknown` deliberately keeps the row: the outcome is
            // unknown, so deleting the correction could discard a boost that
            // did land.
            Err(error @ CorrectionError::BoostOutcomeUnknown(_)) => Err(error),
            Err(cause) => {
                Err(self.compensate_new_claim_failure(cause, &correction.id, newly_claimed))
            }
        }
    }
}
