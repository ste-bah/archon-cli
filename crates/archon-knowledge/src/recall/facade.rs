//! The merge. Quotas, deadlines, and an answer that is never all-or-nothing.
//!
//! # `recall` does not return `Result`
//!
//! That is the whole point of the slice. Four stores, one of which is a code
//! index that may not have been built and another of which may be waiting on an
//! embedding provider — a signature that can fail as a unit turns any one of
//! those into "no recall today". [`UnifiedRecall::recall`] therefore always
//! returns a [`RecallResponse`], and every source the policy allowed appears in
//! [`RecallResponse::sources`] with a named outcome. A source that failed, timed
//! out, panicked, or had no adapter registered is reported as such; the R7 gate
//! rolls back on "any source silently omitted", and the only way to satisfy that
//! is for omission to be impossible to express.
//!
//! # One detached thread per source
//!
//! Each source runs on its own thread and the merge stops waiting at that
//! source's own deadline, not at a shared one — a source with a 50 ms budget
//! cannot make a source with a 5 s budget wait 5 s to be declared late.
//!
//! Abandoning is not cancelling, and this is worth being plain about: the
//! thread keeps running its Cozo query to completion and its result is thrown
//! away when it tries to report. Cancellation is not available — the store APIs
//! are blocking and take no cancellation token — so the budget is enforced at
//! the only place it can be, which is the boundary where the caller waits.
//! Threads are not joined, so a slow store delays nothing but its own reply.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::errors::Result;
use crate::recall::identity::{self, RecallConflict};
use crate::recall::normalize::{self, UNCALIBRATED_METHOD};
use crate::recall::{RecallHit, RecallQuery, RecallSource, RecallSourceAdapter, ScoreCalibration};

/// What became of one source's contribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Answered inside its budget.
    Ok,
    /// The adapter returned an error. The message is the store's own.
    Failed { error: String },
    /// Did not answer inside its budget; its work was abandoned, not cancelled.
    LatencyBudgetExceeded { budget_ms: u128 },
    /// The adapter panicked. Reported rather than converted to "slow", because
    /// the two call for very different investigations.
    Panicked { payload: String },
    /// The policy allowed this source but no adapter is registered for it.
    NoAdapter,
    /// An adapter is registered but the query's policy excluded it.
    ExcludedByPolicy,
}

impl SourceStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, SourceStatus::Ok)
    }
}

/// Per-source accounting, one entry for every source the caller could expect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceOutcome {
    pub source: RecallSource,
    pub status: SourceStatus,
    /// Hits the adapter handed back, before the quota.
    pub returned: usize,
    /// Hits that survived the quota and entered the merge.
    pub kept: usize,
    /// Wall clock from the start of the whole recall, so it includes thread
    /// start-up and lock contention — what the caller actually waited.
    pub elapsed_ms: u128,
}

/// The merged answer, always partial-capable.
#[derive(Debug, Clone, Serialize)]
pub struct RecallResponse {
    pub hits: Vec<RecallHit>,
    /// Every disagreement found, reported in full even when the hit list was
    /// truncated — a conflict dropped by a `limit` is a conflict missed.
    pub conflicts: Vec<RecallConflict>,
    pub sources: Vec<SourceOutcome>,
    /// Calibration of every score in [`RecallResponse::hits`].
    pub calibration: ScoreCalibration,
    /// Plain-language restatement of the above, for output a human reads.
    pub calibration_note: String,
}

impl RecallResponse {
    /// Sources that did not contribute a clean answer.
    pub fn degraded_sources(&self) -> Vec<&SourceOutcome> {
        self.sources
            .iter()
            .filter(|outcome| !outcome.status.is_ok())
            .collect()
    }

    /// Whether any source failed to answer cleanly.
    pub fn is_partial(&self) -> bool {
        !self.degraded_sources().is_empty()
    }
}

