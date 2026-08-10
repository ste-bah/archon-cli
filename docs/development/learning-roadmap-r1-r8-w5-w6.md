# Learning Roadmap: R1–R8 and W5–W6

## Audit verdict

All ten items remain roadmap work. Existing components are substantial, but none forms a closed, measured live learning loop.

| Item | Status | Current reality | Missing boundary |
|---|---|---|---|
| R1 | Partially confirmed | `ExecutiveLoop` classifies, plans, scores, records, verifies, and reflects | Main-agent shadow invocation and live-outcome comparison |
| R2 | Partially confirmed | Corrections select lexically relevant rules and emit learning evidence | Decision/tool causal attribution and structured lesson provenance |
| R3 | Confirmed | Production correction detection remains phrase heuristics | Confidence-scored classifier, abstention, and labeled evaluation |
| R4 | Partially confirmed | Memory Garden prunes, decays, deduplicates, and merges | Governed generative consolidation, retirement, and nightly scheduling |
| R5 | Partially confirmed | `SelfModelStore` and persisted reasoning-quality self-trust evidence exist | Verified aggregate updates, bounded drift, and live briefing/action use |
| R6 | Partially confirmed | Reflection records, writers, and governed proposal plumbing exist | Runtime struggle/surprise triggers, compact private reflection, and measured reuse |
| R7 | Confirmed | Memory, docs, knowledge, and LEANN expose separate retrieval APIs | One provenance-normalized, score-calibrated retrieval facade |
| R8 | Partially confirmed | Cognitive inspection exposes counts and recent rows | Per-session intelligence metrics, baselines, trends, and release gates |
| W5 | Partially confirmed | Verification outcomes and deterministic label mapping exist | Join verified outcomes to training traces; stop treating unverified completion as success |
| W6 | Partially confirmed | Latent surprise is computed and persisted | Replay prioritization, reflection trigger, and runtime metrics |

Status vocabulary follows `docs/reports/audit/prd-008-audit-validation-appendix.md`.

## Global constraints

- R8 instrumentation ships with the first task in every slice, before behavior changes.
- R1 remains shadow-only until measured promotion gates pass.
- No hidden chain-of-thought is stored. Persist compact decisions, evidence, outcomes, and lessons.
- Deterministic evidence outranks model judgment: test/build/verification outcomes define success labels.
- Classifiers must support abstention. Low-confidence output produces no correction or rule mutation.
- Learning writes remain governed through proposal, approval, application, and rollback.
- Telemetry and shadow-analysis failures may fail open for foreground execution, but must record degradation evidence.
- Action selection, policy enforcement, state mutation, and approval-path failures never fail open: defer to the existing approved execution path or fail closed.
- Every new metric/correlation record carries typed source IDs, `session_id`, `turn_number`, model/policy version, timestamp, and idempotency key.
- Every metric is segmented by task class and model/policy version; aggregate-only trends are insufficient.

## R0 entry gate

No behavior-changing roadmap slice may be promoted until findings 9, 11, 17, and 40–43 have verified closure evidence. Finding 9 is now closed in physical source: memory keyword retrieval queries the `content_fts`, `title_fts`, and `tags_fts` indexes first, with a full scan retained only as an explicit fallback when those indexes are unavailable. Re-evaluate findings 11, 17, and 40–43 from physical source before promotion; Slice 1 may implement metrics and run the existing heuristic in shadow mode while any remaining R0 item closes, but it may not mutate rules through the new classifier.

Record R0 closure as immutable evidence references to the relevant test runs, runtime source-of-truth inspections, reviews, and commits. Re-evaluate this gate immediately before each slice starts; do not infer closure from this roadmap.

That record is `docs/development/r0-entry-gate.evidence`. It names, per finding, the pushed closure commit, the source anchors that must remain present, the defect signatures that must remain absent, the live call site, and the behavioural and source-of-truth tests. `scripts/check-r0-entry-gate.sh` re-verifies every one of those statements and prints an explicit R0 PASS/FAIL; it runs as the `r0-entry-gate` job in `.github/workflows/ci.yml` (with `fetch-depth: 0` and `--require-commits`, so commit ancestry is actually checked) and as step 2b of `scripts/ci-gate.sh`. Re-evaluating the gate before a slice starts means running that script, not re-reading this paragraph. As of 2026-08-10 it reports PASS for findings 9, 11, 17, 40–43 and for slice-1 shadow containment; the implementation deviations behind findings 40, 41, 42 and 43 are recorded in the evidence file and are material to how R2/R3 must source lesson provenance.

