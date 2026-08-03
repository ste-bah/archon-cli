//! `Falsifiable` — the top of the ladder, planned here and executed nowhere.
//!
//! # What the level would prove
//!
//! `Exercised` proves the anchored file was in the causal path of a passing
//! verifier. It does not prove the verifier *depends* on the anchored lines: a
//! test that reads a module and asserts nothing about the anchored function
//! still promotes. `Falsifiable` closes that gap by experiment — break the
//! anchored lines, and the named verifier must fail. If it still passes, the
//! edge was decoration and the report says so.
//!
//! # Why it is a plan and not a run
//!
//! Executing a mutation means editing a file in the working tree, running a
//! command, and restoring. That is a write, a build and a test run per anchor;
//! full mutation testing over 93 requirements is far outside what a report may
//! cost, and NFR-004 already forbids the whole-workspace test runs that would
//! result. Scoping to `error`-severity requirements makes it tractable — one
//! requirement's anchor against one command — but the decision to mutate a
//! working tree belongs to whoever runs it, not to a read-only report.
//!
//! So this module emits an executable *plan*: which file, which lines, which
//! mutation, which command, and the criterion that decides. Running it is
//! [`FalsificationPlan::shell_recipe`], reproducible by hand.
//!
//! # An unexecuted plan promotes nothing
//!
//! [`plan`] never returns a [`ProofLevel`]. A requirement with a plan and no
//! result stays at `Exercised`, which is the fail-closed direction: the plan is
//! a statement of what has not been checked yet, and §32 requires exactly that
//! — an explicit, scoped gap with named fail-closed behaviour.
//!
//! # The scope is smaller than it looks, and that is a finding
//!
//! PRD §21 attaches `severity` to a validation *check*, not to a requirement.
//! There is no per-requirement severity marker in the document, so
//! [`super::requirements::Severity`] is derived from a short list of literal
//! phrases and records which one matched. Very few of the 93 requirements match.
//! That is reported rather than worked around: widening the derivation until the
//! scope looks respectable would be inventing severity, which is the same class
//! of error as inventing a mapping.

use serde::{Deserialize, Serialize};

use super::anchors::Anchor;
use super::ladder::{ExercisedProof, ProofLevel};
use super::requirements::Requirement;

/// How the anchored code would be broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    /// Replace the anchored line range with a body that always aborts.
    ///
    /// Chosen over subtler mutations (flipping a comparison, dropping a branch)
    /// because it is unambiguous: if the verifier still passes with the anchored
    /// region replaced by an abort, the verifier does not reach that region at
    /// all, and no finer mutation could tell you anything the coarse one did
    /// not.
    AbortAnchoredRange,
}

impl MutationKind {
    /// The replacement text for a language, or `None` when the language has no
    /// known abort form — in which case there is no plan, rather than a guess.
    pub fn replacement_for(self, language: &str) -> Option<&'static str> {
        match (self, language) {
            (MutationKind::AbortAnchoredRange, "rust") => {
                Some("unreachable!(\"archon falsification probe\")")
            }
            (MutationKind::AbortAnchoredRange, "python") => {
                Some("raise AssertionError('archon falsification probe')")
            }
            (MutationKind::AbortAnchoredRange, "typescript" | "javascript") => {
                Some("throw new Error('archon falsification probe')")
            }
            (MutationKind::AbortAnchoredRange, "go") => {
                Some("panic(\"archon falsification probe\")")
            }
            _ => None,
        }
    }
}

/// A single, reproducible falsification experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FalsificationPlan {
    pub requirement_id: String,
    /// The phrase that put this requirement in `error` scope.
    pub severity_evidence: String,
    pub task_id: String,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    /// The file hash the plan was written against. If the file has changed by
    /// the time the plan runs, the line range is no longer the anchored code and
    /// the plan must be regenerated.
    pub expected_file_hash: String,
    pub mutation: MutationKind,
    /// The verifier that promoted the edge to `Exercised`.
    pub command: String,
}

impl FalsificationPlan {
    /// The criterion that decides. Stated as a sentence because a reviewer
    /// reads this before anyone runs it.
    pub fn pass_criterion(&self) -> String {
        format!(
            "`{}` must FAIL with lines {}-{} of {} replaced by an abort, and must PASS again \
             once the file is restored to hash {}. A verifier that still passes with the \
             anchored region broken does not depend on it: the edge is Candidate at best, \
             whatever the trace showed.",
            self.command,
            self.line_start,
            self.line_end,
            self.file_path,
            &self.expected_file_hash[..self.expected_file_hash.len().min(12)],
        )
    }

    /// The steps, in order, for someone running this by hand or in a harness.
    ///
    /// Deliberately not executed by this crate: step 2 edits the working tree.
    pub fn shell_recipe(&self, language: &str) -> Option<Vec<String>> {
        let replacement = self.mutation.replacement_for(language)?;
        Some(vec![
            format!(
                "verify {} still hashes to {}",
                self.file_path, self.expected_file_hash
            ),
            format!("git stash push -- {} # or copy it aside", self.file_path),
            format!(
                "replace lines {}-{} of {} with: {replacement}",
                self.line_start, self.line_end, self.file_path
            ),
            format!("run `{}` and require a NON-ZERO exit", self.command),
            format!("restore {} and require a ZERO exit", self.file_path),
        ])
    }
}

/// Build a plan for one anchor, or explain why there is none.
///
/// Requires both: the requirement is in `error` scope, and the edge already
/// reached `Exercised`. Planning a falsification for an edge with no passing
/// verifier would be planning to break code and observe nothing.
pub fn plan(
    requirement: &Requirement,
    anchor: &Anchor,
    level: ProofLevel,
    proof: Option<&ExercisedProof>,
) -> std::result::Result<FalsificationPlan, NotPlannable> {
    if !requirement.is_error_severity() {
        return Err(NotPlannable::OutOfSeverityScope);
    }
    if level < ProofLevel::Exercised {
        return Err(NotPlannable::NotYetExercised(level));
    }
    let Some(proof) = proof else {
        return Err(NotPlannable::NotYetExercised(level));
    };
    Ok(FalsificationPlan {
        requirement_id: requirement.id.clone(),
        severity_evidence: requirement
            .severity_evidence
            .clone()
            .unwrap_or_else(|| "unrecorded".to_string()),
        task_id: anchor.task_id.clone(),
        file_path: anchor.file_path.clone(),
        line_start: anchor.line_start,
        line_end: anchor.line_end,
        expected_file_hash: anchor.file_hash.clone(),
        mutation: MutationKind::AbortAnchoredRange,
        command: proof.command.clone(),
    })
}

/// Why an anchor has no falsification plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotPlannable {
    /// The requirement is not `error`-severity (PRD §21). Mutation testing every
    /// requirement is too slow to be a report; one anchor against one command
    /// is not.
    OutOfSeverityScope,
    /// Nothing to break the dependency of: the edge has no passing verifier.
    NotYetExercised(ProofLevel),
}

impl NotPlannable {
    pub fn describe(&self) -> String {
        match self {
            NotPlannable::OutOfSeverityScope => {
                "not error-severity (PRD §21 declares severity per validation check, not per \
                 requirement); out of falsification scope"
                    .to_string()
            }
            NotPlannable::NotYetExercised(level) => format!(
                "at {} — falsification needs a passing verifier to break the dependency of",
                level.as_str()
            ),
        }
    }
}

#[cfg(test)]
mod tests;
