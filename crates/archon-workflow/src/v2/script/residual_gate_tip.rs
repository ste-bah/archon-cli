//! A gap no verifier judged since it was recorded (of any severity: every
//! standing gap blocks, Batch O), judged on the host's OWN run at the final
//! tip.
//!
//! A gap a round could not fix -- its fix landed nothing, so no verifier
//! judged it -- may already be fixed on the tree by another landing (live on
//! wf-0ddadd81: a contest remediation restored the regressed derivation).
//! The host checks what it can itself; it never overrides a verifier:
//!
//! - only a gap no verifier judged since it was recorded is judged here (the
//!   gate passes `unjudged`: its round had no verifier agent after its fix
//!   and no later verifier kept it open or recorded it again);
//! - a restatement of a refused task is never answered by a test run;
//! - the gap must name test ids of its recorder's runs, and every test id it
//!   names must be owed (`HostRuns::tip_owed`: the runs it names, every red
//!   run that named no test id, and a restated branch's red runs);
//! - a gap an AGENT recorded also needs the existing corroboration -- a later
//!   ACCEPTED verifier whose host runs passed everything its recorder left
//!   red (`superseded_by`); only a gap the host built itself
//!   (`Residual::host_built`: a routed red test; never judged by its id)
//!   may be answered by the tip run alone;
//! - on the host's own tip verdict of each owed command (run by
//!   `regression_gate` in a throwaway worktree at the tip, cached per
//!   commit; [`tip_owed_commands`] adds EVERY owed command, uncapped), every
//!   owed test must be named passed, and
//!   neither failing nor ignored; an owed command that named no test must
//!   have passed outright.
//!
//! Answered, the gap is a note that says a claim no test covers cannot be
//! checked this way; otherwise it blocks, saying why.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::super::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use super::super::superseded::HostRuns;
use super::super::{Residual, residuals_of};
use crate::v2::write::test_baseline_run_base::{Tree, cached, host_runnable};

/// What the host's own tip run says of a gap.
pub(super) enum TipJudgment {
    /// Every owed test passed by id at the tip.
    Answered(String),
    /// A test it owes is still red there, or has no verdict.
    StillRed(String),
}

/// The host's tip runs, and the records their gaps' recorders are read from.
pub(super) struct TipRuns<'a> {
    store: &'a WorkflowV2ResultStore,
    host: &'a HostRuns,
    tip: Option<String>,
    records: BTreeMap<String, WorkflowV2CallRecord>,
}

impl<'a> TipRuns<'a> {
    pub(super) fn load(
        store: &'a WorkflowV2ResultStore,
        host: &'a HostRuns,
        repository_root: Option<&Path>,
    ) -> Self {
        let tip = repository_root.and_then(|root| crate::repository_record::git_head(root).ok());
        let records = store
            .load_call_records()
            .unwrap_or_default()
            .into_iter()
            .map(|record| (record.call.id.clone(), record))
            .collect();
        Self {
            store,
            host,
            tip,
            records,
        }
    }

    /// Whether a verifier AGENT that finished after `recorder` recorded
    /// `residual` again or reported it open.
    fn rejudged(&self, residual: &Residual, recorder: &WorkflowV2CallRecord) -> bool {
        let since = super::super::finished(recorder);
        self.records.values().any(|record| {
            record.call.id != recorder.call.id
                && record.invalidated_by.is_none()
                && record.call.method != super::super::super::WorkflowV2HostMethod::Checkpoint
                && super::super::super::remediation_contract_string(&record.call, "stage")
                    == Some("verify")
                && super::super::finished(record) > since
                && (super::super::dispositions::disposition_of(record, &residual.id)
                    == Some(super::super::dispositions::Disposition::Open)
                    || residuals_of(record, None).iter().any(|again| {
                        super::super::dispositions::same_gap(
                            residual,
                            &again.id,
                            &again.description,
                        )
                    }))
        })
    }

