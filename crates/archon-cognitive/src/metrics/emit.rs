//! Runtime-side emission helper for [`CognitiveMetricEvent`].
//!
//! The store is deliberately strict (mandatory window, mandatory cohort,
//! per-kind identities, idempotency). That strictness is what makes the events
//! evidence, but it also means every writer would otherwise re-derive the same
//! three values. This puts that in one place so a writer emits a metric or
//! fails loudly, rather than skipping emission because the boilerplate was
//! inconvenient.

use std::path::Path;

use archon_policy::CognitivePolicy;
use chrono::{DateTime, Utc};
use cozo::DbInstance;

use crate::CognitiveError;
use crate::metrics::event::{CognitiveMetricEvent, MetricEventKind};
use crate::metrics::event_store::{MetricEventStore, MetricWriteOutcome};
use crate::metrics::window::MetricCohort;

/// Window id used while no operator has declared one.
///
/// `evaluation_window_id` is mandatory and an empty one is rejected, so events
/// written before the first declared window need a name rather than a blank.
/// Derivation with no window still admits them, and once a real window is
/// declared its own id takes over — these stay attributable to the period
/// before measurement was framed.
pub const UNWINDOWED_EVALUATION_WINDOW: &str = "unwindowed";

/// Length of the hashed policy identity in [`policy_version`].
///
/// Long enough that two policies in one repository will not collide, short
/// enough to read in a cohort key.
const POLICY_VERSION_HEX_LEN: usize = 12;

pub struct MetricEmitter<'a> {
    store: MetricEventStore<'a>,
    evaluation_window_id: String,
    cohort: MetricCohort,
}

impl<'a> MetricEmitter<'a> {
    pub fn open(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<Path>,
        cohort: MetricCohort,
    ) -> Result<Self, CognitiveError> {
        let store = MetricEventStore::new(db, ledger_dir)?;
        let evaluation_window_id = store
            .latest_window()?
            .map(|window| window.evaluation_window_id)
            .unwrap_or_else(|| UNWINDOWED_EVALUATION_WINDOW.to_string());
        Ok(Self {
            store,
            evaluation_window_id,
            cohort,
        })
    }

    pub fn evaluation_window_id(&self) -> &str {
        &self.evaluation_window_id
    }

    /// Start an event whose id is derived from `subject` rather than random.
    ///
    /// A retried join must reach the same id so the store recognises it as a
    /// replay and refuses to add a second row for one observation.
    pub fn event(
        &self,
        metric_name: &str,
        event_kind: MetricEventKind,
        subject: &str,
        created_at: DateTime<Utc>,
    ) -> CognitiveMetricEvent {
        CognitiveMetricEvent::new(
            format!("{metric_name}:{subject}"),
            metric_name,
            event_kind,
            self.evaluation_window_id.clone(),
            self.cohort.clone(),
            created_at,
        )
    }

    pub fn record(
        &self,
        event: &CognitiveMetricEvent,
    ) -> Result<MetricWriteOutcome, CognitiveError> {
        self.store.record(event)
    }
}

/// Content-addressed identity for the policy in force.
///
/// The cohort must distinguish measurements taken under different policies, and
/// `CognitivePolicy` carries no version field. Hashing its serialised form
/// means any edit that could change behaviour also changes the cohort, which is
/// the property the segmentation actually needs.
pub fn policy_version(policy: Option<&CognitivePolicy>) -> String {
    let Some(policy) = policy else {
        return "no_policy".to_string();
    };
    let Ok(canonical) = serde_json::to_vec(policy) else {
        // Unreachable for a plain derived struct, but a fabricated cohort would
        // silently merge two policies, so say the identity is unknown instead.
        return "unserializable_policy".to_string();
    };
    let digest = blake3::hash(&canonical).to_hex().to_string();
    format!("cog-{}", &digest[..POLICY_VERSION_HEX_LEN])
}

/// Cohort for a runtime observation.
///
/// `task_class` is the situation kind: the roadmap forbids aggregate-only
/// trends, and situation kind is the only task segmentation the cognitive layer
/// actually knows at write time.
pub fn runtime_cohort(
    task_class: &str,
    model_id: &str,
    policy: Option<&CognitivePolicy>,
) -> MetricCohort {
    MetricCohort::new(
        non_empty(task_class, "unknown_task_class"),
        non_empty(model_id, "unknown_model"),
        policy_version(policy),
    )
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}