## Measurement schema (R8 foundation)

Add one append-only `cognitive_metric_events` relation and JSONL mirror. Event fields:

```text
metric_event_id
metric_definition_version
evaluation_dataset_version
evaluation_window_id
session_id
turn_number
event_kind
task_class
model_id
policy_version
decision_id
shadow_decision_id
live_action_id
action_attempt_id
tool_use_id
attempt
candidate_id
candidate_rank
prediction_id
self_model_prediction_id
self_model_dimension
self_model_backed
predicted_success_probability
verification_id
correction_id
attribution_adjudication_id
causal_candidate_id
adjudicated_causal_candidate_id
cause_action_class
attribution_cohort
followup_window_id
followup_match_stratum_id
cohort_entry_window_id
followup_opportunity_id
followup_eligible_opportunity_count
followup_verified_failure_count
lesson_id
retrieval_hit_id
prompt_snapshot_id
ordered_injected_rule_ids
rule_id
rule_operation
rule_injected
rule_population
rule_state_snapshot_id
rule_last_reinforced_at
rule_last_verified_reuse_at
stale_definition_version
injected_rule_count
stale_injected_rule_count
stale_rule_population
governed_proposal_id
proposal_kind
proposal_lifecycle_operation
proposal_decision
proposal_application_id
proposal_reversal_id
self_model_fact_id
self_model_version
self_model_confidence_before
self_model_confidence_after
label_source
label_definition_version
predicted_label
ground_truth_label
confidence
accepted
abstained
numerator
denominator
value
outcome_status
evidence_refs
created_at
idempotency_key
```

Fields irrelevant to an event kind are null, but each event kind defines required foreign keys:

- `correction_classified`: `correction_id`, predicted/ground-truth label, confidence, abstention, dataset version.
- `shadow_decision_compared`: shadow/live decision and action IDs, candidate/rank, `predicted_success_probability` in `[0,1]`, verified outcome.
- `attribution_evaluated`: correction, decision, action/tool-use, attempt, prediction, lesson, proposed and adjudicated causal candidate IDs, cause/action class, `accepted`, abstention, adjudication ID, and adjudication evidence refs. An accepted link is correct only when `causal_candidate_id == adjudicated_causal_candidate_id`; unmatched or explicitly no-cause adjudications are incorrect for precision.
- `attribution_followup_evaluated`: one event per `followup_opportunity_id`, carrying correction, causal candidate/action class, `attribution_cohort` (`accepted`, `abstained`, or `unattributed`), immutable follow-up window ID, `followup_match_stratum_id`, cohort-entry window ID, and deterministic binary verified-failure outcome. Version 1 forms exact-match strata on task class, cause/action class, model ID, policy version, and seven-day UTC cohort-entry bucket; only strata containing accepted and comparator opportunities are eligible. Derive cohort rates as failures/opportunities within each stratum, then compute the opportunity-weighted relative reduction `(comparator_rate - accepted_rate) / comparator_rate`; strata with comparator rate `0` are ineligible. Bootstrap paired match-strata with 10,000 deterministic resamples seeded by `evaluation_window_id`; the 2.5th and 97.5th percentiles form the interval.
- `self_model_prediction_evaluated`: self-model prediction ID, required `self_model_fact_id`, self-model dimension, `self_model_backed = true`, session/turn, task class, model/policy version, predicted success probability constrained to `[0,1]`, verification ID, and resulting verified outcome. A turn is eligible for the self-model-backed population only when this event exists before its action outcome is known.
- `retrieval_hit_observed`: hit, lesson/source identity, injection/citation status, verified outcome.
- `rule_lifecycle_observed`: rule ID, operation (`create`, `reinforce`, `retire`), and resulting rule population.
- `governed_proposal_observed`: one lifecycle operation per event for a governed proposal ID and kind. State machine version 1 is `proposed -> accepted|rejected`, `accepted -> applied`, `applied -> reversed`; every edge is allowed at most once, terminal states are `rejected` and `reversed`, and re-decision/re-application are invalid. `decide` requires `proposal_decision`; `apply` requires a globally unique application ID; `reverse` requires a globally unique reversal ID. Sequence uses a monotonic per-proposal lifecycle ordinal included in `attempt`; timestamps are evidence, not ordering authority. Acceptance denominator is distinct proposals reaching `accepted` or `rejected`; reversal denominator is distinct proposals reaching `applied`.
- `self_model_fact_updated`: self-model fact ID/dimension and version, session, verified evidence refs, confidence before/after constrained to finite `[0,1]`, policy version, and idempotency key. Reject NaN, infinity, and out-of-range confidence. Per-session confidence drift is the maximum absolute `self_model_confidence_after - self_model_confidence_before` among eligible fact updates in that session.
- `prompt_rules_composed`: immutable prompt-snapshot ID, `ordered_injected_rule_ids`, injected-rule denominator, stale-injected-rule numerator, rule/prompt policy version, immutable rule-state snapshot ID, and stale-definition version. A rule is stale under version 1 when it remains active and has neither verified reuse nor reinforcement during the 30-day interval ending at prompt composition; `rule_last_reinforced_at`, `rule_last_verified_reuse_at`, and the immutable rule-state snapshot make that predicate recomputable.

