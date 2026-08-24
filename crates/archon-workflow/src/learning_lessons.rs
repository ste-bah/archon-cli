//! The curated half of the learning bridge: prose a later run can act on.
//!
//! # Why a second stream
//!
//! [`crate::learning`] writes one forensic record per stage — status, quality,
//! artifact ids, attempt counts. That stream is the right shape for the fold
//! that feeds SONA and ReasoningBank, and the wrong shape for a prompt. It was
//! injected into the v3 authoring brief once and cost 340KB of context: a
//! hundred stage ids and artifact paths from finished runs, none of which
//! carried a sentence saying what to do differently. A forensic record answers
//! "what happened"; an author needs "what to do".
//!
//! So this module distils the records into **curated lessons**: a fixed
//! headline, fixed guidance, and a count. Nothing else. The prose is static
//! text selected by [`LessonRule`] — the distiller decides *which* rules fired
//! and *how often*, never what the words are. That is the mechanism that keeps
//! the loop from re-poisoning the prompt:
//!
//! * **Path-free by construction.** No stage id, task id, file path or run id
//!   is ever interpolated into rendered text, because there is no interpolation
//!   point. [`lessons_are_path_free`] holds this as an invariant and a test
//!   asserts it over every rule.
//! * **Project-agnostic by construction.** The rules fire on stage-outcome
//!   shape — accepted-but-empty, retried, verification-heavy — which every
//!   workflow has regardless of language, PRD or task naming.
//! * **Bounded.** [`render_lessons_block`] caps both the lesson count and the
//!   byte size, so a corpus that grows to a thousand runs still renders the
//!   same few hundred bytes.
//!
//! # Why lessons merge instead of accumulate
//!
//! Evidence lives in [`LessonEvidence`], separate from the prose, so the same
//! rule observed in nine runs collapses to one line carrying `runs: 9` rather
//! than nine near-identical lines. Numbers therefore never appear inside a
//! headline — a baked-in "13 of 19 stages" would be stale the moment two runs
//! merged.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::learning::{Verification, WorkflowLearningRecord};
use crate::run::{RunStatus, WorkflowRun};
use crate::spec::ProviderTier;
use crate::store::WorkflowStore;

/// File name of the curated stream, under `<run>/learning/`.
pub const LEARNING_LESSONS_FILE: &str = "lessons.jsonl";

/// Most lessons rendered into a brief.
///
/// Six is what fits beside the rest of the brief without displacing the task
/// paths and wave groups, which are the parts the author cannot work without.
pub const MAX_RENDERED_LESSONS: usize = 6;

/// Hard byte ceiling on the rendered block.
///
/// The read side that was removed injected 340KB. A ceiling rather than a
/// count is the durable guard: it holds even if a future rule writes long
/// guidance or the corpus grows past anything anticipated here.
pub const MAX_LESSONS_BYTES: usize = 4096;

/// Most recent runs whose lessons are considered.
///
/// Without a bound, a defect fixed months ago keeps teaching forever: its
/// lesson files stay on disk and `runs` only ever grows, so the rule that was
/// most common historically outranks the one biting now. A window is the whole
/// mechanism by which a fixed problem stops being taught — nothing deletes a
/// lesson, it simply ages out of the window.
pub const MAX_SOURCE_RUNS: usize = 20;

/// A rule must be seen in at least this many runs before it is rendered.
///
/// One run is an anecdote. Two is the cheapest threshold that still filters
/// a single unlucky run from teaching every later one a false lesson.
pub const MIN_RUNS_TO_RENDER: usize = 2;

/// The distilled failure modes. **The prose lives here and nowhere else** —
/// every string returned by [`LessonRule::headline`] and
/// [`LessonRule::guidance`] is a literal, which is what makes a rendered
/// lesson path-free and project-agnostic without needing to be scrubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonRule {
    /// An implementation stage was accepted having produced no artifact.
    SilentImplementation,
    /// Stages needed more than one attempt to reach a verdict.
    RetryChurn,
    /// Verification and remediation outnumbered the implementation work.
    VerificationDominance,
    /// The run reached a terminal state with stages still unverified.
    UnfinishedRun,
    /// The run produced nothing durable at all.
    NoDurableOutput,
}

impl LessonRule {
    /// Every rule, for exhaustive iteration in tests and rendering order.
    pub const ALL: [Self; 5] = [
        Self::SilentImplementation,
        Self::NoDurableOutput,
        Self::UnfinishedRun,
        Self::VerificationDominance,
        Self::RetryChurn,
    ];

