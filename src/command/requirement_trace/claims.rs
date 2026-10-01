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

mod check;
mod index;
mod observe;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_knowledge::traceability::TaskBinding;
use archon_knowledge::traceability::tasks::KNOWN_RUNNERS;
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
    outside: Vec<String>,
    /// Why the task-universe reader refused the file, when it did: its
    /// deliverables and forbidden files are then unknown.
    unread: Option<String>,
}

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
    for binding in bindings {
        let raw = std::fs::read_to_string(&binding.source_path)
            .with_context(|| format!("reading task file {}", binding.source_path))?;
        tasks.push(task_inputs(&index, binding, raw));
    }
    for task in &tasks {
        for path in &task.changeable {
            index.declare(path, &task.binding.task_id);
        }
    }
    let results: Vec<ClaimResult> = tasks
        .iter()
        .flat_map(|task| {
            task.binding
                .implements
                .iter()
                .map(|requirement| test_claim(&index, task, requirement))
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(ClaimTrace {
        report: render(&index, &results),
        findings: findings(&index, &results),
    })
}

fn task_inputs(index: &EvidenceIndex, binding: TaskBinding, raw: String) -> TaskInputs {
    let tools = binding.required_tools.clone();
    let runners = runner_check(&tools);
    let path = Path::new(&binding.source_path);
    let universe = archon_workflow::task_universe::parsing::parse_task_file(path, &raw);
    let unread = universe.as_ref().err().map(ToString::to_string);
    let universe = universe.ok();
    let mut declared: Vec<String> = binding.path_scopes.clone();
    let mut forbidden = BTreeSet::new();
    if let Some(task) = &universe {
        declared.extend(
            task.deliverable_contracts
                .iter()
                .map(|c| c.artifact_path.clone()),
        );
        declared.extend(task.shared_append_target_files.iter().cloned());
        for bullet in &task.files_forbidden_to_change {
            for span in observe::spans(bullet, &runners) {
                if let Span::Path { raw, .. } = span
                    && let Some(relative) = index.relative(&raw)
                {
                    forbidden.insert(relative);
                }
            }
        }
    }
    let mut changeable = BTreeSet::new();
    let mut outside = Vec::new();
    for entry in declared {
        match index.relative(entry.trim()) {
            Some(relative) if !relative.is_empty() && !forbidden.contains(&relative) => {
                changeable.insert(relative);
            }
            Some(_) => {}
            None => outside.push(entry),
        }
    }
    TaskInputs {
        binding,
        raw,
        changeable,
        outside,
        unread,
    }
}

fn runner_check(tools: &[String]) -> impl Fn(&str) -> bool + '_ {
    move |first: &str| KNOWN_RUNNERS.contains(&first) || tools.iter().any(|tool| tool == first)
}