For each prompt composition, derive stale-rule prompt share as `stale_injected_rule_count / injected_rule_count`; when `injected_rule_count == 0`, define the per-prompt share as `0`. For evaluation windows and promotion gates, aggregate it only as the pooled ratio `sum(stale_injected_rule_count) / sum(injected_rule_count)`; when the pooled denominator is `0`, define the window share as `0`.
- `world_label_materialized`: action-attempt ID, prediction, verification, label source, label-definition version, resulting label.
- `surprise_observed`: prediction, action, verification, surprise value, outcome status.

Metric definitions are versioned code, not mutable rows. Evaluation windows store immutable start/end timestamps, eligible population query version, segmentation keys, and baseline/canary cohort identity. `self_model_confidence_calibration_error` version 1 is the Brier score `mean((predicted_success_probability - y)^2)` over eligible `self_model_prediction_evaluated` events with deterministic binary verification (`y = 1` for passed required verification, `y = 0` for failed required verification or failed execution); unknown/skipped/inconclusive outcomes are excluded and reported through label-coverage metrics.

Derive, do not separately mutate, these metrics:

- `corrections_per_100_turns`
- `correction_classifier_precision`, `recall`, `abstention_rate`
- `decision_shadow_agreement`, `shadow_top_candidate_success`, `shadow_calibration_error`
- `causal_attribution_accept_rate`, `causal_attribution_abstention_rate`, `causal_attribution_precision`, `causal_attribution_repeated_verified_failure_rate`
- `lesson_recall_rate`, `lesson_citation_rate`, `lesson_verified_reuse_rate`
- `memory_recall_precision`, `retrieval_source_coverage`, `retrieval_conflict_rate`
- `rule_create_count`, `rule_reinforce_count`, `rule_retire_count`, `rule_churn_rate`
- `governed_proposal_acceptance_rate`, `governed_proposal_reversal_rate`
- `self_model_confidence_drift`
- `verified_success_label_coverage`, `label_unknown_rate`, `label_conflict_rate`
- `latent_surprise_mean`, `median`, `p95`, and high-surprise rate
- `reflection_trigger_precision`, `reflection_verified_reuse_rate`
- `self_model_confidence_calibration_error`

Surface the same derived snapshot through `CognitiveInspection`, `archon cognitive status`, web/TUI cognitive views, and doctor diagnostics. Raw metric events remain the source of truth.

## Delivery sequence

### Slice 1 — R8 baseline and R3 correction signal

**Goal:** make correction learning measurable and stop heuristic false positives before adding more learning consumers.

1. Capture current heuristic decisions as shadow labels without changing behavior.
2. Build a versioned correction-classifier interface returning:

