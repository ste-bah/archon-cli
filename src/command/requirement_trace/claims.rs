//! Decomposition-time claim falsification for the staged requirement trace.
//!
//! # Why the staged gate needs its own falsifier
//!
//! `--falsify` breaks anchored code and runs the declared verifier. Anchors
//! come from a code index, and at decomposition time there is none: the
//! staged gate refuses `--leann-db`, so every claimed row had no anchor, no
//! plan ran, and an `implements:` entry passed the set gate as if it were
//! proof. What CAN be falsified before any code is written is what the body
//! says the claim rests on: the files, symbols and commands its text ties to
//! the claimed obligation, against the repository the task set records at its
//! recorded base commit ([`index`]).
//!
//! # The three outcomes, and none of them is silence
//!
//! - **Refuted**: an observable is missing from the base commit and the
//!   checkout, and no task in the set declares it will create it -- or the
//!   task declares no file it may change, so no declared file can serve the
//!   claim. A `Body` finding routes the refutation to the task's body.
//! - **Untestable**: the body never names the obligation outside
//!   `implements:`, or everything it names is outside what the index covers.
//!   Also a `Body` finding, with the reason, so the author ties the claim to
//!   something observable.
//! - **Tested**: every observable exists or is derived from a declaration in
//!   the set. Reported per claim, with its counts.

mod body;
mod check;
mod index;
mod observe;
#[cfg(test)]
pub(crate) use body::body_claim_findings;
pub(crate) use body::body_claim_findings_at_path;
use body::untestable_claims;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_knowledge::traceability::TaskBinding;
use archon_workflow::repository_record::read_repository_record;

use super::verdict::TracePolicyFinding;
use check::{TaskScope, Verdict};
use index::EvidenceIndex;
use observe::{BodyLine, Span};

/// What the falsifier found: a report section and the findings it raises.
pub(super) struct ClaimTrace {
    pub(super) report: String,
    pub(super) findings: Vec<TracePolicyFinding>,
}

/// One tested observable of one claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Checked {
    subject: String,
    verdict: Verdict,
}

struct ClaimResult {
    task_id: String,
    source_path: PathBuf,
    requirement: String,
    checked: Vec<Checked>,
    /// Set when the claim cannot be tested at all, with the reason.
    untestable: Option<String>,
}

struct TaskInputs {
    binding: TaskBinding,
    raw: String,
    changeable: BTreeSet<String>,
    forbidden_conflicts: Vec<ForbiddenConflict>,
    outside: Vec<String>,
    /// Why the task-universe reader refused the file, when it did: its
    /// deliverables and forbidden files are then unknown.
    unread: Option<String>,
}

type ForbiddenConflict = archon_write_plan::ForbiddenDeclarationConflict;

