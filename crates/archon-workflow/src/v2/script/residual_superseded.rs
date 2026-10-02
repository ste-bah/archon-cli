//! Issue-121: a HIGH gap a refused verifier recorded, answered by the host's
//! own later run of the tests it names.
//!
//! A verifier that refused may record a HIGH gap the host never planned for
//! (on a live run: a second-pass round's verifier refused and named a
//! regression that left three declared must-pass tests red). The final gate
//! counts such a gap, and the third residual pass plans a round for it,
//! unless the host's OWN record shows it no longer holds: the base-commit
//! baseline the host runs for every focused verification stage
//! (`write::test_baseline`, established before the stage's items are
//! dispatched) names, per declared command, the tests that failed.
//!
//! A gap is answerable only when its text names (as a whole identifier) a
//! failing test of the recording stage's host runs, or it is the host's own
//! restatement of a refused task (`record_landing`: the gap's id is the
//! task's); a gap naming no such test is never answered here: only a round
//! that carried it can resolve it. What it owes is then EVERY red command of
//! the recorder's host runs with every test it named failing -- a red test
//! of another command, named by no gap, is still red on that tree. The gap
//! is answered when a verifier AGENT that accepted, and started after the
//! recorder finished, has a host run of each owed command that passed
//! outright (exit 0, no timeout, no error) AND whose runner named each owed
//! test PASSED by id, AND the latest host run of each such command since the
//! recorder -- by any stage, verifier or write wave -- did the same: a later
//! red run is the gap back, and a test renamed away or `#[ignore]`d inside a
//! command that still exits 0 answers nothing. A run recorded before the
//! host kept passed ids (`passed_ids_kept` false) is read as it always was,
//! passed outright, consistently everywhere.
//!
//! `before` bounds the evidence to stages that started before a moment (the
//! third pass passes its first round's start), so a plan never moves once
//! its rounds run: a round's own verifier answers its gaps through the round,
//! never by rewriting which gaps the round holds.

use std::collections::{BTreeMap, BTreeSet};

use super::super::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use super::{Residual, accepted_verdict, finished};
use crate::v2::write::test_baseline::{BranchBaseline, RoutedFailure, all_records};

/// The host's base-commit runs by stage, when each stage started (a stage
/// with no record yet is still running: the latest of all), and every
/// accepted verifier agent.
pub(super) struct HostRuns {
    by_stage: BTreeMap<String, Vec<BranchBaseline>>,
    started: BTreeMap<String, i64>,
    accepted: BTreeSet<String>,
}

impl HostRuns {
    pub(super) fn load(store: &WorkflowV2ResultStore) -> Self {
        let mut by_stage: BTreeMap<String, Vec<BranchBaseline>> = BTreeMap::new();
        for record in all_records(store) {
            by_stage
                .entry(record.stage_id.clone())
                .or_default()
                .push(record);
        }
        let records = store.load_call_records().unwrap_or_default();
        let started = records
            .iter()
            .map(|record| (record.call.id.clone(), started(record)))
            .collect();
        let accepted = records
            .iter()
            .filter(|record| accepted_verdict(record))
            .map(|record| record.call.id.clone())
            .collect();
        Self {
            by_stage,
            started,
            accepted,
        }
    }

    fn started(&self, stage: &str) -> i64 {
        self.started.get(stage).copied().unwrap_or(i64::MAX)
    }

    /// What `residual` (recorded by `recorder`) owes on the final tip
    /// (`residual_gate_tip`), by command: the red runs of the branches it
    /// restates (its id is their task's), every run whose failing ids its
    /// text names (as whole identifiers), and every red run of the recorder
    /// that named no test id (a lint, a build) -- with every id each run
    /// named failing. `named` is every test id of the recorder's runs,
    /// failing or passing, the text names.
    pub(super) fn tip_owed(&self, residual: &Residual, recorder: &WorkflowV2CallRecord) -> TipOwed {
        let mut owed = TipOwed::default();
        let Some(runs) = self.by_stage.get(&recorder.call.id) else {
            return owed;
        };
        for branch in runs {
            let restated = branch.canonical_task_ids.contains(&residual.id);
            owed.restated |= restated;
            for run in &branch.commands {
                for test in run.failing_tests.iter().chain(&run.passed_tests) {
                    if names(&residual.description, test) {
                        owed.named.insert(test.clone());
                    }
                }
                let named_red = run
                    .failing_tests
                    .iter()
                    .any(|test| names(&residual.description, test));
                let idless = run.failing_tests.is_empty();
                if !run.passed() && (restated || named_red || idless) {
                    owed.by_command
                        .entry(run.command.clone())
                        .or_default()
                        .extend(run.failing_tests.iter().cloned());
                }
            }
        }
        owed
    }