```rust
struct CorrectionClassification {
    is_correction: bool,
    correction_type: Option<CorrectionType>,
    confidence: f32,
    rationale_code: String,
}
```

3. Keep deterministic explicit corrections as high-confidence rules; use a cheap configured provider for ambiguous language.
4. Abstain below threshold; abstention must not create a correction, lesson, proposal, or rule mutation.
5. Create a reviewed evaluation corpus from real correction events with redacted text or stable hashes plus adjudicated labels.
6. Promotion gate: correction-classification precision >= 0.95 for the subset that may proceed to R2 attribution; report recall and abstention without hiding them. Classifier output alone never creates or reinforces rules.

**Primary seams:** `archon-core/src/agent/memory_integration.rs`, `archon-consciousness/src/corrections.rs`, reasoning-quality event bridge, cognitive metric store.

### Slice 2 — R1 shadow ExecutiveLoop

**Goal:** compare cognitive decisions against real turns without changing foreground actions.

1. Add `ShadowExecutiveObserver` in `archon-core`; it receives the live turn context and candidate action metadata.
2. Invoke classify → plan → score with a non-executing shadow adapter. Do not call `NoopActionExecutor` as evidence that an action succeeded.
3. Persist `DecisionRecord` before live execution.
4. After turn finalization, attach actual tool outcomes, verification outcomes, correction classification, and completion status.
5. Record shadow/live agreement and whether the shadow top candidate would have satisfied verified outcomes.
6. Promotion requires a minimum evaluated sample count, calibrated confidence, no regression in verified completion, and explicit operator approval.

**Primary seams:** `archon-core/src/agent/process_message_steps.rs`, `turn_completion.rs`, `archon-cognitive/src/executive_loop.rs`, `DecisionStore`.

### Slice 3 — R2 causal credit assignment

**Goal:** connect a correction to the decision, action, and assumption that caused it.

1. On a high-confidence correction, load the prior finalized decision, provider tool-use IDs, tool results, and verification evidence.
2. Run a bounded attribution model returning ranked causal candidates and confidence.
3. Require confidence and evidence thresholds; otherwise record abstention.
4. Store a structured lesson with provenance edges:

```text
Correction -> Corrects -> Decision
Correction -> CausedBy -> ToolRun/Assumption
Lesson -> DerivedFrom -> Correction + evidence
RuleProposal -> Generalizes -> Lesson
```

5. Deduplicate lessons by embedding similarity plus compatible cause/action class.
6. Reinforce or propose a rule only after attribution; never infer ownership from lexical similarity alone.
7. Evaluate accepted, abstained, and unattributed correction cohorts over the same immutable follow-up window and eligible repeated-opportunity query, matched by task class, cause/action class, model ID, policy version, and cohort-entry calendar window; normalize by eligible repeated opportunities and emit `attribution_followup_evaluated` with deterministic repeated-failure counts.

**Promotion metric:** accepted attributions later predict fewer repeated verified failures than abstained/unattributed corrections.

### Slice 4 — W5 verified training labels

**Goal:** train the world model on deterministic outcomes rather than prose keywords.

1. Introduce one immutable `action_attempt_id` shared by `WorldTraceRow`, advisor/prediction records, guarded actions, runtime outcomes, and verification outcomes. Derive it from session, provider tool-use/action reference, and attempt ordinal; retries get distinct IDs.
2. Add `turn_number`, `action_attempt_id`, `policy_version`, `idempotency_key`, and event time to trace and runtime outcome records. Preserve `row_id`, `prediction_id`, and verification IDs as typed foreign keys rather than replacing them.
3. Materialize label provenance with precedence:
   - passed required verification → `success = Some(true)`
   - failed verification or failed execution → `success = Some(false)`
   - absent, skipped, or inconclusive verification → `success = None`
4. R3 correction classification supplies `user_correction`; heuristic excerpt labels remain migration-only and lower precedence.
5. Record contradictions between old keyword labels and verification labels.
6. Make materialization idempotent by `(action_attempt_id, label_definition_version)`; reject conflicting duplicate identities.
7. Keep unknown labels out of binary success evaluation; do not coerce unknown to false or true.

**Promotion metrics:** join coverage, verified-label coverage, unknown rate, conflict rate, held-out calibration by task class.

