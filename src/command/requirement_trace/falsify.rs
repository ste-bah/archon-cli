//! `--falsify`: running a [`FalsificationPlan`] instead of only printing it.
//!
//! # Opt-in has to mean nothing happens without the flag
//!
//! Absent `--falsify` this module is never entered. No file is read for
//! mutation, no `git` runs, no process is spawned, and
//! [`AnchorVerdict::falsification_outcome`] stays `None` — which is skipped on
//! serialisation, so the JSON is byte-identical to what it was before this
//! module existed, and the text is byte-identical because the renderer only
//! prints an outcome line when there is an outcome. A read-only report that
//! could mutate a working tree as a side effect of being run would not be a
//! read-only report whatever its default was.
//!
//! # Why it lives in the command layer
//!
//! `archon-knowledge` names three inputs and reads all three. Mutation is a
//! write to someone's source file and a spawned build; putting that behind the
//! same crate boundary as the ladder would mean every future caller of a
//! traceability type links a crate that can rewrite files. What the crate does
//! contribute is the part that is proof semantics rather than mechanism:
//! `falsification::mutate` renders the mutant, and
//! `FalsificationOutcome::level_after` is the only thing that turns a result
//! into a [`ProofLevel`].
//!
//! # The order of the checks is the safety argument
//!
//! Everything that can refuse, refuses before anything is written: language,
//! command, stranded backup, file hash, renderability, working-tree cleanliness,
//! and finally the baseline run. Only then is a byte written. The baseline is
//! last because it is the expensive one and first because it must happen before
//! the mutation — a verifier that was already failing fails while mutated too,
//! and counting that as a kill would promote an edge on an unrelated breakage.
//!
//! # The criterion has two halves and this runs one command per half
//!
//! [`FalsificationPlan::pass_criterion`] requires the verifier to fail while
//! mutated *and* to pass again once the file is restored. The second half is
//! checked by comparing bytes rather than by a third run: the guard reads the
//! file back after restoring and requires it to equal the original exactly, and
//! a command that passed on those bytes minutes earlier passes on them again for
//! the same reason it did the first time. Two runs, not three, and the half that
//! is not re-run is the half that byte equality already settles.

mod guard;
mod limits;
mod process_tree;
mod signals;
pub(crate) use limits::VerifyLimits;
pub(crate) use signals::Interrupted;
mod verifier;

use std::path::Path;

use archon_knowledge::traceability::anchors::file_hash;
use archon_knowledge::traceability::falsification::mutate;
use archon_knowledge::traceability::falsification::outcome::build_failure_markers;
use archon_knowledge::traceability::report::strongest_level;
use archon_knowledge::traceability::{
    FalsificationOutcome, FalsificationPlan, Inconclusive, RefusedToRun, TraceReport,
};

/// Run every plan in the report and fold the results back into the levels.
///
/// Progress goes to stderr because the report goes to stdout: this can take
/// tens of minutes, and a caller piping the report to a file should still see
/// what is being mutated while it happens.
#[cfg(test)]
pub(crate) fn execute_plans(cwd: &Path, report: &mut TraceReport) {
    execute_plans_with_policy(cwd, report, None, &VerifyLimits::for_test()).unwrap();
}

pub(crate) fn execute_plans_with_policy(
    cwd: &Path,
    report: &mut TraceReport,
    policy: Option<&archon_workflow::acceptance_check_environment::CheckPolicy>,
    limits: &VerifyLimits,
) -> Result<(), Interrupted> {
    for row in &mut report.rows {
        for verdict in &mut row.anchors {
            let Ok(plan) = verdict.falsification.clone() else {
                continue;
            };
            eprintln!(
                "falsify {}: breaking {}:{}-{}, then `{}`",
                plan.requirement_id, plan.file_path, plan.line_start, plan.line_end, plan.command
            );
            let outcome = match attempt_with_policy(cwd, &plan, policy, limits) {
                Ok(outcome) => outcome,
                Err(AttemptError::Refused(reason)) => FalsificationOutcome::Refused(reason),
                Err(AttemptError::Interrupted(interruption)) => return Err(interruption),
                Err(AttemptError::Signal(_)) => unreachable!("monitor converts pending signals"),
            };
            eprintln!("  {}", outcome.describe());
            verdict.level = outcome.level_after(verdict.level);
            verdict.falsification_outcome = Some(outcome);
        }
        row.level = strongest_level(&row.anchors);
    }
    Ok(())
}

/// One experiment. `Err` is a refusal made before anything was written.
#[cfg(test)]
fn attempt(
    cwd: &Path,
    plan: &FalsificationPlan,
) -> std::result::Result<FalsificationOutcome, RefusedToRun> {
    attempt_with_policy(cwd, plan, None, &VerifyLimits::for_test()).map_err(|error| match error {
        AttemptError::Refused(reason) => reason,
        AttemptError::Interrupted(interruption) => panic!("{interruption}"),
        AttemptError::Signal(_) => unreachable!("monitor converts pending signals"),
    })
}

