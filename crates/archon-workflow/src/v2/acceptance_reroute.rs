//! A failed check no task was named to fix is reassigned, never reported and
//! dropped (A13, hole 12; H3 inside acceptance).
//!
//! Owners came only from `implements` lists, and a check the routing rules
//! could give no unit was marked `blocked` and raised as a finding no round
//! was ever sent for. Here every failed check that RAN and names no owning
//! task, or that the rules blocked, is given one, in this order:
//!
//! 1. the tasks of the landing shown to break it, and the tasks able to
//!    write a file its failure implicates (`routing.writer_tasks`);
//! 2. otherwise the tasks NEAREST the files its failure implicates: those
//!    declaring a path sharing the longest leading directory with one of
//!    them;
//! 3. otherwise, when its failure names no file at all, every task of the
//!    set, together: a cross-owner unit.
//!
//! The chosen tasks become the check's `owning_tasks`, so the script routes
//! it like any owned check, and the routing records who it was reassigned
//! to and why. A check stays `blocked` only when the universe has no task.
//! A blocked check that already had owners keeps them: its implicated files
//! are ones no unit may be given here, and its owners are sent with that.

use std::collections::BTreeSet;

use super::super::acceptance_stage::AcceptanceRoundRecordV1;
use super::AcceptanceRoutingV1;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn components(path: &str) -> Vec<&str> {
    path.trim_start_matches("./")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

/// A declared entry as a path: its first token, without code quotes, a
/// leading `./` or a trailing glob.
fn entry_path(entry: &str) -> Option<String> {
    let token = entry
        .split_whitespace()
        .find(|token| token.contains('/') || token.contains('.'))?
        .trim_matches(|c| c == '`' || c == '"' || c == '\'' || c == ',');
    let token = token.strip_prefix("./").unwrap_or(token);
    let token = token.trim_end_matches("/**").trim_end_matches('/');
    (!token.is_empty() && !token.contains("://")).then(|| token.to_string())
}

fn declared(task: &WorkflowV2TaskUniverseTask) -> Vec<String> {
    (task.files_expected_to_change.iter())
        .chain(&task.shared_append_target_files)
        .filter_map(|entry| entry_path(entry))
        .chain(
            (task.deliverable_contracts.iter())
                .filter_map(|contract| entry_path(&contract.artifact_path)),
        )
        .collect()
}

/// Leading path components `file` shares with `entry`; a declared file that
/// IS `file` counts every component.
fn shared(file: &str, entry: &str) -> usize {
    let (file, entry) = (components(file), components(entry));
    file.iter().zip(&entry).take_while(|(a, b)| a == b).count()
}

/// The tasks declaring a path nearest one of `files`: the most leading
/// directories shared, at least one. Empty when no task shares any.
pub fn nearest_owners(universe: &WorkflowV2TaskUniverse, files: &[String]) -> BTreeSet<String> {
    let mut best = 0;
    let mut nearest = BTreeSet::new();
    for task in &universe.tasks {
        let score = declared(task)
            .iter()
            .flat_map(|entry| files.iter().map(move |file| shared(file, entry)))
            .max()
            .unwrap_or(0);
        if score == 0 || score < best {
            continue;
        }
        if score > best {
            best = score;
            nearest.clear();
        }
        nearest.insert(task.canonical_task_id.clone());
    }
    nearest
}

/// Reassign every failed check of `record` that names no owning task or
/// that the routing rules blocked (see the module docs). Run after
/// `mark_blocked`.
pub fn reroute(universe: Option<&WorkflowV2TaskUniverse>, record: &mut AcceptanceRoundRecordV1) {
    let Some(universe) = universe else {
        return;
    };
    let every: BTreeSet<String> = (universe.tasks.iter())
        .map(|task| task.canonical_task_id.clone())
        .collect();
    for check in &mut record.checks {
        if !check.ran_and_failed() || (!check.owning_tasks.is_empty() && check.blocked.is_none()) {
            continue;
        }
        let rule = check.blocked.take();
        if !check.owning_tasks.is_empty() {
            // Blocked with owners: every file it implicates is one no unit
            // may be given here. Its owners still hold it.
            let routing = check
                .routing
                .get_or_insert_with(AcceptanceRoutingV1::default);
            routing.reassigned_to = check.owning_tasks.clone();
            routing.reassign_reason = format!(
                "re-routed to its owners although {}",
                rule.as_deref().unwrap_or("no rule gave it a unit")
            );
            continue;
        }
        let mut to: BTreeSet<String> = (check.regressed_by.iter())
            .flat_map(|regression| regression.tasks.iter().cloned())
            .chain(
                check
                    .routing
                    .iter()
                    .flat_map(|r| r.writer_tasks.iter().cloned()),
            )
            .collect();
        let mut reason = match &rule {
            Some(rule) => format!("no routing rule gave it a unit ({rule})"),
            None => "no task's `implements` names it".to_string(),
        };
        if to.is_empty() {
            let files: Vec<String> = (check.routing.iter())
                .flat_map(|routing| {
                    (routing.implicated_files.iter().cloned())
                        .chain(routing.unwritable.iter().map(|(file, _)| file.clone()))
                })
                .collect();
            to = nearest_owners(universe, &files);
            if to.is_empty() {
                to.clone_from(&every);
                reason.push_str(
                    "; no task declares a path near what its failure implicates, so every task of the set holds it together",
                );
            } else {
                reason.push_str("; sent to the tasks nearest the files its failure implicates");
            }
        } else {
            reason.push_str("; sent to the landing that broke it and the writers of its files");
        }
        if to.is_empty() {
            check.blocked = Some(rule.unwrap_or_else(|| "the task universe has no task".into()));
            continue;
        }
        check.owning_tasks = to.iter().cloned().collect();
        let routing = check
            .routing
            .get_or_insert_with(AcceptanceRoutingV1::default);
        routing.reassigned_to = check.owning_tasks.clone();
        routing.reassign_reason = reason;
    }
}

#[cfg(test)]
#[path = "acceptance_reroute_tests.rs"]
mod tests;