### Slice 5 — W6 surprise-aware replay and R6 reflection

**Goal:** use prediction error without letting a few anomalies dominate learning.

1. Extend transition metadata with stable prediction/outcome linkage, latent surprise, recency, replay count, and priority version.
2. Sample a bounded mixture of uniform and prioritized examples. Cap surprise weight and preserve an unbiased held-out set.
3. Record importance weights or equivalent correction so prioritized sampling does not silently redefine the training distribution.
4. Trigger private reflection on high surprise, repeated tool failure, or high-confidence correction.
5. Reflection stores only a compact summary: goal, observed mismatch, proposed adjustment, evidence refs, confidence.
6. Inject only unresolved/relevant reflections into the next turn; track whether they are cited and whether verified outcomes improve.

**Promotion metrics:** held-out verified outcome accuracy, surprise distribution, reflection trigger precision, verified lesson reuse. Lower training loss alone is not a promotion gate.

### Slice 6 — R4 governed consolidation and R5 evidence-derived self model

**Goal:** turn repeated verified episodes into governed semantic knowledge and bounded identity state.

1. Nightly scheduler invokes Garden through a single-run lock and bounded work budget.
2. Cluster episodes/lessons by embeddings, but require provenance-compatible clusters.
3. Generate semantic-memory and rule-retirement proposals; never mutate prompt rules directly.
4. Route proposals through existing governed-learning approval and rollback.
5. Update `SelfModelStore` only from verified aggregate statistics: per-domain/tool success, calibrated confidence, repeated failure clusters, and established user preferences.
6. Bound confidence and personality drift per session; preserve static config as operator policy, not learned evidence.
7. Generate startup briefing from current SelfModel facts plus unresolved high-value lessons.
8. Before each action that consumes self-model facts, emit `self_model_prediction_evaluated` with the prediction fields and `self_model_backed = true`; attach deterministic verification after finalization without changing the pre-action prediction identity.

**Promotion metrics:** proposal acceptance/reversal, semantic-memory reuse, rule churn, stale-rule prompt share, self-model calibration.

### Slice 7 — R7 unified retrieval facade

**Goal:** one recall API over existing stores without forcing them into one database.

Define in `archon-knowledge`:

```rust
struct RecallQuery { text: String, limit: usize, source_policy: SourcePolicy }
struct RecallHit {
    source: RecallSource,
    source_id: String,
    content: String,
    normalized_score: f32,
    provenance_refs: Vec<String>,
    created_at: DateTime<Utc>,
    confidence: Option<f32>,
}
trait RecallSourceAdapter { async fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>>; }
```

1. Wrap memory, docs, KB, and LEANN with adapters; do not relocate their storage.
2. Normalize each source score through measured calibration, not ad hoc min-max scaling.
3. Deduplicate by provenance/content identity; detect conflicting claims rather than blending them.
4. Apply source quotas and latency budgets so one source cannot starve the rest.
5. Return partial results with explicit per-source errors.
6. Record which hits were injected, cited, ignored, contradicted, or followed by verified success.

**Promotion metrics:** recall precision, citation/use rate, source coverage, conflict detection accuracy, p50/p95 latency, per-source failure rate.

## Quantitative promotion gates

These are initial version-1 gates. Changing a threshold requires a new metric-definition/policy version and cannot rewrite prior evaluation windows. Use two-sided 95% Wilson intervals for proportions and bootstrap 95% confidence intervals for medians/percentiles.

