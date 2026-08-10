//! Injecting unresolved reflections into later turns, and measuring their reuse.
//!
//! Issue #81 shipped the triggers and the writer, so reflections are recorded —
//! and then never read by anything the agent does. This is the read side, and it
//! is bounded in three independent ways because an unbounded one would turn the
//! prompt into a lesson log:
//!
//! * only reflections whose situation kind matches the current turn are
//!   candidates (relevance);
//! * at most [`MAX_INJECTED_REFLECTIONS`] are injected into any one turn;
//! * a reflection is injected at most [`MAX_INJECTIONS_PER_REFLECTION`] times
//!   per session, so a long session cannot keep re-serving the same lesson and
//!   the injected block cannot grow with session length.
//!
//! Resolution is measured, not assumed. A reflection leaves the pool when it has
//! been *verifiably reused*: cited by the turn that received it **and** followed
//! by a deterministic verified pass. A citation on its own moves the citation
//! counter and nothing else — reading a lesson is not evidence that it helped.

use std::collections::{BTreeMap, BTreeSet};

use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::cozo_guard::run_script_guarded;
use crate::metrics::emit::{MetricEmitter, runtime_cohort};
use crate::metrics::event::MetricEventKind;
use crate::schema::ensure_cognitive_schema;
use crate::self_model::prediction::{LABEL_SOURCE, TurnVerification};
use crate::{CognitiveError, SituationKind};

/// Most reflections injected into a single turn.
pub const MAX_INJECTED_REFLECTIONS: usize = 3;

/// Times one reflection may be injected within one session.
///
/// Three: enough for a lesson to be seen on a couple of later turns, few enough
/// that an unresolved reflection cannot occupy the prompt for a whole session.
pub const MAX_INJECTIONS_PER_REFLECTION: i64 = 3;

/// Metric name for the citation rate of injected reflections.
pub const REFLECTION_CITATION_METRIC: &str = "lesson_citation_rate";

/// Length of the citation marker's hash, in hex characters.
const MARKER_HEX_LEN: usize = 8;

/// One unresolved reflection, ready to be shown to a later turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedReflection {
    pub reflection_id: String,
    /// Stable, short token the turn cites when it acts on the lesson.
    pub marker: String,
    pub lesson: String,
    pub trigger: String,
    pub confidence: f32,
    /// Times this reflection has already been injected in this session.
    pub injection_count: i64,
}

/// What one injected reflection did on the turn that received it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReflectionReuseTally {
    pub injected: usize,
    pub cited: usize,
    /// Cited **and** followed by a deterministic verified pass.
    pub verified_reuse: usize,
    pub metrics_emitted: usize,
}

pub struct ReflectionRecall<'a> {
    db: &'a DbInstance,
    ledger_dir: std::path::PathBuf,
    policy: Option<CognitivePolicy>,
}