/// The recall facade: register adapters, ask once, get everything.
#[derive(Default)]
pub struct UnifiedRecall {
    adapters: Vec<Arc<dyn RecallSourceAdapter>>,
}

impl UnifiedRecall {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one source. A later registration for the same source replaces
    /// the earlier one, so a caller assembling adapters conditionally cannot end
    /// up querying one store twice.
    pub fn with_source(mut self, adapter: Arc<dyn RecallSourceAdapter>) -> Self {
        let source = adapter.source();
        self.adapters.retain(|held| held.source() != source);
        self.adapters.push(adapter);
        self
    }

    pub fn registered_sources(&self) -> Vec<RecallSource> {
        let mut sources: Vec<RecallSource> = self
            .adapters
            .iter()
            .map(|adapter| adapter.source())
            .collect();
        sources.sort();
        sources
    }

    /// Ask every allowed source, merge, and account for all of them.
    pub fn recall(&self, query: &RecallQuery) -> RecallResponse {
        let start = Instant::now();
        let mut outcomes: Vec<SourceOutcome> = Vec::new();

        for adapter in &self.adapters {
            if !query.source_policy.allows(adapter.source()) {
                outcomes.push(idle_outcome(
                    adapter.source(),
                    SourceStatus::ExcludedByPolicy,
                ));
            }
        }
        for budget in query.source_policy.budgets() {
            if !self
                .adapters
                .iter()
                .any(|adapter| adapter.source() == budget.source)
            {
                outcomes.push(idle_outcome(budget.source, SourceStatus::NoAdapter));
            }
        }

        let (sender, receiver) = mpsc::channel::<Dispatch>();
        let mut pending: BTreeMap<RecallSource, Duration> = BTreeMap::new();
        for adapter in &self.adapters {
            let source = adapter.source();
            let Some(budget) = query.source_policy.budget_for(source) else {
                continue;
            };
            let sender = sender.clone();
            let adapter = Arc::clone(adapter);
            let query = query.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("recall-{source}"))
                .spawn(move || {
                    let hits = catch_unwind(AssertUnwindSafe(|| adapter.recall(&query)));
                    // The receiver is gone once the caller's deadline passed.
                    // Dropping the result here is the abandonment described in
                    // the module docs.
                    let _ = sender.send(Dispatch { source, hits });
                });
            match spawned {
                Ok(_) => {
                    pending.insert(source, budget.latency_budget);
                }
                Err(error) => outcomes.push(SourceOutcome {
                    source,
                    status: SourceStatus::Failed {
                        error: format!("could not start recall worker: {error}"),
                    },
                    returned: 0,
                    kept: 0,
                    elapsed_ms: start.elapsed().as_millis(),
                }),
            }
        }
        drop(sender);

        let mut hits: Vec<RecallHit> = Vec::new();
        collect(
            &receiver,
            start,
            query,
            &mut pending,
            &mut hits,
            &mut outcomes,
        );

        let mut merged = identity::merge(hits);
        merged.hits.truncate(query.limit);
        outcomes.sort_by_key(|outcome| outcome.source);

        RecallResponse {
            hits: merged.hits,
            conflicts: merged.conflicts,
            sources: outcomes,
            calibration: normalize::calibration(),
            calibration_note: UNCALIBRATED_METHOD.to_string(),
        }
    }
}

struct Dispatch {
    source: RecallSource,
    hits: std::thread::Result<Result<Vec<RecallHit>>>,
}

fn idle_outcome(source: RecallSource, status: SourceStatus) -> SourceOutcome {
    SourceOutcome {
        source,
        status,
        returned: 0,
        kept: 0,
        elapsed_ms: 0,
    }
}

