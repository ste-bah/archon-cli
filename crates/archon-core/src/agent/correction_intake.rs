//! One writer for corrections, two detectors feeding it.
//!
//! Corrections used to be written from two places that did not know about each
//! other: this keyword detector, firing every turn, and the periodic LLM
//! extractor, which also emitted `correction` memories. The same correction
//! therefore landed twice in different words -- the "exact-text twins" a
//! diagnostic run surfaced.
//!
//! Collapsing to one writer would normally mean losing whatever the other
//! caught. The keyword matcher is immediate and free but only recognises the
//! phrasings listed in [`classify_correction`]; "that's not what I meant" slips
//! straight through. The extractor is semantic and catches those, but runs only
//! every few turns.
//!
//! So neither owns detection and both feed one writer. The keyword pass records
//! immediately, which is what matters -- a correction exists to stop the NEXT
//! turn repeating the mistake, and a record that arrives five turns later has
//! already missed its purpose. The extractor then reports only corrections the
//! keyword pass did not already capture, and those are recorded through the
//! same path. Late beats never; duplicated beats neither.

use archon_consciousness::correction_classifier::{
    CorrectionClassification, CorrectionClassifier, explicit_phrase_match,
};
use archon_consciousness::corrections::{CorrectionTracker, CorrectionType};
use archon_memory::MemoryTrait;
use archon_memory::types::MemoryType;
use std::sync::Arc;

use super::support::stored_correction_content;

/// Classify `user_input` as a correction, if its phrasing matches a known form.
///
/// Shared so both detectors agree on the taxonomy. `None` means "no keyword
/// match", which is a statement about this function's patterns and not about
/// whether the user corrected anything -- the semantic pass decides that.
///
/// The table itself now lives in `archon-consciousness` alongside the taxonomy
/// and the R3 classifier that wraps it. One table, so the live mutating path
/// and the shadow classifier cannot answer differently for the same phrasing --
/// which is what makes the classifier's explicit-case recall measurable at all.
pub(super) fn classify_correction(user_input: &str) -> Option<CorrectionType> {
    explicit_phrase_match(user_input)
}

/// The R3 classifier as it runs on the live path: default config, no provider.
///
/// Constructed per call rather than held on the `Agent`. It owns no state, and
/// a field would be one more thing every `Agent` constructor has to remember to
/// populate for a shadow observation that must never change behaviour anyway.
///
/// The provider arm is off here and stays off until the promotion gate at
/// `docs/development/learning-roadmap-r1-r8-w5-w6.md:300` passes.
pub(super) fn shadow_classify(user_input: &str) -> CorrectionClassification {
    CorrectionClassifier::default().classify(user_input)
}

/// Record corrections the semantic pass found and the keyword pass missed.
///
/// Called from the extractor's background task with the items it typed
/// `correction`. They go through [`CorrectionTracker`] rather than
/// `store_extracted` so that there is exactly one writer of correction content,
/// and so these inherit the same rule linking and scoring as the fast path.
///
/// Returns how many were recorded.
pub(super) fn record_extracted_corrections(
    graph: &Arc<dyn MemoryTrait>,
    corrections: &[archon_memory::extraction::ExtractedMemory],
    context: &str,
) -> usize {
    let tracker = CorrectionTracker::new(graph.as_ref());
    let mut recorded = 0usize;

    for item in corrections {
        if item.memory_type != MemoryType::Correction {
            continue;
        }
        // The extractor supplies no taxonomy. Re-run the keyword classifier over
        // its restatement -- which is likelier to match than the raw turn was,
        // since it is written as a plain instruction -- and fall back to the
        // general bucket rather than dropping a correction over a missing label.
        let correction_type =
            classify_correction(&item.content).unwrap_or(CorrectionType::ApproachCorrection);
        // Bounded on this path too. It is a different writer reaching the same
        // relation, and the reason corrections needed bounding at all was a
        // writer that trusted its input.
        let content = stored_correction_content(&item.content);
        // Recorded without reinforcement, like the fast path. Unlike the fast
        // path there is no attribution to follow: the semantic pass runs several
        // turns late, against an action window that has already moved, so there
        // is nothing left to attribute the correction to. That makes every
        // extractor-found correction permanently unattributed, and an
        // unattributed correction reinforces nothing. The correction is still
        // recorded and still recalled -- what it no longer does is move a rule's
        // score on the strength of a restatement nobody could explain.
        match tracker.record_correction_unreinforced(correction_type, &content, context, None) {
            Ok(_) => recorded += 1,
            Err(error) => {
                tracing::warn!(%error, "recording an extractor-found correction failed")
            }
        }
    }

    recorded
}

// ── R3 shadow labels ─────────────────────────────────────────
//
// Everything below observes; nothing below mutates. The live path keeps
// recording corrections from `classify_correction`, and the classifier's
// verdict is written next to it as evidence. Promotion needs 400 adjudicated
// examples (learning roadmap line 300), and none of them exist until something
// starts writing them down -- which is what this is.

/// Metric name carried by every shadow label row.
const SHADOW_LABEL_METRIC: &str = "correction_classifier_shadow_label";

/// Marks these labels as the pre-change heuristic baseline rather than
/// adjudicated ground truth.
///
/// The roadmap is explicit that heuristic labels are migration-only and rank
/// below adjudication (line 230). Recording that in the row means a later
/// adjudication pass can supersede these without having to guess where they
/// came from.
const SHADOW_LABEL_SOURCE: &str = "live_heuristic_shadow";

