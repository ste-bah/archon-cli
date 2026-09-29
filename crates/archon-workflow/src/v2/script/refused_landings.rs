//! Batch L (Invariant L1): a remediation its verifier did not accept leaves
//! nothing behind that a later acceptance round could pass on.
//!
//! # The false green this ends
//!
//! A remediation's fix lands before its verifier runs, and nothing took the
//! landing back out when the verifier refused it. Live, a unit registered a
//! dangling entry in a project data index to satisfy an acceptance check;
//! its verifier refused the fix (`needs_review`), the entry stayed in the
//! project's data, and the next acceptance round could pass on exactly what
//! was refused.
//!
//! # Every way a remediation's effects land
//!
//! Every landing goes through `patch_apply::apply_wave` (the worktree and the
//! coordinated write paths both call it), item by item in `apply_one`:
//!
//! 1. its patch to tracked files, applied (`apply_git::apply_patch`) and
//!    committed by the host (`wave_commit::commit_wave_outputs`): one commit
//!    per landing, named by its stage -- reverted by a host revert commit
//!    ([`commits`]);
//! 2. its declared ignored deliverables, copied where they are verified
//!    (`materialize::materialize`), logged in `materializations.jsonl` --
//!    put back from the state each copy replaced (`refused_revert`);
//! 3. its captured project-input changes (`project_inputs_apply::apply_judged`)
//!    and 4. the project's copies of tracked inputs its patch changed
//!    (`project_inputs_apply::sync_tracked`), both logged in
//!    `project-inputs.jsonl` -- put back the same way.
//!
//! A serial-mode write (`write::serial`) writes straight into the checkout
//! with no landing and no pre-image: a refused one cannot be reverted, and
//! is raised as a finding that holds acceptance.
//!
//! # When
//!
//! The verdict is recorded after the landing, so the revert follows it: at
//! the start of every acceptance round, over the whole run
//! ([`revert_refused_landings`], the first thing the acceptance stage does),
//! which covers a resume too -- a refused record whose landing is still in
//! the tree is reverted before any check runs. Between one remediation round
//! and the next of the same unit nothing is reverted: the escalated and
//! re-verified rounds (Issue-107, Issue-111) are built on the tree the
//! refused round left, and a later round of the unit that is accepted judged
//! that tree, which makes the earlier landing accepted work (`plan`).
//!
//! Only units the acceptance stage routed are in scope, and only on an
//! actual verdict: a landing whose verification was interrupted or never ran
//! is pending, never reverted.
//!
//! The pass is idempotent, keyed by what `refused-landings.jsonl` and the
//! revert commits' trailers record, and never touches a landing a verdict
//! accepted, whenever it landed. A revert that would overwrite a later change
//! that stands is a conflict: nothing is written, the refused landing stays,
//! and the conflict is a HIGH finding that holds acceptance. Every decision
//! is logged, and a later attempt at the same tasks is given each refusal as
//! a finding ([`refused_landings_preamble`]).

mod commits;
mod ledger;
mod plan;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub use ledger::{RefusedLandingRevert, refused_landing_reverts, refused_landings_preamble};
use plan::{Judged, Standing, Unit, landing_place};

use super::WorkflowV2ResultStore;
use crate::write_coordinator::patch_apply::{
    DataRevert, ProjectInputLanding, revert_copy, revert_input, run_materializations,
    run_project_input_landings,
};

/// What one pass reverted, and what it could not.
#[derive(Debug, Default)]
pub struct RevertReport {
    /// Every decision this pass logged.
    pub decisions: Vec<RefusedLandingRevert>,
    /// HIGH findings: refused landings still in the tree, each named.
    pub findings: Vec<String>,
}

/// Revert every refused landing of the run still in the tree.
pub fn revert_refused_landings(
    store: &WorkflowV2ResultStore,
    repository_root: Option<&Path>,
) -> RevertReport {
    revert(store, repository_root)
}

