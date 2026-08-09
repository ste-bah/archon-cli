use std::time::Instant;

use super::*;
use crate::errors::KnowledgeError;
use crate::recall::{SourceBudget, SourcePolicy};

/// An adapter that answers immediately with the hits it was handed.
struct Fixed {
    source: RecallSource,
    hits: Vec<RecallHit>,
}

impl RecallSourceAdapter for Fixed {
    fn source(&self) -> RecallSource {
        self.source
    }
    fn recall(&self, _query: &RecallQuery) -> Result<Vec<RecallHit>> {
        Ok(self.hits.clone())
    }
}

struct Failing {
    source: RecallSource,
    message: &'static str,
}

impl RecallSourceAdapter for Failing {
    fn source(&self) -> RecallSource {
        self.source
    }
    fn recall(&self, _query: &RecallQuery) -> Result<Vec<RecallHit>> {
        Err(KnowledgeError::Store(self.message.into()))
    }
}

struct Slow {
    source: RecallSource,
    delay: Duration,
}

impl RecallSourceAdapter for Slow {
    fn source(&self) -> RecallSource {
        self.source
    }
    fn recall(&self, _query: &RecallQuery) -> Result<Vec<RecallHit>> {
        std::thread::sleep(self.delay);
        Ok(vec![RecallHit::at_rank(self.source, "late", "late", 0)])
    }
}

struct Panicking {
    source: RecallSource,
}

impl RecallSourceAdapter for Panicking {
    fn source(&self) -> RecallSource {
        self.source
    }
    fn recall(&self, _query: &RecallQuery) -> Result<Vec<RecallHit>> {
        panic!("adapter blew up")
    }
}

fn fixed(source: RecallSource, count: usize) -> Arc<dyn RecallSourceAdapter> {
    Arc::new(Fixed {
        source,
        hits: (0..count)
            .map(|rank| {
                RecallHit::at_rank(
                    source,
                    format!("{source}-{rank}"),
                    format!("{source} result {rank}"),
                    rank,
                )
            })
            .collect(),
    })
}

fn outcome(response: &RecallResponse, source: RecallSource) -> &SourceOutcome {
    response
        .sources
        .iter()
        .find(|outcome| outcome.source == source)
        .unwrap_or_else(|| panic!("{source} was omitted from the accounting"))
}

fn policy(sources: &[(RecallSource, usize, Duration)]) -> SourcePolicy {
    SourcePolicy::from_budgets(
        sources
            .iter()
            .map(|&(source, quota, latency_budget)| SourceBudget {
                source,
                quota,
                latency_budget,
            })
            .collect(),
    )
}

fn query(sources: &[(RecallSource, usize, Duration)], limit: usize) -> RecallQuery {
    RecallQuery {
        text: "anything".into(),
        limit,
        source_policy: policy(sources),
    }
}

#[test]
fn quota_truncates_an_over_eager_source() {
    let response = UnifiedRecall::new()
        .with_source(fixed(RecallSource::Docs, 10))
        .with_source(fixed(RecallSource::Memory, 10))
        .recall(&query(
            &[
                (RecallSource::Docs, 2, Duration::from_secs(5)),
                (RecallSource::Memory, 3, Duration::from_secs(5)),
            ],
            50,
        ));

    assert_eq!(outcome(&response, RecallSource::Docs).returned, 10);
    assert_eq!(outcome(&response, RecallSource::Docs).kept, 2);
    assert_eq!(outcome(&response, RecallSource::Memory).kept, 3);
    assert_eq!(response.hits.len(), 5);
    // The quota keeps the source's own best, not an arbitrary five.
    let docs: Vec<&str> = response
        .hits
        .iter()
        .filter(|hit| hit.source == RecallSource::Docs)
        .map(|hit| hit.source_id.as_str())
        .collect();
    assert_eq!(docs, vec!["docs-0", "docs-1"]);
}

#[test]
fn one_failing_source_yields_partial_results_and_a_named_error() {
    let response = UnifiedRecall::new()
        .with_source(fixed(RecallSource::Docs, 3))
        .with_source(Arc::new(Failing {
            source: RecallSource::Code,
            message: "no code index at /nowhere",
        }))
        .recall(&query(
            &[
                (RecallSource::Docs, 5, Duration::from_secs(5)),
                (RecallSource::Code, 5, Duration::from_secs(5)),
            ],
            10,
        ));

    assert_eq!(response.hits.len(), 3, "the healthy source still answered");
    assert!(response.is_partial());
    match &outcome(&response, RecallSource::Code).status {
        SourceStatus::Failed { error } => assert!(
            error.contains("no code index at /nowhere"),
            "store's own message was lost: {error}"
        ),
        other => panic!("expected a named failure, got {other:?}"),
    }
    assert!(outcome(&response, RecallSource::Docs).status.is_ok());
}

#[test]
fn a_slow_source_does_not_hold_up_the_rest() {
    let started = Instant::now();
    let response = UnifiedRecall::new()
        .with_source(fixed(RecallSource::Docs, 2))
        .with_source(Arc::new(Slow {
            source: RecallSource::Memory,
            delay: Duration::from_secs(30),
        }))
        .recall(&query(
            &[
                (RecallSource::Docs, 5, Duration::from_secs(30)),
                (RecallSource::Memory, 5, Duration::from_millis(20)),
            ],
            10,
        ));
    let elapsed = started.elapsed();

    // The slow source's own 20ms budget bounds the wait, even though the other
    // source is allowed 30s. Generous ceiling: this asserts "did not wait for
    // the sleeper", not a performance figure.
    assert!(
        elapsed < Duration::from_secs(5),
        "waited {elapsed:?} for a source with a 20ms budget"
    );
    assert_eq!(response.hits.len(), 2);
    match outcome(&response, RecallSource::Memory).status {
        SourceStatus::LatencyBudgetExceeded { budget_ms } => assert_eq!(budget_ms, 20),
        ref other => panic!("expected a budget breach, got {other:?}"),
    }
}