    /// The kind of work this lesson bears on. Deliberately one of a fixed set
    /// of generic labels — never a task id or a stage id.
    pub fn scope(self) -> &'static str {
        match self {
            Self::SilentImplementation | Self::NoDurableOutput => "implementation",
            Self::VerificationDominance | Self::RetryChurn => "verification",
            Self::UnfinishedRun => "run",
        }
    }

    /// One line naming the failure. No numbers: lessons merge across runs and
    /// a baked-in count would be stale after the first merge.
    pub fn headline(self) -> &'static str {
        match self {
            Self::SilentImplementation => {
                "Implementation calls were accepted after changing nothing."
            }
            Self::RetryChurn => "Stages needed repeat attempts before reaching a verdict.",
            Self::VerificationDominance => {
                "Calls that inspected or judged the work outnumbered the calls that wrote any."
            }
            Self::UnfinishedRun => "The run ended with stages that never reached a verdict.",
            Self::NoDurableOutput => "The run finished having produced nothing durable.",
        }
    }

    /// What to do differently. Addressed to the author of the next script, in
    /// terms of the dialect it is writing, not of any particular project.
    pub fn guidance(self) -> &'static str {
        match self {
            Self::SilentImplementation => {
                "Gate every implementation call on evidence of change. An outcome with no files \
                 changed and no commands run is a failure, not a success, unless it is an explicit \
                 typed no-op carrying the proof that the work already existed."
            }
            Self::RetryChurn => {
                "Give each call one narrow objective and one declared check. A stage that retries \
                 is usually a stage that was asked for several things at once, so the verdict \
                 could never be reached in a single pass."
            }
            Self::VerificationDominance => {
                "State the acceptance condition in the implementation call itself, not only in the \
                 verification that follows it. Work verified against a condition it never saw \
                 fails, and each failure costs a remediation cycle as expensive as the original."
            }
            Self::UnfinishedRun => {
                "Account for every task exactly once, with its real status, and keep the number of \
                 calls small enough that the run can reach the end. A plan that cannot finish \
                 teaches nothing about the tasks it never reached."
            }
            Self::NoDurableOutput => {
                "Make the run's deliverable an artifact something else can read. Work that exists \
                 only as text in a call summary is indistinguishable from work that was never done."
            }
        }
    }
}

/// Counts backing a lesson. Counts only — never identifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonEvidence {
    /// Distinct runs in which the rule fired. 1 for a freshly distilled lesson.
    pub runs: usize,
    /// Stages that exhibited the rule, summed across those runs.
    pub occurrences: usize,
    /// Stages examined, summed across those runs.
    ///
    /// A magnitude indicator, not a strict denominator: it counts every stage
    /// in the run, while some rules count only the stages they apply to. It is
    /// here so a reader can tell one bad stage in three from one in three
    /// hundred, not so the two numbers can be divided.
    pub stages: usize,
}

impl LessonEvidence {
    fn merge(&mut self, other: &Self) {
        self.runs += other.runs;
        self.occurrences += other.occurrences;
        self.stages += other.stages;
    }
}

/// One curated lesson. `run_id` and `outcome` are provenance for the store and
/// are deliberately **not** rendered — a run id in a prompt is an invitation to
/// go and read that run, which is the behaviour this stream replaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CuratedLesson {
    pub rule: LessonRule,
    pub evidence: LessonEvidence,
    pub run_id: String,
    /// Terminal status label of the run this was distilled from.
    pub outcome: String,
    pub ts: DateTime<Utc>,
}

impl CuratedLesson {
    fn new(rule: LessonRule, evidence: LessonEvidence, run: &WorkflowRun) -> Self {
        Self {
            rule,
            evidence,
            run_id: run.id.clone(),
            // Via serde rather than `Debug`: `RunStatus` is snake_case on the
            // wire and every other consumer reads that spelling.
            outcome: serde_json::to_value(&run.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string()),
            ts: Utc::now(),
        }
    }

    pub fn scope(&self) -> &'static str {
        self.rule.scope()
    }

    /// The rendered line, without the leading bullet.
    pub fn render(&self) -> String {
        format!(
            "[{}] {} {} (seen in {} run{}, {} of {} stages)",
            self.rule.scope(),
            self.rule.headline(),
            self.rule.guidance(),
            self.evidence.runs,
            if self.evidence.runs == 1 { "" } else { "s" },
            self.evidence.occurrences,
            self.evidence.stages,
        )
    }
}

/// Whether every rule's rendered prose is free of path- and identifier-shaped
/// text. Held as an invariant rather than enforced by scrubbing: the prose is
/// literal, so the correct guard is an assertion that it stayed literal.
pub fn lessons_are_path_free() -> bool {
    LessonRule::ALL.iter().all(|rule| {
        let text = format!("{} {}", rule.headline(), rule.guidance());
        !text.contains('/') && !text.contains('\\') && !text.contains("::")
    })
}