/// One refused landing, with the unit and verdict that refused it.
struct Refused<'a> {
    unit: &'a Unit<'a>,
    fix: &'a Judged<'a>,
    verdict: String,
    summary: String,
}

impl Refused<'_> {
    fn decision(&self, kind: &str, landing: String, paths: Vec<String>) -> RefusedLandingRevert {
        RefusedLandingRevert {
            at: chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            kind: kind.into(),
            landing,
            paths,
            fix_call_id: self.fix.record.call.id.clone(),
            verdict_call_id: self.verdict.clone(),
            task_ids: self.unit.task_ids.clone(),
            verdict: self.summary.clone(),
            outcome: String::new(),
            revert_commit: String::new(),
            reason: String::new(),
        }
    }
}

struct Pass<'a> {
    run_root: &'a Path,
    logged: Vec<RefusedLandingRevert>,
    report: RevertReport,
}

impl Pass<'_> {
    fn done(&self, landing: &str) -> bool {
        self.logged
            .iter()
            .any(|line| line.landing == landing && line.done())
    }

    /// Log `decision` (a conflict only when it is news) and report it.
    fn record(&mut self, decision: RefusedLandingRevert) {
        if !decision.done() {
            self.report.findings.push(format!(
                "refused remediation landing {} of {} (tasks {}) is still in the tree: {}. Its verdict {} did not accept it, so no acceptance round may pass on it; a person must resolve it",
                decision.landing,
                decision.fix_call_id,
                decision.task_ids.join(", "),
                decision.reason,
                decision.verdict_call_id
            ));
        }
        let repeated = self
            .logged
            .iter()
            .rev()
            .find(|line| line.landing == decision.landing)
            .is_some_and(|line| line.outcome == decision.outcome && line.reason == decision.reason);
        if !repeated {
            if let Err(error) = ledger::append(self.run_root, &decision) {
                self.report.findings.push(format!(
                    "the refused-landing log could not be written for {}: {error}",
                    decision.landing
                ));
            }
            self.logged.push(decision.clone());
        }
        self.report.decisions.push(decision);
    }
}

fn revert(store: &WorkflowV2ResultStore, root: Option<&Path>) -> RevertReport {
    let run_root = store.run_root();
    let records = match store.load_call_records() {
        Ok(records) => records,
        Err(error) => {
            return RevertReport {
                findings: vec![format!(
                    "refused remediation landings could not be checked: the call records are unreadable: {error}"
                )],
                ..RevertReport::default()
            };
        }
    };
    let units = plan::units(&records);
    let candidates: Vec<&Unit<'_>> = units.values().filter(|unit| unit.may_refuse()).collect();
    if candidates.is_empty() {
        return RevertReport::default();
    }
    let mut pass = Pass {
        run_root,
        logged: Vec::new(),
        report: RevertReport::default(),
    };
    match ledger::refused_landing_reverts(run_root) {
        Ok(logged) => pass.logged = logged,
        Err(error) => {
            pass.report.findings.push(format!(
                "refused remediation landings could not be checked: {error}"
            ));
            return pass.report;
        }
    }
    let fixes: BTreeMap<&str, (&Unit<'_>, &Judged<'_>)> = candidates
        .iter()
        .flat_map(|unit| {
            unit.fixes
                .iter()
                .map(move |fix| (fix.record.call.id.as_str(), (*unit, fix)))
        })
        .collect();
    let refused = |stage: &str, logged: Option<i64>| -> Option<Refused<'_>> {
        let (unit, fix) = fixes.get(stage)?;
        let (verdict, summary) = match unit.standing(landing_place(fix, logged)) {
            Standing::Refused { verdict, summary } => (verdict, summary),
            // Unjudged (its verification was interrupted or never ran) is
            // not refused: it waits for its verdict.
            Standing::Pending => return None,
            Standing::Stands => return None,
        };
        Some(Refused {
            unit,
            fix,
            verdict,
            summary,
        })
    };
    for unit in &candidates {
        for (fix, standing) in unit.refused_serial_fixes() {
            let Standing::Refused { verdict, summary } = standing else {
                continue;
            };
            let refusal = Refused {
                unit,
                fix,
                verdict,
                summary,
            };
            let mut decision = refusal.decision("serial", fix.record.call.id.clone(), Vec::new());
            decision.outcome = "unrevertable".into();
            decision.reason = "it wrote in serial mode, straight into the checkout, and no landing recorded what it replaced".into();
            pass.record(decision);
        }
    }
    // Under the lock every landing takes: no landing moves the tree, the
    // project data or the copy order while a refused one is taken out.
    match root {
        Some(root) => {
            let locked = crate::write_coordinator::with_repo_lock(root, || {
                let failed = revert_commits(&mut pass, root, &store.run_id(), &refused);
                revert_data(&mut pass, &refused, &failed);
                Ok(())
            });
            if let Err(error) = locked {
                pass.report.findings.push(format!(
                    "refused remediation landings could not be reverted: the repository write lock was not taken: {error}"
                ));
            }
        }
        None => revert_data(&mut pass, &refused, &BTreeSet::new()),
    }
    pass.report
}