/// Wait for the pending sources, expiring each at its own deadline.
fn collect(
    receiver: &mpsc::Receiver<Dispatch>,
    start: Instant,
    query: &RecallQuery,
    pending: &mut BTreeMap<RecallSource, Duration>,
    hits: &mut Vec<RecallHit>,
    outcomes: &mut Vec<SourceOutcome>,
) {
    while !pending.is_empty() {
        let elapsed = start.elapsed();
        let next = pending.values().copied().min().unwrap_or_default();
        if elapsed >= next {
            expire(pending, elapsed, outcomes);
            continue;
        }
        match receiver.recv_timeout(next - elapsed) {
            Ok(dispatch) => {
                // A reply that arrives after its own deadline has already been
                // accounted for as late; taking it now would make the budget
                // depend on scheduling luck.
                if pending.remove(&dispatch.source).is_none() {
                    continue;
                }
                outcomes.push(accept(dispatch, start.elapsed(), query, hits));
            }
            Err(RecvTimeoutError::Timeout) => expire(pending, start.elapsed(), outcomes),
            Err(RecvTimeoutError::Disconnected) => {
                let elapsed = start.elapsed().as_millis();
                for (source, _) in std::mem::take(pending) {
                    outcomes.push(SourceOutcome {
                        source,
                        status: SourceStatus::Failed {
                            error: "recall worker ended without reporting".into(),
                        },
                        returned: 0,
                        kept: 0,
                        elapsed_ms: elapsed,
                    });
                }
            }
        }
    }
}

fn expire(
    pending: &mut BTreeMap<RecallSource, Duration>,
    elapsed: Duration,
    outcomes: &mut Vec<SourceOutcome>,
) {
    let late: Vec<RecallSource> = pending
        .iter()
        .filter(|&(_, &budget)| elapsed >= budget)
        .map(|(&source, _)| source)
        .collect();
    for source in late {
        let budget = pending.remove(&source).unwrap_or_default();
        outcomes.push(SourceOutcome {
            source,
            status: SourceStatus::LatencyBudgetExceeded {
                budget_ms: budget.as_millis(),
            },
            returned: 0,
            kept: 0,
            elapsed_ms: elapsed.as_millis(),
        });
    }
}

/// Turn one source's reply into an outcome, applying its quota.
fn accept(
    dispatch: Dispatch,
    elapsed: Duration,
    query: &RecallQuery,
    hits: &mut Vec<RecallHit>,
) -> SourceOutcome {
    let source = dispatch.source;
    let elapsed_ms = elapsed.as_millis();
    match dispatch.hits {
        Ok(Ok(mut returned)) => {
            let total = returned.len();
            // Rank order is the source's own judgement, so the quota keeps its
            // best rather than whatever order it happened to build the vector
            // in. `source_id` breaks ties so a store that ranks two hits equally
            // still truncates the same way on every replay.
            returned.sort_by(|a, b| {
                a.source_rank
                    .cmp(&b.source_rank)
                    .then_with(|| a.source_id.cmp(&b.source_id))
            });
            returned.truncate(query.quota_for(source));
            // Re-derive the score from rank rather than trusting the adapter:
            // an adapter that invented its own number would be smuggling in an
            // uncorroborated calibration.
            for hit in &mut returned {
                hit.normalized_score = normalize::uncalibrated_rank_score(hit.source_rank);
                hit.calibration = normalize::calibration();
            }
            let kept = returned.len();
            hits.append(&mut returned);
            SourceOutcome {
                source,
                status: SourceStatus::Ok,
                returned: total,
                kept,
                elapsed_ms,
            }
        }
        Ok(Err(error)) => SourceOutcome {
            source,
            status: SourceStatus::Failed {
                error: error.to_string(),
            },
            returned: 0,
            kept: 0,
            elapsed_ms,
        },
        Err(payload) => SourceOutcome {
            source,
            status: SourceStatus::Panicked {
                payload: panic_message(payload.as_ref()),
            },
            returned: 0,
            kept: 0,
            elapsed_ms,
        },
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

#[cfg(test)]
#[path = "facade/tests.rs"]
mod tests;
