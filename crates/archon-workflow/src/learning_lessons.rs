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
//! # Why the call records and not the stage records
//!
//! An earlier version of this module distilled [`crate::learning`]'s stage
//! records. It could not work, and the reason is structural rather than a
//! tuning problem: `StageState::artifacts` has **no production writer at all**
//! — `WorkflowStore::write_artifact` is called from two tests and nowhere else
//! — so `artifact_count` is 0 and `durable` is false on every stage of every
//! run ever recorded (0 of 3143 stages across 40 runs carry one). Any rule
//! keyed on those fields is not a weak rule, it is a constant. The stage kind
//! is no better: a v3 `implement()` call becomes a `fanout` stage, so no stage
//! anywhere has ever had the `implementation` kind, and roughly a third of a
//! run's stages have no spec entry to read a kind from at all.
//!
//! The v2 call records carry what actually happened — `write_mode` says whether
//! a call was allowed to write, and `files_changed`, `commands_run`,
//! `task_coverage`, `residual_gaps` and `completion_evidence` say what it did.
//! Every rule below was checked against a real run's call records before being
//! written, and each one both fires and stays silent on that data.
//!
//! So this module distils the call records into **curated lessons**: a fixed
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

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{RunStatus, WorkflowRun};
use crate::store::WorkflowStore;
use crate::v2::result::WorkflowV2Status;
// `result_store_records.rs` is `include!`d into `result_store`, so the record
// types live in that module rather than a module of their own.
use crate::v2::result_store::WorkflowV2CallRecord;

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
    /// A call allowed to write was accepted having changed no file and run no
    /// command.
    SilentImplementation,
    /// A call claimed a no-op without the coverage or evidence that proves it.
    UnprovenNoop,
    /// A call was accepted while still carrying unresolved residual gaps.
    AcceptedWithGaps,
    /// One task consumed more calls than a clean implement-and-verify pair.
    RepeatedTaskCycles,
    /// The run ended with declared tasks that never reached an accepted call.
    UnfinishedRun,
}

impl LessonRule {
    /// Every rule, for exhaustive iteration in tests and rendering order.
    pub const ALL: [Self; 5] = [
        Self::SilentImplementation,
        Self::UnprovenNoop,
        Self::AcceptedWithGaps,
        Self::RepeatedTaskCycles,
        Self::UnfinishedRun,
    ];