type RefusedOf<'r, 'a> = dyn Fn(&str, Option<i64>) -> Option<Refused<'a>> + 'r;

/// Revert every refused landing commit still in the tree, newest first;
/// the fixes whose commit could not be reverted.
fn revert_commits(
    pass: &mut Pass<'_>,
    root: &Path,
    run_id: &str,
    refused: &RefusedOf<'_, '_>,
) -> BTreeSet<String> {
    let mut failed = BTreeSet::new();
    let commits = match commits::host_commits(root, run_id) {
        Ok(commits) => commits,
        Err(error) => {
            pass.report.findings.push(format!(
                "refused remediation landings could not be checked: the run's commits are unreadable: {error}"
            ));
            return failed;
        }
    };
    let reverted: BTreeSet<&str> = commits
        .iter()
        .filter_map(|c| c.reverts.as_deref())
        .collect();
    let mut due = Vec::new();
    for (at, commit) in commits.iter().enumerate().filter(|(_, c)| !c.revert) {
        if pass.done(&commit.sha) {
            continue;
        }
        // A commit's time is its second: its landing is the whole second.
        let Some(refusal) = refused(&commit.stage, Some(commit.at + 999_999_999)) else {
            continue;
        };
        if reverted.contains(commit.sha.as_str()) {
            // Reverted by an earlier pass that stopped before logging it.
            let mut decision = refusal.decision("commit", commit.sha.clone(), Vec::new());
            decision.outcome = "already_reverted".into();
            pass.record(decision);
            continue;
        }
        due.push((at, commit, refusal));
    }
    let taken: BTreeSet<&str> = due
        .iter()
        .map(|(_, commit, _)| commit.sha.as_str())
        .collect();
    for (at, commit, refusal) in &due {
        let mut decision = refusal.decision("commit", commit.sha.clone(), Vec::new());
        // A later landing that stands and touched the same files was judged
        // on a tree holding this one: taking it out from under that work
        // could break accepted work, so that is a conflict for a person.
        let later = commits[..*at].iter().filter(|later| {
            !later.revert
                && !taken.contains(later.sha.as_str())
                && !reverted.contains(later.sha.as_str())
                && !pass.done(&later.sha)
        });
        let result = commits::overlapping_later(root, commit, later).and_then(|over| match over {
            Some(reason) => Err(reason),
            None => commits::revert_commit(root, run_id, commit, &refusal.verdict),
        });
        match result {
            Ok((sha, paths)) => {
                decision.outcome = if sha.is_empty() {
                    "already_reverted"
                } else {
                    "reverted"
                }
                .into();
                decision.paths = paths;
                decision.revert_commit = sha;
            }
            Err(reason) => {
                decision.outcome = "conflict".into();
                decision.reason = reason;
                failed.insert(refusal.fix.record.call.id.clone());
            }
        }
        pass.record(decision);
    }
    failed
}