/// A call that was asked to **write** something.
///
/// Keyed on the provider tier, which the host derives from the call method, so
/// this holds for any workflow in any language and reads no stage names. The
/// `phase` fallback covers records written before the tier was carried, and
/// hand-authored specs that set no tier.
///
/// Deliberately narrow: in the v3 dialect a verification is an ordinary agent
/// call, so counting `agent` as producing would both hide the imbalance below
/// and mislabel every verification that legitimately wrote nothing.
fn is_producing(record: &WorkflowLearningRecord) -> bool {
    match record.provider_tier {
        Some(tier) => tier == ProviderTier::Coder,
        None => record.phase == "implementation",
    }
}

/// A call that inspects, judges or reviews rather than writes.
///
/// Excludes checkpoints, artifact saves and tool calls: they are bookkeeping,
/// and counting them would make every run look verification-heavy.
fn is_inspecting(record: &WorkflowLearningRecord) -> bool {
    match record.provider_tier {
        Some(tier) => matches!(
            tier,
            ProviderTier::Researcher | ProviderTier::Critic | ProviderTier::Reducer
        ),
        None => matches!(
            record.phase.as_str(),
            "agent" | "quality_gate" | "human_gate" | "reduce"
        ),
    }
}

/// Distil a run's forensic records into curated lessons.
///
/// Side-effect free, and deterministic in everything that is read back: the
/// same records always yield the same rules with the same counts. Only `ts` is
/// wall-clock, and nothing ranks or renders on a single lesson's `ts` — it
/// exists to order whole runs in [`collect_curated_lessons`]. That is what lets
/// a resumed run rewrite the file wholesale without double-counting itself.
pub fn distil_lessons(run: &WorkflowRun, records: &[WorkflowLearningRecord]) -> Vec<CuratedLesson> {
    let stages = records.len();
    if stages == 0 {
        return Vec::new();
    }
    let mut lessons = Vec::new();
    let mut push = |rule: LessonRule, occurrences: usize| {
        if occurrences > 0 {
            lessons.push(CuratedLesson::new(
                rule,
                LessonEvidence {
                    runs: 1,
                    occurrences,
                    stages,
                },
                run,
            ));
        }
    };

    let silent = records
        .iter()
        .filter(|record| {
            is_producing(record)
                && record.verification == Verification::Accepted
                && record.telemetry.artifact_count == 0
        })
        .count();
    push(LessonRule::SilentImplementation, silent);

    let retried = records
        .iter()
        .filter(|record| record.telemetry.attempt > 1)
        .count();
    push(LessonRule::RetryChurn, retried);

    // Fires only when inspection actually outweighed writing. A run with one
    // verification per implementation is the intended shape and must not teach
    // a later run that verification is a problem.
    let producing = records.iter().filter(|record| is_producing(record)).count();
    let inspecting = records
        .iter()
        .filter(|record| is_inspecting(record))
        .count();
    if inspecting > producing {
        push(LessonRule::VerificationDominance, inspecting);
    }

    if run.status != RunStatus::Completed {
        let unverified = records
            .iter()
            .filter(|record| record.verification == Verification::Unverified)
            .count();
        push(LessonRule::UnfinishedRun, unverified);
    }

    if !records.iter().any(|record| record.durable) {
        // The whole run is the occurrence: there is no per-stage instance of
        // "nothing durable existed anywhere".
        push(LessonRule::NoDurableOutput, 1);
    }

    lessons
}

fn learning_dir(store: &WorkflowStore, run_id: &str) -> PathBuf {
    store.run_dir(run_id).join("learning")
}

/// Path of the curated stream for a run.
pub fn lessons_path(store: &WorkflowStore, run_id: &str) -> PathBuf {
    learning_dir(store, run_id).join(LEARNING_LESSONS_FILE)
}

/// Write `<run>/learning/lessons.jsonl`.
///
/// Truncating, for the same reason the record stream truncates: a resume
/// re-derives every stage's current state, so appending would double-count.
pub fn write_lessons(
    store: &WorkflowStore,
    run_id: &str,
    lessons: &[CuratedLesson],
) -> WorkflowResult<()> {
    let dir = learning_dir(store, run_id);
    std::fs::create_dir_all(&dir).map_err(|e| WorkflowError::io(&dir, e))?;
    let path = dir.join(LEARNING_LESSONS_FILE);
    let mut body = String::new();
    for lesson in lessons {
        body.push_str(&serde_json::to_string(lesson)?);
        body.push('\n');
    }
    std::fs::write(&path, body).map_err(|e| WorkflowError::io(path, e))
}