    /// The kind of work this lesson bears on. Deliberately one of a fixed set
    /// of generic labels — never a task id or a call id.
    pub fn scope(self) -> &'static str {
        match self {
            Self::SilentImplementation | Self::UnprovenNoop => "implementation",
            Self::AcceptedWithGaps => "acceptance",
            Self::RepeatedTaskCycles | Self::UnfinishedRun => "plan",
        }
    }

    /// One line naming the failure. No numbers: lessons merge across runs and
    /// a baked-in count would be stale after the first merge.
    pub fn headline(self) -> &'static str {
        match self {
            Self::SilentImplementation => {
                "A call allowed to write was accepted having changed nothing and run nothing."
            }
            Self::UnprovenNoop => "A no-op was claimed without the evidence that proves it.",
            Self::AcceptedWithGaps => "Work was accepted while still carrying unresolved findings.",
            Self::RepeatedTaskCycles => {
                "Single tasks consumed many calls before reaching a verdict."
            }
            Self::UnfinishedRun => {
                "The run ended with declared tasks that never reached an accepted call."
            }
        }
    }

    /// What to do differently. Addressed to the author of the next script, in
    /// terms of the dialect it is writing, not of any particular project.
    pub fn guidance(self) -> &'static str {
        match self {
            Self::SilentImplementation => {
                "Treat an outcome with empty files_changed and empty commands_run as FAILED and \
                 send it to remediation. A cheerful summary is not evidence; the only exception \
                 is an explicit typed no-op carrying task_coverage that proves the work already \
                 existed."
            }
            Self::UnprovenNoop => {
                "A task claimed as already-implemented must carry the file or test output that \
                 shows it. Record task_coverage and at least one piece of evidence on every \
                 no-op, or the claim is indistinguishable from work never done."
            }
            Self::AcceptedWithGaps => {
                "Decide each residual gap before accepting. A gap carried past acceptance is \
                 never revisited, so it silently becomes part of the delivered result — either \
                 remediate it, or state plainly that it is being accepted and why."
            }
            Self::RepeatedTaskCycles => {
                "State the acceptance condition inside the implementing call, not only in the \
                 verification that follows it. Work verified against a condition it never saw \
                 fails, and each failure costs a remediation cycle as expensive as the original \
                 attempt. Check the task's declared deliverables are satisfiable as written \
                 before implementing against them."
            }
            Self::UnfinishedRun => {
                "Plan for the whole task set to finish. Batch every independent task into one \
                 wave, keep the call count proportionate to the work, and account for each task \
                 exactly once — a plan that never reaches its later tasks teaches nothing about \
                 them."
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
    /// Host calls examined, summed across those runs.
    ///
    /// A magnitude indicator, not a strict denominator: it counts every call in
    /// the run, while some rules count only the calls — or the tasks — they
    /// apply to. It is here so a reader can tell one bad call in three from one
    /// in three hundred, not so the two numbers can be divided.
    pub calls: usize,
}

impl LessonEvidence {
    fn merge(&mut self, other: &Self) {
        self.runs += other.runs;
        self.occurrences += other.occurrences;
        self.calls += other.calls;
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
            "[{}] {} {} (seen in {} run{}, {} of {} calls)",
            self.rule.scope(),
            self.rule.headline(),
            self.rule.guidance(),
            self.evidence.runs,
            if self.evidence.runs == 1 { "" } else { "s" },
            self.evidence.occurrences,
            self.evidence.calls,
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

/// A task needing more calls than this has stopped converging.
///
/// Four, not two. Two is the ideal — one call implements, one verifies — but
/// recovering from a negative verdict costs two more (remediate, re-verify) and
/// that recovery is the system working, not failing. A threshold of two would
/// have flagged every task that needed a single remediation, which on the run
/// this was calibrated against is half of them: a rule that fires on the normal
/// case teaches nothing. Above four, a task has been round-tripped more than
/// once and is not converging.
pub const MAX_CLEAN_CYCLES: usize = 4;

/// Whether the host allowed this call to modify the repository.
///
/// `write_mode` is set by the host from the call itself, so it says "this was
/// an implementation" without reading a stage name, a task id, or a language.
/// It is the signal the stage-derived version lacked.
fn is_write_capable(record: &WorkflowV2CallRecord) -> bool {
    record.call.write_mode.is_some()
}

/// Canonical task ids this call concerned, from the source graph it carried and
/// the completion evidence it produced.
fn task_ids(record: &WorkflowV2CallRecord) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    if let Some(graph) = record.source_task_graph.as_ref() {
        for item in &graph.items {
            ids.extend(item.canonical_task_ids.iter().cloned());
        }
    }
    ids.extend(
        record
            .completion_evidence
            .iter()
            .map(|evidence| evidence.task_id.clone()),
    );
    ids
}

/// The declared task universe, as any call that carried it reports it.
///
/// Every call stamps the same universe, so the first one that carries a
/// non-empty list is authoritative and there is nothing to reconcile.
fn task_universe(records: &[WorkflowV2CallRecord]) -> BTreeSet<String> {
    records
        .iter()
        .filter_map(|record| record.source_task_graph.as_ref())
        .map(|graph| &graph.canonical_task_universe)
        .find(|universe| !universe.is_empty())
        .map(|universe| universe.iter().cloned().collect())
        .unwrap_or_default()
}

/// Distil a run's call records into curated lessons.
///
/// Side-effect free, and deterministic in everything that is read back: the
/// same records always yield the same rules with the same counts. Only `ts` is
/// wall-clock, and nothing ranks or renders on a single lesson's `ts` — it
/// exists to order whole runs in [`collect_curated_lessons`]. That is what lets
/// a resumed run rewrite the file wholesale without double-counting itself.
pub fn distil_lessons(run: &WorkflowRun, records: &[WorkflowV2CallRecord]) -> Vec<CuratedLesson> {
    let calls = records.len();
    if calls == 0 {
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
                    calls,
                },
                run,
            ));
        }
    };

    // A write-capable call accepted with nothing changed AND nothing run. Both
    // halves are required: a call that ran the test suite and correctly changed
    // nothing is a legitimate no-op, and flagging it would teach the next run
    // to make pointless edits.
    let silent = records
        .iter()
        .filter(|record| {
            is_write_capable(record)
                && record.status == WorkflowV2Status::Accepted
                && record.result.files_changed.is_empty()
                && record.result.commands_run.is_empty()
        })
        .count();
    push(LessonRule::SilentImplementation, silent);

    let unproven = records
        .iter()
        .filter(|record| {
            record.status == WorkflowV2Status::Noop
                && record.result.task_coverage.is_empty()
                && record.result.evidence.is_empty()
        })
        .count();
    push(LessonRule::UnprovenNoop, unproven);

    let with_gaps = records
        .iter()
        .filter(|record| {
            record.status == WorkflowV2Status::Accepted && !record.result.residual_gaps.is_empty()
        })
        .count();
    push(LessonRule::AcceptedWithGaps, with_gaps);

    // Counted per task rather than per call: the occurrence being reported is
    // "a task that churned", not "a call that happened".
    let mut per_task: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        for id in task_ids(record) {
            *per_task.entry(id).or_default() += 1;
        }
    }
    let churning = per_task
        .values()
        .filter(|count| **count > MAX_CLEAN_CYCLES)
        .count();
    push(LessonRule::RepeatedTaskCycles, churning);

    if run.status != RunStatus::Completed {
        let accepted: BTreeSet<String> = records
            .iter()
            .filter(|record| record.status == WorkflowV2Status::Accepted)
            .flat_map(task_ids)
            .collect();
        let never_accepted = task_universe(records)
            .into_iter()
            .filter(|id| !accepted.contains(id))
            .count();
        push(LessonRule::UnfinishedRun, never_accepted);
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