    /// The later accepted verifier whose host runs passed, by id, every
    /// test the recorder's host runs left red, if any.
    pub(super) fn superseded_by(
        &self,
        residual: &Residual,
        recorder: &WorkflowV2CallRecord,
        before: Option<i64>,
    ) -> Option<String> {
        let runs = self.by_stage.get(&recorder.call.id)?;
        // The gap must name a red test of the recorder's host runs (or be the
        // host's own restatement of a refused task): a gap naming none is
        // never answered here.
        let named = runs.iter().any(|branch| {
            let restated = branch.canonical_task_ids.contains(&residual.id);
            branch.commands.iter().any(|run| {
                run.failing_tests
                    .iter()
                    .any(|test| restated || names(&residual.description, test))
            })
        });
        if !named {
            return None;
        }
        // Every command the recorder's host runs left red, with every test
        // it named failing -- not only the ones this gap names, and a red
        // command that named none too: a red test or check of another
        // command is still red on the tree the gap was recorded against, and
        // nothing about this gap is answered while it is. (Unchanged, so a
        // pass whose rounds already ran never moves on an upgrade.)
        let mut owed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for run in runs.iter().flat_map(|branch| &branch.commands) {
            if !run.passed() {
                owed.entry(run.command.clone())
                    .or_default()
                    .extend(run.failing_tests.iter().cloned());
            }
        }
        // A red test named in a run that exited 0: the record contradicts
        // itself, and nothing it owes can be proven.
        if owed.is_empty() {
            return None;
        }
        let since = finished(recorder);
        let later: Vec<(i64, &str)> = self
            .by_stage
            .keys()
            .map(|stage| (self.started(stage), stage.as_str()))
            .filter(|(at, stage)| {
                *at > since && *stage != recorder.call.id && before.is_none_or(|cut| *at < cut)
            })
            .collect();
        // Passed outright AND its runner named each owed test passed: a test
        // renamed away or `#[ignore]`d inside a command that still exits 0
        // answers nothing.
        // A run recorded before the host kept passed ids names none, so it is
        // read as it always was -- passed outright -- everywhere, the same way
        // at every pass's slot, dispatch check and the final gate, so no plan
        // moves under a host upgrade. Every run recorded since answers only
        // with each owed test named passed.
        let passes = |stage: &str, command: &str, tests: &BTreeSet<String>| {
            self.by_stage[stage]
                .iter()
                .flat_map(|branch| &branch.commands)
                .any(|run| {
                    run.command == command
                        && (run.passed_by_id(tests.iter())
                            || (!run.passed_ids_kept && run.passed()))
                })
        };
        // The latest host run of each command since the recorder passed.
        let latest_green = owed.iter().all(|(command, tests)| {
            later
                .iter()
                .filter(|(_, stage)| {
                    self.by_stage[*stage]
                        .iter()
                        .flat_map(|branch| &branch.commands)
                        .any(|run| &run.command == command)
                })
                .max_by_key(|(at, _)| *at)
                .is_some_and(|(_, stage)| passes(stage, command, tests))
        });
        if !latest_green {
            return None;
        }
        later
            .iter()
            .filter(|(_, stage)| self.accepted.contains(*stage))
            .find(|(_, stage)| {
                owed.iter()
                    .all(|(command, tests)| passes(stage, command, tests))
            })
            .map(|(_, stage)| (*stage).to_string())
    }
}

impl HostRuns {
    /// Every failure the host's base-commit runs of `stage` routed to
    /// another task that declares its file.
    pub(super) fn routed(&self, stage: &str) -> Vec<&RoutedFailure> {
        self.by_stage
            .get(stage)
            .into_iter()
            .flatten()
            .flat_map(|branch| &branch.routed)
            .collect()
    }

    /// Whether the host's own runs since `since` (and before `before`)
    /// answer the red test `test` of `command`: the latest host run of the
    /// command, one that ran to a verdict, names it passed by id (the
    /// command's other tests are other gaps').
    pub(super) fn test_answered(
        &self,
        test: &str,
        command: &str,
        since: i64,
        before: Option<i64>,
    ) -> bool {
        let wanted = test.to_string();
        let latest = self
            .by_stage
            .iter()
            .filter(|(stage, _)| {
                let at = self.started(stage);
                at > since && before.is_none_or(|cut| at < cut)
            })
            .flat_map(|(stage, branches)| {
                branches
                    .iter()
                    .flat_map(|branch| &branch.commands)
                    .filter(|run| run.command == command)
                    .map(move |run| (self.started(stage), run))
            })
            .max_by_key(|(at, _)| *at);
        // The answering run must be a recorded stage's: one with no record
        // yet (a stage still running, or killed) counts as the latest, so it
        // can take an answer back, but never gives one.
        latest.is_some_and(|(at, run)| {
            at != i64::MAX
                && !run.timed_out
                && run.error.is_none()
                && run.passed_tests.contains(&wanted)
                && !run.failing_tests.contains(&wanted)
        })
    }
}

/// What a gap owes on the final tip (`HostRuns::tip_owed`).
#[derive(Debug, Default)]
pub(super) struct TipOwed {
    pub(super) restated: bool,
    pub(super) named: BTreeSet<String>,
    pub(super) by_command: BTreeMap<String, BTreeSet<String>>,
}

/// Whether `text` names `test` as a whole identifier (a `::` path counts as
/// one).
fn names(text: &str, test: &str) -> bool {
    let test = test.trim();
    if test.is_empty() {
        return false;
    }
    let part = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == ':';
    text.match_indices(test).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + test.len()..].chars().next();
        !before.is_some_and(part) && !after.is_some_and(part)
    })
}

pub(super) fn started(record: &WorkflowV2CallRecord) -> i64 {
    chrono::DateTime::parse_from_rfc3339(&record.started_at)
        .ok()
        .and_then(|at| at.timestamp_nanos_opt())
        .unwrap_or(i64::MIN)
}

#[cfg(test)]
#[path = "residual_answered_tests.rs"]
mod answered_tests;

#[cfg(test)]
mod tests {
    use super::names;

    #[test]
    fn a_test_is_named_only_as_a_whole_identifier() {
        let text = "leaves complete_artifacts_precede_registry_commit red (tests/a.rs:140)";
        assert!(names(text, "complete_artifacts_precede_registry_commit"));
        assert!(!names(text, "artifacts_precede"));
        assert!(!names(text, "complete_artifacts_precede_registry"));
        assert!(!names("store::tests::stale_x", "stale"));
        assert!(names("see store::tests::stale.", "store::tests::stale"));
        assert!(!names(text, " "));
    }
}
