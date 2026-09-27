//! A HIGH gap judged on the host's OWN run at the final tip.
//!
//! A gap a round could not fix -- its fix landed nothing, so no verifier
//! judged it -- may already be fixed on the tree by another landing (live on
//! wf-0ddadd81: a contest remediation restored the regressed derivation;
//! the residual round then had nothing to change). Blocking on it would
//! report a gap the host can check itself, and the rule is that the harness
//! fixes, and only a gap still red may block.
//!
//! At the final gate the host has run, at the tip commit in a throwaway
//! worktree (`regression_gate`, which also runs every command a HIGH gap
//! owes: [`tip_owed_commands`]), each declared plain test-runner command,
//! under the Issue-118 rules: `--no-fail-fast`, a verdict only when every
//! test binary reported. A gap whose owed tests (`owed_tests`: the red tests
//! of its recorder's host runs, by command and id) all appear PASSED by id,
//! and not failing, in the host's own tip verdict of their command is
//! answered: a note, not a block. A gap owing a test the tip run left red,
//! or gave no verdict for, stands, saying so. A gap owing no test at all --
//! it names none the host ran red -- is judged as before: no test run can
//! answer it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::super::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use super::super::superseded::HostRuns;
use super::super::{Residual, residuals_of};
use crate::v2::write::test_baseline_run_base::{Tree, cached, host_runnable};

/// Most extra commands the final gate runs at the tip for HIGH gaps.
pub const MAX_TIP_OWED_COMMANDS: usize = 16;

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

    /// The judgment on `residual`, or `None` when it owes no test.
    pub(super) fn judge(&self, residual: &Residual) -> Option<TipJudgment> {
        let recorder = self.records.get(&residual.recorded_by)?;
        let owed = self.host.owed_tests(residual, recorder);
        if owed.is_empty() {
            return None;
        }
        let Some(tip) = self.tip.as_deref() else {
            return Some(TipJudgment::StillRed(
                "the final tip is unreadable, so the host could not run the tests it owes".into(),
            ));
        };
        let at: String = tip.chars().take(9).collect();
        let mut red: Vec<String> = Vec::new();
        for (command, tests) in &owed {
            let verdict =
                cached(self.store, Tree::RunBase, tip, command).filter(|verdict| verdict.ids_kept);
            let Some(verdict) = verdict else {
                red.push(format!(
                    "the host's own run of `{command}` at the final tip {at} gave no verdict"
                ));
                continue;
            };
            let unpassed: Vec<&str> = tests
                .iter()
                .filter(|test| {
                    !verdict.passed_tests.contains(test) || verdict.failing_tests.contains(test)
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
        if red.is_empty() {
            let named: Vec<String> = owed
                .iter()
                .map(|(command, tests)| {
                    format!(
                        "{} (`{command}`)",
                        tests.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect();
            return Some(TipJudgment::Answered(format!(
                "residual gap {} is answered at the final tip {at}: the host's own run passed, by id, every test it owes: {}",
                residual.label(),
                named.join("; ")
            )));
        }
        Some(TipJudgment::StillRed(red.join("; ")))
    }
}

/// Every host-runnable command a HIGH gap any verifier recorded owes a test
/// in, sorted, at most [`MAX_TIP_OWED_COMMANDS`]: what the final gate runs at
/// the tip so no such gap blocks unjudged.
pub fn tip_owed_commands(
    store: &WorkflowV2ResultStore,
    repository_root: Option<&Path>,
) -> Vec<String> {
    let host = HostRuns::load(store);
    let mut commands: BTreeSet<String> = BTreeSet::new();
    for record in store.load_call_records().unwrap_or_default() {
        for residual in residuals_of(&record, repository_root)
            .into_iter()
            .filter(|residual| residual.severity == super::super::ResidualSeverity::High)
        {
            commands.extend(host.owed_tests(&residual, &record).into_keys());
        }
        for routed in host.routed(&record.call.id) {
            commands.insert(routed.command.clone());
        }
    }
    commands
        .into_iter()
        .filter(|command| host_runnable(command))
        .take(MAX_TIP_OWED_COMMANDS)
        .collect()
}

#[cfg(test)]
#[path = "residual_gate_tip_tests.rs"]
mod tests;