/// Whether the project-input line at `index` put its state there itself: a
/// line that found its state "already in place" did only when the nearest
/// earlier decision on that path is its own interrupted apply (an intent of
/// the same landing and bytes), never when another landing put it there.
fn placed_by_itself(lines: &[ProjectInputLanding], index: usize) -> bool {
    let line = &lines[index];
    if line.reason != "already in place" {
        return true;
    }
    let own_intent = |other: &ProjectInputLanding| {
        other.outcome == "intent"
            && other.stage_id == line.stage_id
            && other.item_id == line.item_id
            && other.after == line.after
    };
    let mut earlier = lines[..index]
        .iter()
        .rev()
        .filter(|other| other.path == line.path && other.outcome != "refused");
    // Its own intent, logged just before it applied.
    if !earlier.next().is_some_and(own_intent) {
        return false;
    }
    earlier.next().is_some_and(own_intent)
}

fn revert_data(pass: &mut Pass<'_>, refused: &RefusedOf<'_, '_>, failed: &BTreeSet<String>) {
    let run_root = pass.run_root;
    let lines = match run_project_input_landings(run_root) {
        Ok(lines) => lines,
        Err(error) => {
            pass.report.findings.push(format!(
                "refused remediation landings could not be checked: the project input log is unreadable: {error}"
            ));
            Vec::new()
        }
    };
    for (index, line) in lines.iter().enumerate().rev() {
        // A fix whose commit stays keeps its data with it: never half out.
        if !line.landed() || failed.contains(&line.stage_id) || !placed_by_itself(&lines, index) {
            continue;
        }
        let landing = format!(
            "{}/{}:{}@{}",
            line.stage_id, line.item_id, line.path, line.at
        );
        if pass.done(&landing) {
            continue;
        }
        let Some(refusal) = refused(&line.stage_id, Some(line.at)) else {
            continue;
        };
        let why = format!("refused by {}", refusal.verdict);
        let outcome = revert_input(run_root, line, &why);
        pass.record(settle(
            refusal.decision("project_input", landing, vec![line.path.clone()]),
            outcome,
        ));
    }
    let copies = match run_materializations(run_root) {
        Ok(copies) => copies,
        Err(error) => {
            pass.report.findings.push(format!(
                "refused remediation landings could not be checked: the copy log is unreadable: {error}"
            ));
            return;
        }
    };
    let mut copies: Vec<_> = copies.into_iter().filter(|copy| !copy.reverted).collect();
    copies.sort_by_key(|copy| std::cmp::Reverse(copy.receipt.sequence));
    for copy in &copies {
        let landing = format!(
            "{}/{}:{}#{}",
            copy.stage_id, copy.item_id, copy.path, copy.receipt.sequence
        );
        if pass.done(&landing) || failed.contains(&copy.stage_id) {
            continue;
        }
        let Some(refusal) = refused(&copy.stage_id, Some(copy.at)) else {
            continue;
        };
        let why = format!("refused by {}", refusal.verdict);
        let outcome = revert_copy(run_root, copy, &why);
        pass.record(settle(
            refusal.decision("copy", landing, vec![copy.path.clone()]),
            outcome,
        ));
    }
}

fn settle(mut decision: RefusedLandingRevert, outcome: DataRevert) -> RefusedLandingRevert {
    match outcome {
        DataRevert::Reverted { .. } => decision.outcome = "reverted".into(),
        DataRevert::Already => decision.outcome = "already_reverted".into(),
        DataRevert::Conflict(reason) => {
            decision.outcome = "conflict".into();
            decision.reason = reason;
        }
    }
    decision
}

#[cfg(test)]
#[path = "refused_landings_tests.rs"]
mod tests;
