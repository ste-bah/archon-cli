//! Batch O2 (CUT-11b): each regression the host's own check found before a
//! residual pass (`verification::regression_slot`), routed to its owner as a
//! host-planned round of that pass.
//!
//! One HIGH gap per regression, built by the host (`host_built`), whose text
//! is stable across passes -- the command, the test and its file, never a
//! commit -- so a regression that persists keeps its identity: an earlier
//! pass's round that carried it is the one that answered it, and a later
//! pass never plans it again as new (a round left open is retried by the
//! pass rules like any other). It is routed as every gap is (`route`: the
//! tasks declaring the test's file, or an expansion of the tasks declaring
//! the command); a regression that names no file (a command with no verdict
//! at the tip) is a round of the tasks declaring the command. The
//! post-script regression gate stays the final judge of every one: the
//! residual gate leaves them to it (`is_regression_gap`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::residual_paths::TaskTexts;
use super::super::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use super::{
    MAX_GAPS_PER_ROUND, PlannedRound, Residual, ResidualSeverity, RoundKind, round, route,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::regression_compare::RegressionFinding;
use crate::v2::verification::regression_slot::{SlotFinding, slot_record};

/// Id prefix of a host regression gap.
pub const REGRESSION_GAP_ID: &str = "host_regression";
/// Who records every host regression gap: one name for every pass, so a
/// regression that persists keeps its identity (`Residual::key`).
pub const REGRESSION_RECORDER: &str = "host-regression-check";

/// Whether `residual` is a regression gap the host built.
pub fn is_regression_gap(residual: &Residual) -> bool {
    residual.host_built && residual.id.starts_with(REGRESSION_GAP_ID)
}

/// The slot record of pass `pass` among `stored`: its latest checkpoint.
fn slot_of(stored: &[WorkflowV2CallRecord], pass: u64) -> Option<&WorkflowV2CallRecord> {
    stored
        .iter()
        .filter(|record| super::slot_pass(&record.call) == Some(pass))
        .max_by_key(|record| super::finished(record))
}

/// Pass `pass`'s regression gaps, less every gap `known` (carried or
/// reported by an earlier pass, or by this pass's other rounds).
pub(super) fn regression_gaps(
    store: &WorkflowV2ResultStore,
    stored: &[WorkflowV2CallRecord],
    universe: &WorkflowV2TaskUniverse,
    pass: u64,
    known: &BTreeSet<String>,
) -> Vec<Residual> {
    let Some(slot) = slot_of(stored, pass).and_then(|record| slot_record(store, &record.call.id))
    else {
        return Vec::new();
    };
    let mut gaps: Vec<Residual> = slot
        .findings
        .iter()
        .map(|finding| gap(finding, universe, &slot.call_id))
        .chain(
            slot.unjudged
                .as_deref()
                .map(|why| unjudged(why, &slot.call_id)),
        )
        .filter(|gap| !known.contains(&gap.key()))
        .collect();
    gaps.sort_by_key(Residual::key);
    gaps.dedup_by_key(|gap| gap.key());
    gaps
}

/// m2: a check that could not judge the tree is surfaced as a gap of its
/// own (no file, no owner: reported, and left to the final regression gate,
/// which judges the run again), never silently nothing.
fn unjudged(why: &str, by: &str) -> Residual {
    Residual {
        recorded_by: REGRESSION_RECORDER.to_string(),
        id: format!("{REGRESSION_GAP_ID}@unjudged"),
        severity: ResidualSeverity::High,
        description: format!(
            "The host's own regression check could not judge the tree before this residual pass: {why}."
        ),
        files: Vec::new(),
        unit_tasks: BTreeSet::new(),
        recorded_summary: format!("host regression check before residual pass slot `{by}`"),
        host_built: true,
    }
}

/// One regression, built from the slot's RECORD alone (M2): its command,
/// test, file and failure files as the check recorded them -- the live
/// tree is never read here, so the gap is the same on every resume.
fn gap(found: &SlotFinding, universe: &WorkflowV2TaskUniverse, by: &str) -> Residual {
    let finding = &found.finding;
    let command = finding.command();
    let what = match finding {
        RegressionFinding::NewFailure { test, .. } => {
            format!("`{test}` fails now and did not fail at the run's base commit")
        }
        RegressionFinding::Vanished { test, .. } => format!(
            "`{test}` passed at the run's base commit and is no longer reported: it was removed or renamed, and a test that is gone proves nothing passes"
        ),
        RegressionFinding::Hidden { test, .. } => format!(
            "`{test}` passed at the run's base commit and is ignored now: it was hidden, not fixed"
        ),
        RegressionFinding::NoTipVerdict { .. } => "the command gives no verdict now (a build failure, a timeout, or a runner that did not report every test binary)".to_string(),
        _ => "the command fails now with failures its runner does not name, which the run's base commit did not".to_string(),
    };
    let mut files: Vec<String> = found
        .file
        .iter()
        .chain(&found.failure_files)
        .cloned()
        .collect();
    let mut seen = BTreeSet::new();
    files.retain(|file| seen.insert(file.clone()));
    // Only the test's own file is named in the text (it is stable across
    // passes); the failure locations route it too.
    let place = found
        .file
        .as_ref()
        .map(|file| format!(" The test lives in {file}."))
        .unwrap_or_default();
    let label = finding.test().map_or_else(
        || format!("command:{}", &digest(command)[..8]),
        str::to_string,
    );
    Residual {
        recorded_by: REGRESSION_RECORDER.to_string(),
        id: format!("{REGRESSION_GAP_ID}@{label}"),
        severity: ResidualSeverity::High,
        description: format!(
            "The host's own regression check ran the declared test command `{command}` at the run's base commit and again before this residual pass: {what}.{place} Make it pass again without weakening or removing the test."
        ),
        files,
        unit_tasks: universe
            .tasks
            .iter()
            .filter(|task| task.focused_tests.iter().any(|c| c.trim() == command))
            .map(|task| task.canonical_task_id.clone())
            .collect(),
        recorded_summary: format!("host regression check before residual pass slot `{by}`"),
        host_built: true,
    }
}

/// The rounds for `gaps`, one per owner set, at most a handful of gaps each.
pub(super) fn regression_rounds(
    gaps: Vec<Residual>,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    rekey: impl Fn(PlannedRound) -> PlannedRound,
    reported: &mut Vec<(Residual, String)>,
) -> Vec<PlannedRound> {
    let texts = TaskTexts::read(universe, root);
    let ids: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect();
    let mut groups: BTreeMap<Vec<String>, Vec<(Residual, BTreeSet<String>)>> = BTreeMap::new();
    for mut gap in gaps {
        gap.unit_tasks.retain(|task| ids.contains(task));
        match route(&gap, universe, root, &texts) {
            Ok((tasks, files)) => groups
                .entry(tasks.into_iter().collect())
                .or_default()
                .push((gap, files)),
            // No file to route by: the tasks declaring the command answer.
            Err(_) if !gap.unit_tasks.is_empty() => groups
                .entry(gap.unit_tasks.iter().cloned().collect())
                .or_default()
                .push((gap, BTreeSet::new())),
            Err(why) => reported.push((gap, why)),
        }
    }
    let mut rounds = Vec::new();
    for (tasks, members) in groups {
        for chunk in members.chunks(MAX_GAPS_PER_ROUND) {
            let files: BTreeSet<String> = chunk.iter().flat_map(|(_, f)| f.clone()).collect();
            let kind = if files.is_empty() {
                RoundKind::Owned
            } else {
                RoundKind::Expansion
            };
            let gaps: Vec<Residual> = chunk.iter().map(|(g, _)| g.clone()).collect();
            rounds.push(rekey(round(kind, tasks.clone(), files, gaps, None, None)));
        }
    }
    rounds.sort_by(|a, b| a.key.cmp(&b.key));
    rounds
}

fn digest(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
#[path = "residual_regression_tests.rs"]
mod tests;