#[test]
fn a_panicking_adapter_is_reported_as_a_panic_not_as_slowness() {
    // The default panic hook still prints to stderr from the worker thread;
    // that noise is expected in this test's output.
    let response = UnifiedRecall::new()
        .with_source(Arc::new(Panicking {
            source: RecallSource::Knowledge,
        }))
        .with_source(fixed(RecallSource::Docs, 1))
        .recall(&query(
            &[
                (RecallSource::Knowledge, 5, Duration::from_secs(5)),
                (RecallSource::Docs, 5, Duration::from_secs(5)),
            ],
            10,
        ));

    match &outcome(&response, RecallSource::Knowledge).status {
        SourceStatus::Panicked { payload } => assert!(payload.contains("adapter blew up")),
        other => panic!("expected a panic outcome, got {other:?}"),
    }
    assert_eq!(response.hits.len(), 1);
}

/// The R7 gate rolls back on "any source silently omitted", so a policy that
/// names a source with no adapter must say so rather than shrink the answer.
#[test]
fn a_source_with_no_adapter_is_accounted_for() {
    let response = UnifiedRecall::new()
        .with_source(fixed(RecallSource::Docs, 1))
        .recall(&query(
            &[
                (RecallSource::Docs, 5, Duration::from_secs(5)),
                (RecallSource::Code, 5, Duration::from_secs(5)),
            ],
            10,
        ));

    assert_eq!(
        outcome(&response, RecallSource::Code).status,
        SourceStatus::NoAdapter
    );
    assert!(response.is_partial());
}

#[test]
fn a_registered_source_the_policy_excludes_is_accounted_for() {
    let response = UnifiedRecall::new()
        .with_source(fixed(RecallSource::Docs, 1))
        .with_source(fixed(RecallSource::Memory, 1))
        .recall(&query(
            &[(RecallSource::Docs, 5, Duration::from_secs(5))],
            10,
        ));

    assert_eq!(
        outcome(&response, RecallSource::Memory).status,
        SourceStatus::ExcludedByPolicy
    );
    assert_eq!(response.hits.len(), 1);
}

#[test]
fn registering_a_source_twice_queries_it_once() {
    let recall = UnifiedRecall::new()
        .with_source(fixed(RecallSource::Docs, 1))
        .with_source(fixed(RecallSource::Docs, 4));
    assert_eq!(recall.registered_sources(), vec![RecallSource::Docs]);

    let response = recall.recall(&query(
        &[(RecallSource::Docs, 9, Duration::from_secs(5))],
        10,
    ));
    assert_eq!(outcome(&response, RecallSource::Docs).returned, 4);
}

/// An adapter cannot smuggle in a score of its own: the facade re-derives every
/// score from the rank the source reported.
#[test]
fn adapter_supplied_scores_are_overwritten() {
    let mut hit = RecallHit::at_rank(RecallSource::Docs, "d0", "text", 5);
    hit.normalized_score = 99.0;
    hit.calibration = ScoreCalibration::Measured {
        corpus_id: "invented".into(),
        corpus_version: "0".into(),
    };

    let response = UnifiedRecall::new()
        .with_source(Arc::new(Fixed {
            source: RecallSource::Docs,
            hits: vec![hit],
        }))
        .recall(&query(
            &[(RecallSource::Docs, 5, Duration::from_secs(5))],
            10,
        ));

    assert_eq!(response.hits.len(), 1);
    assert_eq!(
        response.hits[0].normalized_score,
        normalize::uncalibrated_rank_score(5)
    );
    assert_eq!(
        response.hits[0].calibration,
        ScoreCalibration::UncalibratedRankOrder
    );
    assert!(!response.calibration.is_measured());
    assert!(response.calibration_note.contains("uncalibrated"));
}

#[test]
fn limit_truncates_the_merged_answer_but_not_the_conflicts() {
    let disputed = |source: RecallSource, id: &str, content: &str| {
        RecallHit::at_rank(source, id, content, 0).with_provenance(["chunk:shared".to_string()])
    };

    let response = UnifiedRecall::new()
        .with_source(Arc::new(Fixed {
            source: RecallSource::Docs,
            hits: vec![disputed(
                RecallSource::Docs,
                "d0",
                "Retention is thirty days.",
            )],
        }))
        .with_source(Arc::new(Fixed {
            source: RecallSource::Knowledge,
            hits: vec![disputed(
                RecallSource::Knowledge,
                "k0",
                "Retention is ninety days.",
            )],
        }))
        .recall(&query(
            &[
                (RecallSource::Docs, 5, Duration::from_secs(5)),
                (RecallSource::Knowledge, 5, Duration::from_secs(5)),
            ],
            1,
        ));

    assert_eq!(response.hits.len(), 1, "limit was not applied");
    assert_eq!(
        response.conflicts.len(),
        1,
        "a conflict must survive truncation of the hit list"
    );
}
