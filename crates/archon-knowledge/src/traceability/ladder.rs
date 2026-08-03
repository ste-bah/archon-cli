//! The proof ladder: what an edge has actually earned.
//!
//! # The rule
//!
//! An unproven edge never counts as satisfied and never satisfies a promotion
//! gate — the same rule REQ-BT-003 already applies to diagnostic overrides.
//! [`ProofLevel::satisfies_promotion_gate`] is the single place that decides,
//! and it is a match on an enum, not a threshold on a number.
//!
//! # How `Exercised` kills F1
//!
//! F1 was *"repeated generic evidence for REQ-DL-001..004"* — one sentence
//! standing in as proof for four requirements. Promotion to `Exercised` requires
//! two independent facts to line up:
//!
//! 1. a command the task itself named ran and passed, and
//! 2. the ambient trace shows that run **read the anchored file**.
//!
//! One command's trace cannot touch four unrelated anchors. Reusing the same
//! evidence across `REQ-DL-001..004` promotes at most the requirements whose
//! anchored files that command actually read, and leaves the rest at
//! `Candidate` with the missing half named. The padding does not become an
//! error; it becomes *visible*, which is stronger, because a declared gap is
//! actionable and a false pass is not.
//!
//! # The honest limit
//!
//! `TraceKind::FileRead` is file-granular. "This run read the file containing
//! the anchor" is weaker than "this run executed the anchored lines"; a broad
//! test that reads a file without exercising the anchored function still
//! promotes. Line-granular proof needs coverage instrumentation
//! (`cargo-llvm-cov`), which is the upgrade path, not the first cut. What
//! `Exercised` does establish is that the anchored file was in the causal path
//! of a passing named verifier — which is exactly the fact F1's evidence could
//! not supply for three of its four requirements.

use serde::{Deserialize, Serialize};

use super::anchors::Anchor;
use super::tasks::{TaskBinding, VerifierOrigin, normalize_command};

/// How far up the ladder an edge has climbed. Ordered: higher is stronger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofLevel {
    /// The fail-closed floor. No live anchor — none found, or every anchor
    /// stale. A declared gap, not a failure and not a pass.
    Unproven,
    /// An anchor exists and its file still hashes to what was recorded. Cheap,
    /// unproven, and exactly what F1 mistook for proof.
    Candidate,
    /// A named verifier passed and the trace shows that run read the anchor.
    Exercised,
    /// Breaking the anchored code breaks the verifier. Planned, never inferred.
    Falsifiable,
}

impl ProofLevel {
    /// The only place satisfaction is decided.
    pub fn satisfies_promotion_gate(self) -> bool {
        matches!(self, ProofLevel::Exercised | ProofLevel::Falsifiable)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProofLevel::Unproven => "Unproven",
            ProofLevel::Candidate => "Candidate",
            ProofLevel::Exercised => "Exercised",
            ProofLevel::Falsifiable => "Falsifiable",
        }
    }
}

/// A command that a run recorded, from verifier `commands_run` evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEvidence {
    pub command: String,
    /// The recorded status was "succeeded".
    pub succeeded: bool,
    pub exit_code: Option<i32>,
}

impl CommandEvidence {
    /// A command passed only when its status and its exit code agree.
    ///
    /// A "succeeded" record carrying a non-zero exit code is self-contradictory
    /// evidence, and self-contradictory evidence proves nothing.
    pub fn passed(&self) -> bool {
        self.succeeded && self.exit_code.is_none_or(|code| code == 0)
    }
}

/// A `TraceKind::FileRead` observation from the ambient trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadEvidence {
    /// The trace node the read was attributed to.
    pub node_id: String,
    /// Repository-relative, forward slashes.
    pub file_path: String,
}

/// How tightly the read that promoted an edge could be attributed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadScope {
    /// The trace attributed the read to the implementing task's own node. The
    /// strong form: this task read this file.
    Node(String),
    /// The read is in the run's trace but attributed to another node (the tool
    /// tap attributes ambient reads to the root node). Strictly weaker — it
    /// says the run read the file, not that this task did.
    Run,
}

/// What promoted an edge to `Exercised`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExercisedProof {
    /// The declared command, as declared.
    pub command: String,
    pub origin: VerifierOrigin,
    pub read_scope: ReadScope,
    /// The anchored file the trace shows being read.
    pub read_path: String,
}

