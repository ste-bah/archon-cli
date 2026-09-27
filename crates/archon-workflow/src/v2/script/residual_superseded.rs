//! Issue-121: a HIGH gap a refused verifier recorded, answered by the host's
//! own later run of the tests it names.
//!
//! A verifier that refused may record a HIGH gap the host never planned for
//! (live on wf-0ddadd81: a second-pass round's verifier refused and named a
//! regression that left three declared must-pass tests red). The final gate
//! counts such a gap, and the third residual pass plans a round for it,
//! unless the host's OWN record shows it no longer holds: the base-commit
//! baseline the host runs for every focused verification stage
//! (`write::test_baseline`, established before the stage's items are
//! dispatched) names, per declared command, the tests that failed.
//!
//! The gap's red tests are the failing tests of the recording stage's host
//! runs that its text names (as a whole identifier), and, for the host's own
//! restatement of a refused task (`record_landing`: the gap's id is the
//! task's), every failing test of that task's branch. The gap is answered
//! when a verifier AGENT that accepted, and started after the recorder
//! finished, has a host run of EVERY command those tests failed in that
//! passed outright (exit 0, no timeout, no error), AND the latest host run of
//! each such command since the recorder -- by any stage, verifier or write
//! wave -- passed too: a later red run is the gap back. A gap naming no such
//! test is never answered here: only a round that carried it can resolve it.
//!
//! `before` bounds the evidence to stages that started before a moment (the
//! third pass passes its first round's start), so a plan never moves once
//! its rounds run: a round's own verifier answers its gaps through the round,
//! never by rewriting which gaps the round holds.

use std::collections::{BTreeMap, BTreeSet};

use super::super::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use super::{Residual, accepted_verdict, finished};
use crate::v2::write::test_baseline::{BranchBaseline, all_records};

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

    /// The later accepted verifier whose host runs passed every command the
    /// red tests of `residual` (recorded by `recorder`) failed in, if any.
    pub(super) fn superseded_by(
        &self,
        residual: &Residual,
        recorder: &WorkflowV2CallRecord,
        before: Option<i64>,
    ) -> Option<String> {
        let runs = self.by_stage.get(&recorder.call.id)?;
        let mut commands: BTreeSet<&str> = BTreeSet::new();
        for branch in runs {
            let restated = branch.canonical_task_ids.contains(&residual.id);
            for run in &branch.commands {
                if run
                    .failing_tests
                    .iter()
                    .any(|test| restated || names(&residual.description, test))
                {
                    commands.insert(run.command.as_str());
                }
            }
        }
        if commands.is_empty() {
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
        let passes = |stage: &str, command: &str| {
            self.by_stage[stage]
                .iter()
                .flat_map(|branch| &branch.commands)
                .any(|run| run.command == command && run.passed())
        };
        // The latest host run of each command since the recorder passed.
        let latest_green = commands.iter().all(|command| {
            later
                .iter()
                .filter(|(_, stage)| {
                    self.by_stage[*stage]
                        .iter()
                        .flat_map(|branch| &branch.commands)
                        .any(|run| run.command == *command)
                })
                .max_by_key(|(at, _)| *at)
                .is_some_and(|(_, stage)| passes(stage, command))
        });
        if !latest_green {
            return None;
        }
        later
            .iter()
            .filter(|(_, stage)| self.accepted.contains(*stage))
            .find(|(_, stage)| commands.iter().all(|command| passes(stage, command)))
            .map(|(_, stage)| (*stage).to_string())
    }
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