/// Read one run's curated lessons.
///
/// A missing file means the run predates this stream or wrote nothing; an
/// unparseable line is skipped rather than failing the read, which is also how
/// a lesson written by a future version carrying an unknown rule is tolerated.
pub fn read_lessons(path: &Path) -> Vec<CuratedLesson> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<CuratedLesson>(line).ok())
        .collect()
}

/// Collect and merge curated lessons across every run in the store.
///
/// Reads `learning/lessons.jsonl` directly rather than going through
/// `list_runs`, which loads and parses every `state.json` — those run to tens
/// of megabytes across a busy store, for data this has no use for.
///
/// `exclude_run_id` drops the in-flight run: a run must not be taught by its
/// own partial record, which on a resume would be a lesson drawn from the very
/// stages it is about to retry.
pub fn collect_curated_lessons(
    store: &WorkflowStore,
    exclude_run_id: Option<&str>,
) -> Vec<CuratedLesson> {
    let Ok(entries) = std::fs::read_dir(store.root()) else {
        return Vec::new();
    };
    let mut per_run: Vec<(DateTime<Utc>, Vec<CuratedLesson>)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(run_id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if exclude_run_id == Some(run_id.as_str()) {
            continue;
        }
        let lessons = read_lessons(&entry.path().join("learning").join(LEARNING_LESSONS_FILE));
        // Ordered by the lessons' own timestamps rather than by directory
        // mtime: a run dir is touched by anything that reads it, and the
        // question here is when the run *ended*.
        if let Some(newest) = lessons.iter().map(|lesson| lesson.ts).max() {
            per_run.push((newest, lessons));
        }
    }
    per_run.sort_by(|a, b| b.0.cmp(&a.0));
    per_run.truncate(MAX_SOURCE_RUNS);

    let mut merged: BTreeMap<LessonRule, CuratedLesson> = BTreeMap::new();
    for lesson in per_run.into_iter().flat_map(|(_, lessons)| lessons) {
        // Written as a `match` rather than `and_modify(..).or_insert(..)`: the
        // modify closure would borrow `lesson` while `or_insert` moves it, in
        // one expression.
        match merged.entry(lesson.rule) {
            Entry::Occupied(mut slot) => {
                let existing = slot.get_mut();
                existing.evidence.merge(&lesson.evidence);
                // Provenance tracks the most recent contributor, so a merged
                // lesson still says which run last saw it.
                if lesson.ts > existing.ts {
                    existing.ts = lesson.ts;
                    existing.run_id = lesson.run_id;
                    existing.outcome = lesson.outcome;
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(lesson);
            }
        }
    }
    let mut lessons: Vec<CuratedLesson> = merged.into_values().collect();
    // Breadth first: a rule seen across nine runs outranks one seen many times
    // in a single unlucky run. `rule` breaks ties so the order is stable, which
    // keeps the brief content-keyed for frontier reuse.
    lessons.sort_by(|a, b| {
        b.evidence
            .runs
            .cmp(&a.evidence.runs)
            .then(b.evidence.occurrences.cmp(&a.evidence.occurrences))
            .then(a.rule.cmp(&b.rule))
    });
    lessons
}

/// Render the block handed to the authoring brief.
///
/// Returns an empty string when nothing clears [`MIN_RUNS_TO_RENDER`], and the
/// caller substitutes that empty string — an empty section is better than a
/// section explaining that there is nothing in it.
pub fn render_lessons_block(lessons: &[CuratedLesson]) -> String {
    debug_assert!(
        lessons_are_path_free(),
        "curated lesson prose must carry no paths"
    );
    let mut out = String::new();
    let mut rendered = 0usize;
    for lesson in lessons {
        if rendered >= MAX_RENDERED_LESSONS {
            break;
        }
        if lesson.evidence.runs < MIN_RUNS_TO_RENDER {
            continue;
        }
        let line = format!("- {}\n", lesson.render());
        if out.len() + line.len() > MAX_LESSONS_BYTES {
            break;
        }
        out.push_str(&line);
        rendered += 1;
    }
    if out.is_empty() {
        return String::new();
    }
    format!(
        "WHAT PRIOR RUNS OF THIS PROJECT GOT WRONG. Distilled from finished runs — \
         these are rules, not history, and there is nothing to go and look up:\n{out}"
    )
}

/// The whole read side in one call: collect, merge, rank, render.
pub fn curated_lessons_block(store: &WorkflowStore, exclude_run_id: Option<&str>) -> String {
    render_lessons_block(&collect_curated_lessons(store, exclude_run_id))
}

#[cfg(test)]
#[path = "learning_lessons_tests.rs"]
mod tests;