| Slice | Minimum eligible evidence | Promotion threshold | Automatic rollback trigger |
|---|---:|---|---|
| R3 | 400 adjudicated examples, including >=100 corrections and >=100 non-corrections | rule-mutating precision point estimate >=0.95 and 95% lower bound >=0.90; explicit deterministic cases recall =1.0; abstention reported | any confirmed false-positive rule mutation in canary, or precision point estimate <0.95 |
| R1 | 500 non-trivial shadow turns, >=200 with deterministic verification, >=50 per promoted task-class stratum | shadow top-candidate verified-success rate non-inferior to live path by <=2 percentage points; Brier score <=0.20; zero shadow suggestions violating existing policy | any unsafe/policy-violating suggestion, or verified-success non-inferiority margin >2 points |
| R2 | 200 adjudicated correction attributions, >=100 accepted causal links; >=100 eligible repeated opportunities in the accepted cohort and >=100 across the pooled abstained/unattributed comparator, all belonging to eligible matched strata with comparator rate >0 | accepted-link precision >=0.90 with 95% lower bound >=0.85; 100% provenance join integrity; accepted-cohort repeated verified failure rate must be at least 10% relatively lower than the pooled comparator, with a two-sided 95% bootstrap confidence interval for the relative reduction excluding 0; zero eligible matched strata or an undefined effect means no promotion | any causal lesson linked to wrong session/action, conflicting duplicate attribution identity, accepted-cohort repeated verified failure rate exceeding the pooled comparator, or effect becoming undefined after eligibility filtering |
| W5 | 500 eligible verified action attempts across >=3 task classes | trace/action/outcome join coverage >=0.95; conflicting deterministic labels <0.01; unknown rate reported, never coerced | join coverage <0.95 or any duplicate identity materializes conflicting labels |
| W6/R6 | 1,000 linked transitions; matched 500-uniform baseline and 500-prioritized canary evaluations | held-out verified-success accuracy non-inferior by <=2 points; calibration error does not worsen by >0.02; reflection verified-reuse rate >=0.20 after >=100 triggered reflections | accuracy margin >2 points, calibration worsens >0.02, or one priority decile supplies >40% of a batch |
| R4/R5 | 100 governed proposals and 500 self-model-backed turns, with >=50 self-model-backed verified turns per promoted task-class/model/policy/self-model-dimension stratum | proposal reversal rate <=0.05 among applied proposals; if baseline stale-rule prompt share >0, decrease >=20% relative; if baseline is zero, canary must remain zero; self-model Brier score <=0.10 in every eligible promoted stratum | unapproved or invalid-state mutation, reversal rate >0.05, stale share rises from zero, any eligible stratum Brier score >0.10, or confidence drift >0.10 in one session |
| R7 | 500 replayable queries with adjudicated relevant sources, >=50 per source | recall precision@10 >=0.80; source coverage >=0.95; conflict recall >=0.90; p95 latency no more than 20% above the slowest enabled source budget | precision <0.80, missed known conflict, or any source silently omitted |

For online comparisons, baseline is the latest eligible pre-change cohort matching the canary cohort size and segmentation. A stratum below its minimum remains `Needs runtime/data validation`; aggregate success cannot promote it. False-completion rate, unsafe-action count, and corrections per 100 turns must be non-inferior with a zero-tolerance rollback for newly introduced unsafe actions.

## Release gates

Each slice ships independently and must satisfy:

- unit tests for deterministic policy and schema behavior;
- integration tests with real stores and separate source-of-truth reads;
- shadow/canary run before any behavior-changing promotion;
- baseline and post-change metric windows with identical eligibility query, cohort size, and segmentation;
- quantitative slice gate above passes, with confidence interval and evidence population reported;
- zero newly introduced unsafe actions; false-completion and correction rates remain within stated non-inferiority margins;
- threshold breach automatically disables the new classifier/observer/replay/consolidation/retrieval policy version and restores the prior version;
- independent cold-read review.

## Research basis

- Prioritized replay can improve sample efficiency but shifts the sampled distribution; retain uniform coverage and correct or bound prioritization bias. Relevant work: *Introspective Experience Replay: Look Back When Surprised* (arXiv:2206.03171), *Revisiting Prioritized Experience Replay* (arXiv:2102.03261), and *Regularized Optimal Experience Replay* (arXiv:2407.03995).
- Agent-memory evaluation must measure retrieval usefulness, learning, long-range consistency, and conflict resolution—not storage volume alone.
- Causal memory selection and attribution should be evaluated by intervention or downstream verified outcome, not semantic similarity alone.

## Non-goals

- No autonomous policy enforcement in R1.
- No raw chain-of-thought persistence.
- No direct rule mutation from classifier or reflection output.
- No single physical database migration for R7.
- No claim of intelligence improvement from counts, training loss, or anecdotal examples alone.