/// Precisely what an edge below `Exercised` is missing.
///
/// Named rather than summarised, because "unproven" without a reason is the
/// prose that F1 was made of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissingForPromotion {
    /// The task named no runnable verifier command at all.
    NoDeclaredVerifier {
        /// Focused-test bullets that were descriptions rather than commands.
        prose_entries: usize,
    },
    /// The task named commands, but none of them appears in the run's evidence.
    NoDeclaredCommandRan { declared: Vec<String> },
    /// A declared command ran and did not pass.
    DeclaredCommandFailed {
        command: String,
        exit_code: Option<i32>,
    },
    /// A declared command passed, but the trace never shows the anchored file
    /// being read by that run. This is the F1 case.
    AnchorNotRead {
        command: String,
        anchor_path: String,
    },
    /// No trace records at all: nothing can be proven from an absent trace.
    NoTrace,
}

impl MissingForPromotion {
    /// One line naming what would have to become true.
    pub fn describe(&self) -> String {
        match self {
            MissingForPromotion::NoDeclaredVerifier { prose_entries } => format!(
                "task declares no runnable verifier command ({prose_entries} focused-test \
                 entries are descriptions, not invocations); declare one in `## Focused Tests` \
                 or as a contract `typed_verifier_command`"
            ),
            MissingForPromotion::NoDeclaredCommandRan { declared } => format!(
                "no declared verifier appears in the run's commands_run evidence; declared: {}",
                declared.join(", ")
            ),
            MissingForPromotion::DeclaredCommandFailed { command, exit_code } => format!(
                "declared verifier `{command}` did not pass (exit {})",
                exit_code.map_or_else(|| "unknown".to_string(), |c| c.to_string())
            ),
            MissingForPromotion::AnchorNotRead {
                command,
                anchor_path,
            } => format!(
                "declared verifier `{command}` passed but the run's trace never read \
                 `{anchor_path}`; the evidence does not reach this anchor"
            ),
            MissingForPromotion::NoTrace => {
                "the run recorded no ambient trace, so no read can be shown".to_string()
            }
        }
    }
}

/// Decide the level of one anchor against one run's evidence.
///
/// Returns the level it earned and, below `Exercised`, exactly what is missing.
pub fn promote(
    anchor: &Anchor,
    binding: &TaskBinding,
    commands: &[CommandEvidence],
    reads: &[ReadEvidence],
) -> (
    ProofLevel,
    Option<ExercisedProof>,
    Option<MissingForPromotion>,
) {
    if binding.verifier_commands.is_empty() {
        return (
            ProofLevel::Candidate,
            None,
            Some(MissingForPromotion::NoDeclaredVerifier {
                prose_entries: binding.prose_focused_tests().len(),
            }),
        );
    }

    let mut ran_but_failed: Option<MissingForPromotion> = None;
    let mut passed_but_unread: Option<MissingForPromotion> = None;

    for declared in &binding.verifier_commands {
        let wanted = normalize_command(&declared.command);
        let Some(record) = commands
            .iter()
            .find(|record| normalize_command(&record.command) == wanted)
        else {
            continue;
        };
        if !record.passed() {
            ran_but_failed.get_or_insert(MissingForPromotion::DeclaredCommandFailed {
                command: declared.command.clone(),
                exit_code: record.exit_code,
            });
            continue;
        }
        match read_scope_for(anchor, binding, reads) {
            Some(read_scope) => {
                return (
                    ProofLevel::Exercised,
                    Some(ExercisedProof {
                        command: declared.command.clone(),
                        origin: declared.origin,
                        read_scope,
                        read_path: anchor.file_path.clone(),
                    }),
                    None,
                );
            }
            None => {
                passed_but_unread.get_or_insert(MissingForPromotion::AnchorNotRead {
                    command: declared.command.clone(),
                    anchor_path: anchor.file_path.clone(),
                });
            }
        }
    }

    // Order matters: report the nearest miss. "Passed but did not reach the
    // anchor" is a more useful thing to hear than "nothing ran".
    let no_trace =
        (reads.is_empty() && commands.is_empty()).then_some(MissingForPromotion::NoTrace);
    let missing = passed_but_unread
        .or(ran_but_failed)
        .or(no_trace)
        .unwrap_or_else(|| MissingForPromotion::NoDeclaredCommandRan {
            declared: binding
                .verifier_commands
                .iter()
                .map(|v| v.command.clone())
                .collect(),
        });
    (ProofLevel::Candidate, None, Some(missing))
}

/// Prefer a read attributed to the task's own node; fall back to the run.
fn read_scope_for(
    anchor: &Anchor,
    binding: &TaskBinding,
    reads: &[ReadEvidence],
) -> Option<ReadScope> {
    let matches = |read: &&ReadEvidence| read.file_path.replace('\\', "/") == anchor.file_path;
    if let Some(read) = reads
        .iter()
        .filter(|read| read.node_id == binding.task_id)
        .find(matches)
    {
        return Some(ReadScope::Node(read.node_id.clone()));
    }
    reads.iter().find(matches).map(|_| ReadScope::Run)
}

#[cfg(test)]
mod tests;