impl<'a> ReflectionRecall<'a> {
    pub fn new(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<std::path::Path>,
        policy: Option<CognitivePolicy>,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
            policy,
        })
    }

    /// Unresolved, relevant reflections for a turn of `situation_kind`.
    ///
    /// Newest first, capped at [`MAX_INJECTED_REFLECTIONS`].
    pub fn unresolved_for_turn(
        &self,
        session_id: &str,
        situation_kind: SituationKind,
    ) -> Result<Vec<UnresolvedReflection>, CognitiveError> {
        let injections = self.session_injections(session_id)?;
        let mut params = BTreeMap::new();
        params.insert(
            "situation_kind".into(),
            DataValue::from(situation_kind.as_str()),
        );
        let rows = run_script_guarded(
            self.db,
            "?[reflection_id, lesson, created_at, trigger, confidence] := \
             *cognitive_reflections{reflection_id, situation_kind: $situation_kind, lesson, created_at}, \
             *cognitive_reflection_evidence{reflection_id, trigger, confidence}",
            params,
            ScriptMutability::Immutable,
            "read unresolved reflections for turn",
        )?;

        let mut candidates: Vec<(String, UnresolvedReflection)> = rows
            .rows
            .iter()
            .filter_map(|row| {
                let reflection_id = str_col(row, 0);
                let lesson = str_col(row, 1);
                if reflection_id.is_empty() || lesson.trim().is_empty() {
                    return None;
                }
                let (injection_count, verified_reuse_count) =
                    injections.get(&reflection_id).copied().unwrap_or((0, 0));
                // Verified reuse is what resolves a reflection. Anything else
                // (including having been cited) leaves it in the pool.
                if verified_reuse_count > 0 || injection_count >= MAX_INJECTIONS_PER_REFLECTION {
                    return None;
                }
                Some((
                    str_col(row, 2),
                    UnresolvedReflection {
                        marker: marker_for(&reflection_id),
                        reflection_id,
                        lesson,
                        trigger: str_col(row, 3),
                        confidence: row[4].get_float().unwrap_or(0.0) as f32,
                        injection_count,
                    },
                ))
            })
            .collect();
        // Sort by recorded time, then id, so the selection is deterministic even
        // when two reflections share a timestamp.
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.reflection_id.cmp(&left.1.reflection_id))
        });
        candidates.truncate(MAX_INJECTED_REFLECTIONS);
        Ok(candidates
            .into_iter()
            .map(|(_, reflection)| reflection)
            .collect())
    }

    /// Count an injection before the turn runs.
    ///
    /// Recorded at injection time rather than at completion: a turn that dies
    /// mid-way still consumed one of the reflection's injections, and counting
    /// it only on success would let a repeatedly failing turn re-inject forever.
    pub fn record_injection(
        &self,
        session_id: &str,
        turn_number: u64,
        reflections: &[UnresolvedReflection],
    ) -> Result<(), CognitiveError> {
        let existing = self.session_rows(session_id)?;
        for reflection in reflections {
            let now = Utc::now().to_rfc3339();
            let previous = existing.get(&reflection.reflection_id);
            let mut params = base_params(&reflection.reflection_id, session_id);
            params.insert(
                "injection_count".into(),
                DataValue::from(previous.map(|row| row.injection_count).unwrap_or(0) + 1),
            );
            params.insert(
                "cited_count".into(),
                DataValue::from(previous.map(|row| row.cited_count).unwrap_or(0)),
            );
            params.insert(
                "verified_reuse_count".into(),
                DataValue::from(previous.map(|row| row.verified_reuse_count).unwrap_or(0)),
            );
            params.insert(
                "last_turn_number".into(),
                DataValue::from(turn_number as i64),
            );
            params.insert(
                "first_injected_at".into(),
                DataValue::from(
                    previous
                        .map(|row| row.first_injected_at.as_str())
                        .unwrap_or(now.as_str()),
                ),
            );
            params.insert("last_injected_at".into(), DataValue::from(now.as_str()));
            run_script_guarded(
                self.db,
                INJECTION_PUT,
                params,
                ScriptMutability::Mutable,
                "record reflection injection",
            )?;
        }
        Ok(())
    }

    /// Score the injected reflections against what the turn actually did.
    ///
    /// `cited` names the reflections whose marker appeared in the turn's own
    /// output. Verified reuse additionally requires `verification` to be a
    /// deterministic pass.
    pub fn record_outcome(
        &self,
        session_id: &str,
        turn_number: u64,
        reflections: &[UnresolvedReflection],
        cited: &BTreeSet<String>,
        verification: TurnVerification,
        model_id: &str,
        situation_kind: SituationKind,
    ) -> Result<ReflectionReuseTally, CognitiveError> {
        let mut tally = ReflectionReuseTally {
            injected: reflections.len(),
            ..ReflectionReuseTally::default()
        };
        if reflections.is_empty() {
            return Ok(tally);
        }
        let existing = self.session_rows(session_id)?;
        let emitter = MetricEmitter::open(
            self.db,
            &self.ledger_dir,
            runtime_cohort(situation_kind.as_str(), model_id, self.policy.as_ref()),
        )?;
        for reflection in reflections {
            let was_cited = cited.contains(&reflection.reflection_id);
            let verified_reuse = was_cited && verification == TurnVerification::Passed;
            tally.cited += usize::from(was_cited);
            tally.verified_reuse += usize::from(verified_reuse);
            self.bump_counts(session_id, &existing, reflection, was_cited, verified_reuse)?;

            let hit_id = format!("refhit:{session_id}:{turn_number}:{}", reflection.marker);
            let mut event = emitter
                .event(
                    REFLECTION_CITATION_METRIC,
                    MetricEventKind::RetrievalHitObserved,
                    &hit_id,
                    Utc::now(),
                )
                .with_session(session_id, turn_number)
                .with_value(f64::from(reflection.confidence.clamp(0.0, 1.0)))
                .with_outcome(verification.outcome_status())
                .with_identity("retrieval_hit_id", &hit_id)
                .with_identity("lesson_id", &reflection.reflection_id)
                .with_identity("rule_injected", "true")
                .with_identity("cited", bool_identity(was_cited))
                .with_identity("verified_reuse", bool_identity(verified_reuse))
                .with_identity("reflection_trigger", &reflection.trigger);
            event.label_source = LABEL_SOURCE.into();
            event.evidence_refs = vec![
                format!("cognitive_reflection:{}", reflection.reflection_id),
                format!("verification:turn_exec:{session_id}:{turn_number}"),
            ];
            if matches!(
                emitter.record(&event)?,
                crate::metrics::MetricWriteOutcome::Written
            ) {
                tally.metrics_emitted += 1;
            }
        }
        Ok(tally)
    }

    fn bump_counts(
        &self,
        session_id: &str,
        existing: &BTreeMap<String, InjectionRow>,
        reflection: &UnresolvedReflection,
        cited: bool,
        verified_reuse: bool,
    ) -> Result<(), CognitiveError> {
        let Some(previous) = existing.get(&reflection.reflection_id) else {
            // Nothing was recorded at injection time, so there is no row to
            // grade. Writing one here would invent an injection.
            return Ok(());
        };
        let mut params = base_params(&reflection.reflection_id, session_id);
        params.insert(
            "injection_count".into(),
            DataValue::from(previous.injection_count),
        );
        params.insert(
            "cited_count".into(),
            DataValue::from(previous.cited_count + i64::from(cited)),
        );
        params.insert(
            "verified_reuse_count".into(),
            DataValue::from(previous.verified_reuse_count + i64::from(verified_reuse)),
        );
        params.insert(
            "last_turn_number".into(),
            DataValue::from(previous.last_turn_number),
        );
        params.insert(
            "first_injected_at".into(),
            DataValue::from(previous.first_injected_at.as_str()),
        );
        params.insert(
            "last_injected_at".into(),
            DataValue::from(previous.last_injected_at.as_str()),
        );
        run_script_guarded(
            self.db,
            INJECTION_PUT,
            params,
            ScriptMutability::Mutable,
            "record reflection reuse outcome",
        )?;
        Ok(())
    }

    fn session_injections(
        &self,
        session_id: &str,
    ) -> Result<BTreeMap<String, (i64, i64)>, CognitiveError> {
        Ok(self
            .session_rows(session_id)?
            .into_iter()
            .map(|(id, row)| (id, (row.injection_count, row.verified_reuse_count)))
            .collect())
    }

    fn session_rows(
        &self,
        session_id: &str,
    ) -> Result<BTreeMap<String, InjectionRow>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".into(), DataValue::from(session_id));
        let rows = run_script_guarded(
            self.db,
            "?[reflection_id, injection_count, cited_count, verified_reuse_count, last_turn_number, first_injected_at, last_injected_at] := \
             *cognitive_reflection_injections{reflection_id, session_id: $session_id, injection_count, cited_count, verified_reuse_count, last_turn_number, first_injected_at, last_injected_at}",
            params,
            ScriptMutability::Immutable,
            "read reflection injections for session",
        )?;
        Ok(rows
            .rows
            .iter()
            .map(|row| {
                (
                    str_col(row, 0),
                    InjectionRow {
                        injection_count: row[1].get_int().unwrap_or(0),
                        cited_count: row[2].get_int().unwrap_or(0),
                        verified_reuse_count: row[3].get_int().unwrap_or(0),
                        last_turn_number: row[4].get_int().unwrap_or(0),
                        first_injected_at: str_col(row, 5),
                        last_injected_at: str_col(row, 6),
                    },
                )
            })
            .collect())
    }
}