#[derive(Debug)]
enum AttemptError {
    Refused(RefusedToRun),
    Interrupted(Interrupted),
    Signal(i32),
}

impl From<RefusedToRun> for AttemptError {
    fn from(reason: RefusedToRun) -> Self {
        Self::Refused(reason)
    }
}

fn attempt_with_policy(
    cwd: &Path,
    plan: &FalsificationPlan,
    policy: Option<&archon_workflow::acceptance_check_environment::CheckPolicy>,
    limits: &VerifyLimits,
) -> std::result::Result<FalsificationOutcome, AttemptError> {
    let mut monitor = match signals::Monitor::new() {
        Ok(monitor) => monitor,
        Err(error) => return Ok(handler_failure(&plan.command, "registration", error)),
    };
    let result = attempt_monitored(cwd, plan, policy, limits);
    let restored = monitor.restore();
    if let Some(signal) = monitor.pending().or_else(|| match &result {
        Err(AttemptError::Signal(signal)) => Some(*signal),
        _ => None,
    }) {
        if let Err(error) = restored {
            eprintln!("interrupted verifier handler restoration failed: {error}");
        }
        return Err(AttemptError::Interrupted(
            monitor.interruption(signal, limits.reraise_default),
        ));
    }
    if let Err(error) = restored {
        return Ok(handler_failure(&plan.command, "restoration", error));
    }
    result
}

fn handler_failure(command: &str, operation: &str, error: std::io::Error) -> FalsificationOutcome {
    FalsificationOutcome::Inconclusive(Inconclusive::VerifierNotLaunchable {
        reason: format!("interrupt handler {operation} for `{command}`: {error}"),
    })
}

fn attempt_monitored(
    cwd: &Path,
    plan: &FalsificationPlan,
    policy: Option<&archon_workflow::acceptance_check_environment::CheckPolicy>,
    limits: &VerifyLimits,
) -> std::result::Result<FalsificationOutcome, AttemptError> {
    let language = mutate::language_for_path(&plan.file_path).ok_or_else(no_mutation(plan))?;
    let replacement = plan
        .mutation
        .replacement_for(language)
        .ok_or_else(no_mutation(plan))?;
    let argv = verifier::vet(&plan.command, limits)?;

    let path = cwd.join(&plan.file_path);
    let backup = guard::backup_path(&path);
    if backup.exists() {
        return Err(RefusedToRun::UnreconciledBackup {
            backup_path: backup.display().to_string(),
        }
        .into());
    }

    let original = std::fs::read(&path).map_err(|err| RefusedToRun::FileUnreadable {
        file_path: plan.file_path.clone(),
        reason: err.to_string(),
    })?;
    // The anchor was fresh when the row was built, but time passed between that
    // hash and this one — a concurrent editor, a rebase, a formatter on save.
    if file_hash(&original) != plan.expected_file_hash {
        return Err(RefusedToRun::FileChangedSincePlan {
            file_path: plan.file_path.clone(),
        }
        .into());
    }
    let text = String::from_utf8(original.clone()).map_err(|_| RefusedToRun::UnusableRange {
        file_path: plan.file_path.clone(),
        reason: "the file is not valid UTF-8, so a line range cannot be replaced in it".to_string(),
    })?;
    let mutant = mutate::render_mutant(&text, plan.line_start, plan.line_end, replacement)
        .map_err(|err| RefusedToRun::UnusableRange {
            file_path: plan.file_path.clone(),
            reason: err.describe(),
        })?;

    refuse_if_dirty(cwd, &plan.file_path)?;

    // Baseline and mutant must receive exactly the same captured variables.
    // Output text never decides whether a failure is an environment problem.
    let environment = match archon_workflow::acceptance_check_environment::CommandEnvironment::capture(policy) {
        Ok(environment) => environment.with_remedy("Ask the operator to allowlist needed names in [workflow.acceptance_execution] environment_allowlist in the configuration consumed by requirements trace"),
        Err(reason) => return Ok(FalsificationOutcome::Inconclusive(Inconclusive::VerifierNotLaunchable { reason: format!("`{}`: {reason}", plan.command) })),
    };
    match verifier::run_in_environment(cwd, &argv, limits, &environment) {
        verifier::Ran::Interrupted { signal } => return Err(AttemptError::Signal(signal)),
        verifier::Ran::DiskFloorBeforeLaunch(floor) => return Err(floor.refusal().into()),
        verifier::Ran::DiskFloor(floor) => {
            return Ok(FalsificationOutcome::Inconclusive(floor.inconclusive()));
        }
        verifier::Ran::NotLaunchable { reason } => {
            return Ok(FalsificationOutcome::Inconclusive(
                Inconclusive::VerifierNotLaunchable { reason },
            ));
        }
        verifier::Ran::TimedOut { seconds } => {
            return Ok(FalsificationOutcome::Inconclusive(Inconclusive::TimedOut {
                seconds,
            }));
        }
        verifier::Ran::Finished {
            code,
            success,
            note,
            ..
        } if !success => {
            return Err(RefusedToRun::BaselineDidNotPass {
                command: plan.command.clone(),
                exit_code: code,
                note,
            }
            .into());
        }
        verifier::Ran::Finished { .. } => {}
    }

    // Past this line the working tree is modified. Every path out of it goes
    // through the guard.
    let mut installed =
        guard::MutationGuard::install(&path, original, mutant.as_bytes()).map_err(|err| {
            RefusedToRun::FileUnreadable {
                file_path: plan.file_path.clone(),
                reason: format!("could not install the mutation: {err}"),
            }
        })?;
    let mutated = verifier::run_in_environment(cwd, &argv, limits, &environment);
    // Explicit, so the restore happens before the outcome is reported rather
    // than at the end of the enclosing scope. `Drop` remains the net.
    if let Err(err) = installed.restore() {
        eprintln!("  restore of {} failed: {err}", plan.file_path);
    }

    if let verifier::Ran::Interrupted { signal } = mutated {
        return Err(AttemptError::Signal(signal));
    }
    Ok(classify(language, mutated))
}