/// One turn's worth of "what did each detector decide".
pub(super) struct ShadowCorrectionLabel {
    pub session_id: String,
    pub turn_number: u64,
    /// Cognitive situation kind, when one was classified for this turn. The
    /// roadmap forbids aggregate-only trends, and this is the task-class axis.
    pub task_class: String,
    pub model_id: String,
    /// The classifier's verdict.
    pub classification: CorrectionClassification,
    /// The live heuristic's verdict -- the thing that actually mutated rules.
    pub heuristic: Option<CorrectionType>,
    /// Id of the correction the heuristic recorded, when it recorded one.
    pub correction_id: Option<String>,
    /// SHA-256 of the user turn. The corpus needs to join rows to turns and to
    /// deduplicate them; it does not need the user's text, and an append-only
    /// measurement log is the last place that should hold a copy of it.
    pub user_input_hash: String,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

impl ShadowCorrectionLabel {
    fn label_agreement(&self) -> &'static str {
        // An abstention is not a disagreement: the classifier declined to
        // answer, so there is nothing to agree or disagree with.
        if self.classification.abstained() {
            return "undefined";
        }
        if self.classification.is_correction == self.heuristic.is_some() {
            "true"
        } else {
            "false"
        }
    }

    /// Deterministic identity, so a retried write is recognised as a replay
    /// rather than counted twice.
    fn metric_event_id(&self) -> String {
        format!("correction-shadow:{}:{}", self.session_id, self.turn_number)
    }

    fn evaluation_window(&self) -> archon_cognitive::metrics::EvaluationWindow {
        // A UTC day. Windows are immutable once declared, so the definition has
        // to be a pure function of the date -- a window derived from "now"
        // would be redeclared with different bounds on the next turn and
        // rejected.
        let day = self.observed_at.date_naive();
        let started_at = day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
        let ended_at = started_at + chrono::Duration::days(1);
        archon_cognitive::metrics::EvaluationWindow::new(
            format!("correction-shadow-{day}"),
            started_at,
            ended_at,
        )
    }

    fn event(&self) -> archon_cognitive::metrics::CognitiveMetricEvent {
        use archon_cognitive::metrics::{CognitiveMetricEvent, MetricCohort, MetricEventKind};

        let window = self.evaluation_window();
        let classifier_version =
            archon_consciousness::correction_classifier::CORRECTION_CLASSIFIER_VERSION;
        let mut event = CognitiveMetricEvent::new(
            self.metric_event_id(),
            SHADOW_LABEL_METRIC,
            MetricEventKind::CorrectionClassified,
            window.evaluation_window_id,
            // Policy version is the classifier version: a threshold or arm
            // change must not pool with rows measured under the old one.
            MetricCohort::new(
                self.task_class.clone(),
                self.model_id.clone(),
                classifier_version,
            ),
            self.observed_at,
        )
        .with_session(self.session_id.clone(), self.turn_number)
        .with_value(f64::from(self.classification.confidence))
        // Not a verified outcome. This row says what two detectors thought,
        // not whether either was right; adjudication supplies that later.
        .with_outcome("shadow")
        // A turn where nothing was recorded still needs a typed source id, or
        // the non-correction half of the corpus cannot be joined back to the
        // turn it came from.
        .with_identity(
            "correction_id",
            self.correction_id
                .clone()
                .unwrap_or_else(|| format!("shadow:{}:{}", self.session_id, self.turn_number)),
        )
        .with_identity("predicted_label", self.classification.predicted_label())
        .with_identity(
            "ground_truth_label",
            if self.heuristic.is_some() {
                "correction"
            } else {
                "not_correction"
            },
        )
        .with_identity("abstained", bool_identity(self.classification.abstained()))
        .with_identity("agreement", self.label_agreement())
        .with_identity("rationale_code", self.classification.rationale_code.clone())
        .with_identity("classifier_version", classifier_version)
        .with_identity(
            "predicted_correction_type",
            self.classification
                .correction_type
                .map_or("none", CorrectionType::as_code),
        )
        .with_identity(
            "heuristic_correction_type",
            self.heuristic.map_or("none", CorrectionType::as_code),
        )
        // The audit finding this work answers is that a misfire becomes a
        // permanent rule. Recording which detector was allowed to mutate makes
        // "the classifier changed nothing" checkable from the rows themselves.
        .with_identity("mutation_source", "heuristic")
        .with_identity("user_input_hash", self.user_input_hash.clone());
        // Set directly rather than through the builder, which has no setter for
        // it. Load-bearing rather than decorative: it is what marks these rows
        // as the heuristic baseline so an adjudication pass can outrank them.
        event.label_source = SHADOW_LABEL_SOURCE.to_string();
        event
    }
}

fn bool_identity(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// SHA-256 of a user turn, hex encoded.
pub(super) fn user_input_hash(user_input: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(user_input.as_bytes()))
}

/// Append one shadow label to the cognitive metric substrate.
///
/// Writes through `archon-cognitive`'s public [`MetricEventStore`] rather than
/// a private ledger of our own, so the same `correction_classified` rows the
/// R8 derivations already know how to read are the ones that accumulate.
///
/// Takes an open store rather than a path: the agent already holds one, and
/// opening a second handle on the same SQLite file per turn would be a cost
/// paid by a measurement that must not slow the turn down.
///
/// [`MetricEventStore`]: archon_cognitive::metrics::MetricEventStore
pub(super) fn record_shadow_correction_label(
    store: &archon_cognitive::PersistentCognitiveStore,
    label: &ShadowCorrectionLabel,
) -> Result<archon_cognitive::metrics::MetricWriteOutcome, archon_cognitive::CognitiveError> {
    let event_store = archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())?;
    // Idempotent for an identical definition, so every turn can assert the
    // window it is about to write into rather than depending on start-up order.
    event_store.declare_window(&label.evaluation_window())?;
    event_store.record(&label.event())
}

#[cfg(test)]
#[path = "correction_intake_tests.rs"]
mod tests;