#[derive(Debug, Clone, Default)]
struct InjectionRow {
    injection_count: i64,
    cited_count: i64,
    verified_reuse_count: i64,
    last_turn_number: i64,
    first_injected_at: String,
    last_injected_at: String,
}

const INJECTION_PUT: &str = "?[reflection_id, session_id, injection_count, cited_count, verified_reuse_count, last_turn_number, first_injected_at, last_injected_at] <- \
     [[$reflection_id, $session_id, $injection_count, $cited_count, $verified_reuse_count, $last_turn_number, $first_injected_at, $last_injected_at]]
     :put cognitive_reflection_injections { reflection_id, session_id => injection_count, cited_count, verified_reuse_count, last_turn_number, first_injected_at, last_injected_at }";

/// Stable citation token for a reflection.
///
/// Hashed rather than the raw id: the marker is shown to the model and echoed
/// back, and a short opaque token keeps that channel from carrying anything but
/// an identifier.
pub fn marker_for(reflection_id: &str) -> String {
    let digest = blake3::hash(reflection_id.as_bytes()).to_hex().to_string();
    format!("ref:{}", &digest[..MARKER_HEX_LEN])
}

/// Reflection ids whose marker appears in `text`.
///
/// Exact marker match, so a turn that merely repeats a lesson's wording is not
/// counted as citing it.
pub fn cited_reflection_ids(text: &str, reflections: &[UnresolvedReflection]) -> BTreeSet<String> {
    let haystack = text.to_ascii_lowercase();
    reflections
        .iter()
        .filter(|reflection| haystack.contains(&reflection.marker.to_ascii_lowercase()))
        .map(|reflection| reflection.reflection_id.clone())
        .collect()
}

/// Prompt block for the injected reflections, or `None` when there are none.
///
/// The block carries only the lesson strings the writer composed from enums and
/// counts, plus their markers; there is no path here for turn text.
pub fn render_block(reflections: &[UnresolvedReflection]) -> Option<String> {
    if reflections.is_empty() {
        return None;
    }
    let mut block = String::from(
        "Unresolved lessons from earlier turns. If one of them changes what you do, cite its \
         marker verbatim in your reply so the lesson can be measured; do not cite one you did \
         not use.\n",
    );
    for reflection in reflections {
        block.push_str(&format!(
            "- [{}] ({}, confidence {:.2}) {}\n",
            reflection.marker, reflection.trigger, reflection.confidence, reflection.lesson
        ));
    }
    Some(block)
}

fn base_params(reflection_id: &str, session_id: &str) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert("reflection_id".into(), DataValue::from(reflection_id));
    params.insert("session_id".into(), DataValue::from(session_id));
    params
}

fn bool_identity(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}