fn test_claim(index: &EvidenceIndex, task: &TaskInputs, requirement: &str) -> ClaimResult {
    let runners = runner_check(&task.binding.required_tools);
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
    if named.is_empty() {
        result.untestable = Some(format!(
            "the body never names {requirement} outside its implements list, so no file, symbol or command it promises is tied to the claim"
        ));
        return result;
    }
    let mut checked = BTreeSet::new();
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
        checked.insert(Checked {
            subject: "declared files".into(),
            verdict: Verdict::Refuted(format!(
                "{} declares no file it may change (Files Expected to Change and deliverable_contracts name nothing outside its forbidden files), so no declared file can serve its claims",
                task.binding.task_id
            )),
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
        result.untestable = Some(format!(
            "nothing the body ties to {requirement} is inside the recorded repository, so the index cannot check it"
        ));
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
            Span::Path { raw, .. } => {
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

fn short(index: &EvidenceIndex) -> String {
    index.base().chars().take(12).collect()
}

fn findings(index: &EvidenceIndex, results: &[ClaimResult]) -> Vec<TracePolicyFinding> {
    let base = short(index);
    // (task, subject, reason) -> the claims that observable refutes.
    let mut refuted: BTreeMap<(String, PathBuf, String, String), Vec<String>> = BTreeMap::new();
    let mut out = Vec::new();
    for result in results {
        for checked in &result.checked {
            if let Verdict::Refuted(reason) = &checked.verdict {
                refuted
                    .entry((
                        result.task_id.clone(),
                        result.source_path.clone(),
                        checked.subject.clone(),
                        reason.clone(),
                    ))
                    .or_default()
                    .push(result.requirement.clone());
            }
        }
        if let Some(reason) = &result.untestable {
            out.push(TracePolicyFinding {
                text: format!(
                    "task {}: its claim of '{}' cannot be tested at base commit {base}: {reason}; re-author the task body so a sentence naming {} also names the file, symbol or verifier command that makes it observable",
                    result.task_id, result.requirement, result.requirement
                ),
                subject: result.task_id.clone(),
                source_path: result.source_path.clone(),
                remediation_scope: archon_workflow::RemediationScope::Body,
            });
        }
    }
    for ((task, source_path, subject, reason), claims) in refuted {
        let quoted: Vec<String> = claims.iter().map(|id| format!("'{id}'")).collect();
        out.push(TracePolicyFinding {
            text: format!(
                "task {task}: its claim of {} is refuted at base commit {base}: {subject}: {reason}; re-author the task body so every claim rests only on files, symbols and verifier commands that exist at the base commit or that a task in the set declares it will create",
                quoted.join(", ")
            ),
            subject: task,
            source_path,
            remediation_scope: archon_workflow::RemediationScope::Body,
        });
    }
    out
}

fn render(index: &EvidenceIndex, results: &[ClaimResult]) -> String {
    let refuted = |r: &ClaimResult| {
        r.checked
            .iter()
            .any(|c| matches!(c.verdict, Verdict::Refuted(_)))
    };
    let count = |f: &dyn Fn(&Verdict) -> bool| {
        results
            .iter()
            .flat_map(|r| &r.checked)
            .filter(|c| f(&c.verdict))
            .count()
    };
    let refuted_claims = results.iter().filter(|r| refuted(r)).count();
    let untestable = results.iter().filter(|r| r.untestable.is_some()).count();
    let mut out = format!(
        "\nClaim falsification against {} at base commit {}:\n  {} claim(s): {} tested, {} refuted, {} untestable\n  observables: {} exist, {} derived from declarations, {} refuted, {} untested\n",
        index.root_text(),
        index.base(),
        results.len(),
        results.len() - untestable,
        refuted_claims,
        untestable,
        count(&|v| matches!(v, Verdict::Exists(_))),
        count(&|v| matches!(v, Verdict::Derived(_))),
        count(&|v| matches!(v, Verdict::Refuted(_))),
        count(&|v| matches!(v, Verdict::Untested(_))),
    );
    for result in results {
        let head = format!("{} {}", result.task_id, result.requirement);
        if let Some(reason) = &result.untestable {
            out.push_str(&format!("  UNTESTABLE {head}: {reason}\n"));
        } else if !refuted(result) {
            let exist = result
                .checked
                .iter()
                .filter(|c| matches!(c.verdict, Verdict::Exists(_)))
                .count();
            let derived = result
                .checked
                .iter()
                .filter(|c| matches!(c.verdict, Verdict::Derived(_)))
                .count();
            out.push_str(&format!(
                "  tested {head}: {exist} exist, {derived} derived\n"
            ));
        }
        for checked in &result.checked {
            match &checked.verdict {
                Verdict::Refuted(reason) => {
                    out.push_str(&format!(
                        "  REFUTED {head}: {}: {reason}\n",
                        checked.subject
                    ));
                }
                Verdict::Untested(reason) => {
                    out.push_str(&format!("    untested {}: {reason}\n", checked.subject));
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod tests;