/// What a mutated run means.
fn classify(language: &str, ran: verifier::Ran) -> FalsificationOutcome {
    let note = match &ran {
        verifier::Ran::Finished { note, .. } => note.clone(),
        _ => None,
    };
    let outcome = classify_result(language, ran);
    match note {
        Some(note) => FalsificationOutcome::WithDiagnostics {
            outcome: Box::new(outcome),
            note,
        },
        None => outcome,
    }
}

fn classify_result(language: &str, ran: verifier::Ran) -> FalsificationOutcome {
    match ran {
        verifier::Ran::Interrupted { .. } => {
            unreachable!("interruption handled before classification")
        }
        verifier::Ran::DiskFloor(floor) | verifier::Ran::DiskFloorBeforeLaunch(floor) => {
            FalsificationOutcome::Inconclusive(floor.inconclusive())
        }
        verifier::Ran::NotLaunchable { reason } => {
            FalsificationOutcome::Inconclusive(Inconclusive::VerifierNotLaunchable { reason })
        }
        verifier::Ran::TimedOut { seconds } => {
            FalsificationOutcome::Inconclusive(Inconclusive::TimedOut { seconds })
        }
        verifier::Ran::Finished {
            code,
            success: true,
            ..
        } => FalsificationOutcome::EdgeIsDecoration { mutated_exit: code },
        verifier::Ran::Finished { code, output, .. } => match build_failure_markers(language) {
            None => FalsificationOutcome::Inconclusive(Inconclusive::UnclassifiableFailure {
                language: language.to_string(),
            }),
            Some(markers) => match markers.iter().find(|marker| output.contains(**marker)) {
                Some(marker) => {
                    FalsificationOutcome::Inconclusive(Inconclusive::MutantDidNotBuild {
                        marker: (*marker).to_string(),
                    })
                }
                None => FalsificationOutcome::DependencyShown { mutated_exit: code },
            },
        },
    }
}

/// Refuse a file with uncommitted changes, and refuse a file whose state cannot
/// be established at all.
///
/// Fail-closed in both directions: an untracked file is "dirty" here even though
/// its bytes could be restored, because a mutation that lands in a file with no
/// committed original is indistinguishable, to the person reading the diff
/// afterwards, from work they had not saved.
fn refuse_if_dirty(cwd: &Path, file_path: &str) -> std::result::Result<(), RefusedToRun> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--porcelain", "--"])
        .arg(file_path)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(err) => {
            return Err(RefusedToRun::CleanlinessUnknown {
                file_path: file_path.to_string(),
                reason: format!("git is not available: {err}"),
            });
        }
    };
    if !output.status.success() {
        return Err(RefusedToRun::CleanlinessUnknown {
            file_path: file_path.to_string(),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let status = String::from_utf8_lossy(&output.stdout);
    let status = status.trim();
    if status.is_empty() {
        return Ok(());
    }
    Err(RefusedToRun::DirtyFile {
        file_path: file_path.to_string(),
        status: status.lines().next().unwrap_or(status).to_string(),
    })
}

fn no_mutation(plan: &FalsificationPlan) -> impl FnOnce() -> RefusedToRun + '_ {
    move || RefusedToRun::NoMutationForLanguage {
        file_path: plan.file_path.clone(),
    }
}

#[cfg(test)]
mod tests;