/// Test every claim in `task_dir` against its recorded repository.
pub(super) fn falsify_claims(task_dir: &Path, bindings: Vec<TaskBinding>) -> Result<ClaimTrace> {
    let record = read_repository_record(task_dir)?.ok_or_else(|| {
        anyhow!(
            "task set {} has no {}; the staged requirement trace tests every claim against the repository the decomposition recorded, and without the record no claim can be tested",
            task_dir.display(),
            archon_workflow::repository_record::REPOSITORY_LOCK_FILE
        )
    })?;
    let mut index = EvidenceIndex::load(&record).context("loading the recorded repository")?;
    let mut tasks = Vec::with_capacity(bindings.len());
    let root = crate::command::topology_lint::scope_declarations::repository_root(
        Path::new(&index.root_text()),
        task_dir,
    );
    for binding in bindings {
        let raw = std::fs::read_to_string(&binding.source_path)
            .with_context(|| format!("reading task file {}", binding.source_path))?;
        tasks.push(task_inputs(&root, binding, raw));
    }
    for task in &tasks {
        for path in &task.changeable {
            index.declare(path, &task.binding.task_id);
        }
    }
    let results: Vec<ClaimResult> = tasks
        .iter()
        .filter(|task| task.unread.is_none())
        .flat_map(|task| {
            task.binding
                .implements
                .iter()
                .map(|requirement| test_claim(&index, task, requirement))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut policy_findings = findings(&index, &results);
    for task in &tasks {
        if let Some(error) = &task.unread {
            policy_findings.push(TracePolicyFinding {
                text: format!("{}: {error}; make the exact parser-required edit and re-run `workflow lint --task-file {}`", task.binding.source_path, task.binding.source_path),
                subject: task.binding.task_id.clone(),
                source_path: PathBuf::from(&task.binding.source_path),
                remediation_scope: archon_workflow::RemediationScope::Body,
            });
        }
    }
    Ok(ClaimTrace {
        report: render(&index, &results),
        findings: policy_findings,
    })
}

fn task_inputs(root: &Path, mut binding: TaskBinding, raw: String) -> TaskInputs {
    let path = Path::new(&binding.source_path);
    let mut universe = archon_workflow::task_universe::parsing::parse_task_file(path, &raw);
    if let Ok(task) = &mut universe {
        task.focused_commands = Some(
            binding
                .focused_tests
                .iter()
                .filter_map(|entry| match entry {
                    archon_knowledge::traceability::tasks::FocusedTestEntry::Command(command) => {
                        Some(command.clone())
                    }
                    _ => None,
                })
                .collect(),
        );
    }
    let unread = universe.as_ref().err().map(ToString::to_string);
    let universe = universe.ok();
    let mut declared: Vec<String> = binding.path_scopes.clone();
    let mut forbidden_bullets = Vec::new();
    if let Some(task) = &universe {
        binding.use_focused_commands(&task.declared_focused_test_commands());
        declared.extend(
            task.deliverable_contracts
                .iter()
                .map(|c| c.artifact_path.clone()),
        );
        declared.extend(task.shared_append_target_files.iter().cloned());
        forbidden_bullets = task.files_forbidden_to_change.clone();
    }
    let mut changeable = BTreeSet::new();
    let mut normalized_declared = Vec::new();
    let mut outside = Vec::new();
    for entry in declared {
        match archon_write_plan::normalize_declared_path(entry.trim(), root) {
            Some(relative) => {
                normalized_declared.push(relative.clone());
                changeable.insert(relative);
            }
            None => outside.push(entry),
        }
    }
    let forbidden_conflicts =
        archon_write_plan::declaration_conflicts(normalized_declared, &forbidden_bullets);
    changeable.retain(|path| {
        !forbidden_conflicts
            .iter()
            .any(|conflict| conflict.path == *path)
    });
    TaskInputs {
        binding,
        raw,
        changeable,
        forbidden_conflicts,
        outside,
        unread,
    }
}

fn runner_check(binding: &TaskBinding) -> impl Fn(&str) -> bool + '_ {
    |command| {
        binding.focused_tests.iter().any(|entry| {
            let archon_knowledge::traceability::tasks::FocusedTestEntry::Command(frozen) = entry
            else {
                return false;
            };
            (frozen.split_whitespace().eq(command.split_whitespace())
                || archon_workflow::task_universe::focused_commands::leading_program(frozen)
                    == Some(command))
                && archon_workflow::verifier_strength::verifier_strength_defect(
                    Some(frozen),
                    None,
                    None,
                )
                .is_none()
        })
    }
}

fn test_claim(index: &EvidenceIndex, task: &TaskInputs, requirement: &str) -> ClaimResult {
    let runners = runner_check(&task.binding);
    let scope = TaskScope {
        task_id: &task.binding.task_id,
        changeable: &task.changeable,
    };
    let mut result = ClaimResult {
        task_id: task.binding.task_id.clone(),
        source_path: PathBuf::from(&task.binding.source_path),
        requirement: requirement.to_string(),
        checked: Vec::new(),
        untestable: None,
    };
    let lines = observe::body_lines(&task.raw);
    let named: Vec<usize> = (0..lines.len())
        .filter(|&i| !lines[i].metadata && observe::mentions(&lines[i].text, requirement))
        .collect();
    let mut checked = BTreeSet::new();
    for conflict in &task.forbidden_conflicts {
        checked.insert(Checked {
            subject: format!("declared `{}`", conflict.path),
            verdict: Verdict::Refuted(conflict.finding()),
        });
    }
    if let Some((_, reason)) =
        untestable_claims(&task.raw, std::slice::from_ref(&result.requirement))
            .into_iter()
            .next()
    {
        result.checked = checked.into_iter().collect();
        result.untestable = Some(reason);
        return result;
    }
    if task.changeable.is_empty() && !task.outside.is_empty() {
        result.checked = checked.into_iter().collect();
        result.untestable = Some(body::outside_declarations_reason(&task.binding.task_id));
        return result;
    }
    for &at in &named {
        line_observables(index, &scope, &lines, at, &runners, &mut checked);
    }
    if let Some(reason) = &task.unread {
        checked.insert(Checked {
            subject: "declared files".into(),
            verdict: Verdict::Untested(format!(
                "the task universe reader refused the file ({reason}), so its deliverables and forbidden files are unknown"
            )),
        });
    } else if task.changeable.is_empty() && task.outside.is_empty() {
        let reason = if task.forbidden_conflicts.is_empty() {
            body::no_file_reason(&task.binding.task_id)
        } else {
            task.forbidden_conflicts
                .iter()
                .map(ForbiddenConflict::finding)
                .collect::<Vec<_>>()
                .join("; ")
        };
        checked.insert(Checked {
            subject: "declared files".into(),
            verdict: Verdict::Refuted(reason),
        });
    }
    for path in &task.changeable {
        checked.insert(Checked {
            subject: format!("declared `{path}`"),
            verdict: check::path(index, path),
        });
    }
    for entry in &task.outside {
        checked.insert(Checked {
            subject: format!("declared `{entry}`"),
            verdict: check::outside(entry),
        });
    }
    for verifier in &task.binding.verifier_commands {
        for subject in observe::command_subjects(&verifier.command, &runners) {
            checked.insert(Checked {
                subject: format!("verifier `{}`", verifier.command),
                verdict: check::command_subject(index, &verifier.command, &subject),
            });
        }
    }
    result.checked = checked.into_iter().collect();
    if !result
        .checked
        .iter()
        .any(|c| matches!(c.verdict, Verdict::Refuted(_)))
        && !result.checked.iter().any(|c| c.verdict.tested())
    {
        result.untestable = Some(body::outside_reason(requirement));
    }
    result
}

fn line_observables(
    index: &EvidenceIndex,
    scope: &TaskScope<'_>,
    lines: &[BodyLine],
    at: usize,
    runners: &dyn Fn(&str) -> bool,
    checked: &mut BTreeSet<Checked>,
) {
    let line = &lines[at];
    // A line that forbids, removes or reports a thing absent promises it
    // nothing; only its commands are still observables.
    let promises = observe::line_promises(line);
    let spans = observe::spans(&line.text, runners);
    let files: Vec<&str> = spans
        .iter()
        .filter_map(|span| match span {
            Span::Path {
                raw,
                directory: false,
                ..
            } => Some(raw.as_str()),
            _ => None,
        })
        .collect();
    let home = match files.as_slice() {
        [only] => Some((*only).to_string()),
        [] => observe::parent_path(lines, at, runners),
        _ => None,
    };
    let home = home
        .and_then(|raw| index.relative(&raw))
        .filter(|rel| index.is_repository_path(rel));
    for span in spans {
        let (subject, verdict) = match span {
            Span::Path { raw, anchor, .. } => {
                let Some(relative) = index.relative(&raw) else {
                    checked.insert(Checked {
                        subject: format!("`{raw}`"),
                        verdict: check::outside(&raw),
                    });
                    continue;
                };
                if !index.is_repository_path(&relative) || !promises {
                    continue;
                }
                if let Some(anchor) = &anchor {
                    checked.insert(Checked {
                        subject: format!("`{}`", check::anchored_text(&relative, anchor)),
                        verdict: check::anchor(index, scope, &relative, anchor),
                    });
                }
                (format!("`{relative}`"), check::path(index, &relative))
            }
            Span::Symbol(_) if !promises => continue,
            Span::Symbol(name) => (
                format!("symbol `{name}`"),
                check::symbol(index, scope, &name, home.as_deref()),
            ),
            Span::Command(command) => {
                for subject in observe::command_subjects(&command, runners) {
                    checked.insert(Checked {
                        subject: format!("command `{command}`"),
                        verdict: check::command_subject(index, &command, &subject),
                    });
                }
                continue;
            }
        };
        checked.insert(Checked { subject, verdict });
    }
}

use output::{findings, render};
mod output;

#[cfg(test)]
mod anchor_tests;
#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod p6_tests;

#[cfg(test)]
mod root_tests;