    /// The judgment on `residual`, or `None` when the tip run cannot judge
    /// it (it is then weighed as before).
    pub(super) fn judge(&self, residual: &Residual) -> Option<TipJudgment> {
        let host_gap = residual.host_built;
        let recorder = self.records.get(&residual.recorded_by)?;
        let owed = self.host.tip_owed(residual, recorder);
        if owed.restated || owed.named.is_empty() || self.rejudged(residual, recorder) {
            return None;
        }
        let owed_ids: BTreeSet<&String> = owed.by_command.values().flatten().collect();
        let unowed: Vec<&str> = owed
            .named
            .iter()
            .filter(|test| !owed_ids.contains(test))
            .map(String::as_str)
            .collect();
        if !unowed.is_empty() {
            return Some(TipJudgment::StillRed(format!(
                "it names test(s) its recorder's runs do not show red ({}), so no test run answers it",
                unowed.join(", ")
            )));
        }
        if !host_gap && self.host.superseded_by(residual, recorder, None).is_none() {
            return Some(TipJudgment::StillRed(
                "no later accepted verifier's host runs passed everything its recorder left red, and a test run alone never answers a gap a verifier agent recorded".into(),
            ));
        }
        let Some(tip) = self.tip.as_deref() else {
            return Some(TipJudgment::StillRed(
                "the final tip is unreadable, so the host could not run the tests it owes".into(),
            ));
        };
        let at: String = tip.chars().take(9).collect();
        let mut red: Vec<String> = Vec::new();
        for (command, tests) in &owed.by_command {
            let verdict =
                cached(self.store, Tree::RunBase, tip, command).filter(|verdict| verdict.ids_kept);
            let Some(verdict) = verdict else {
                red.push(format!(
                    "the host's own run of `{command}` at the final tip {at} gave no verdict (only plain test-runner commands run)"
                ));
                continue;
            };
            if tests.is_empty() {
                if verdict.exit_code != Some(0) || verdict.failed_count.unwrap_or(0) > 0 {
                    red.push(format!(
                        "`{command}` is still red on the host's own run at the final tip {at}"
                    ));
                }
                continue;
            }
            let unpassed: Vec<&str> = tests
                .iter()
                .filter(|test| {
                    !verdict.passed_tests.contains(test)
                        || verdict.failing_tests.contains(test)
                        || verdict.ignored_tests.contains(test)
                })
                .map(String::as_str)
                .collect();
            if !unpassed.is_empty() {
                red.push(format!(
                    "{} still not passed on the host's own run of `{command}` at the final tip {at}",
                    unpassed.join(", ")
                ));
            }
        }
        if !red.is_empty() {
            return Some(TipJudgment::StillRed(red.join("; ")));
        }
        let listed: Vec<String> = owed
            .by_command
            .iter()
            .map(|(command, tests)| {
                if tests.is_empty() {
                    format!("`{command}` passed")
                } else {
                    format!(
                        "{} (`{command}`)",
                        tests.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                }
            })
            .collect();
        Some(TipJudgment::Answered(format!(
            "residual gap {} is answered at the final tip {at}: no verifier judged it since it was recorded, and the host's own run passed, by id, everything it owes: {} (a claim of it no test covers cannot be checked by a test run)",
            residual.label(),
            listed.join("; ")
        )))
    }
}

impl super::ResidualVerdict {
    pub(super) fn weigh(&mut self, residual: &Residual, why: &str) {
        let files = if residual.files.is_empty() {
            String::new()
        } else {
            format!(" on {}", residual.files.join(", "))
        };
        // Every standing gap blocks: a MEDIUM one is work a verifier saw
        // undone as surely as a HIGH one (Batch O), never a warning.
        self.blocking.push(format!(
            "residual gap {}{files} stands: {why}",
            residual.label()
        ));
    }

    /// [`Self::weigh`], a gap no verifier judged since it was recorded
    /// (`unjudged`) first judged on the host's own tip run: answered there,
    /// it is a note; still red, the clause says so. A gap a verifier kept
    /// open or recorded again is never overridden by a test run. Only a gap
    /// the host built itself (`Residual::host_built`, a routed red test --
    /// never judged by its id) may be answered by the tip run alone.
    pub(super) fn weigh_at_tip(
        &mut self,
        residual: &Residual,
        why: &str,
        tip: &TipRuns<'_>,
        unjudged: bool,
    ) {
        if !unjudged {
            return self.weigh(residual, why);
        }
        match tip.judge(residual) {
            Some(TipJudgment::Answered(note)) => self.notes.push(note),
            Some(TipJudgment::StillRed(red)) => self.weigh(residual, &format!("{why}; {red}")),
            None => self.weigh(residual, why),
        }
    }
}

/// Every host-runnable command a gap owes on the tip (`tip_owed`), for any
/// verifier's gaps of any severity and every routed failure's gap, sorted
/// and uncapped: what the final gate runs at the tip (each run is cached per
/// commit, so a resume never repeats one).
pub fn tip_owed_commands(
    store: &WorkflowV2ResultStore,
    repository_root: Option<&Path>,
) -> Vec<String> {
    let host = HostRuns::load(store);
    let stored = store.load_call_records().unwrap_or_default();
    let mut commands: BTreeSet<String> = BTreeSet::new();
    let mut owe = |residual: &Residual, record: &WorkflowV2CallRecord| {
        commands.extend(host.tip_owed(residual, record).by_command.into_keys());
    };
    for record in &stored {
        for residual in residuals_of(record, repository_root) {
            owe(&residual, record);
        }
    }
    let everything: Vec<&WorkflowV2CallRecord> = stored.iter().collect();
    for residual in super::super::owed::routed_gaps(
        &everything,
        &stored,
        &BTreeSet::new(),
        &host,
        None,
        repository_root,
    ) {
        if let Some(record) = stored.iter().find(|r| r.call.id == residual.recorded_by) {
            owe(&residual, record);
        }
    }
    commands
        .into_iter()
        .filter(|command| host_runnable(command))
        .collect()
}

#[cfg(test)]
#[path = "residual_gate_tip_tests.rs"]
pub(in crate::v2::script::residual_plan) mod tests;
