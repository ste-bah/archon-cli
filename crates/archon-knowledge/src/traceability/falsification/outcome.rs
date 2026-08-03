//! What an *executed* plan established, and the single rule that lets it
//! promote.
//!
//! # Why the verdict lives here and the runner does not
//!
//! [`super`] emits a plan; running one means writing a file in someone's
//! working tree and spawning a process, and this crate still does neither. What
//! has to live here is the *verdict type* and the promotion rule, because
//! [`ProofLevel`] and [`ProofLevel::satisfies_promotion_gate`] live here, and an
//! executor that decided its own level would be a second place where
//! satisfaction is decided. There is one such place, and this is the function.
//!
//! # Exactly one outcome promotes
//!
//! [`FalsificationOutcome::level_after`] returns `Falsifiable` for one variant:
//! the verifier FAILED with the anchored lines replaced by an abort, having
//! PASSED on the same bytes moments earlier. Every other outcome — the verifier
//! still passing, the mutant not building, a timeout, a refusal — returns the
//! level the edge already held.
//!
//! Nothing here *lowers* a level either, and that is deliberate rather than
//! lenient. [`super::super::ladder::ExercisedProof`] is a statement about a run
//! that happened: a named verifier passed and the trace shows it read the
//! anchored file. A failed experiment does not unmake that run. What
//! [`FalsificationOutcome::EdgeIsDecoration`] says is narrower and worth saying
//! on its own: the verifier does not *depend* on the anchored lines, so
//! `Exercised` is the ceiling for this edge and the anchor is a citation to go
//! re-derive. Demoting it to `Candidate` would erase the trace evidence that is
//! still true; leaving it silent would be F1.
//!
//! # Why a non-zero exit is not enough
//!
//! A mutated tree fails to build far more often than it fails a test — the
//! abort form replaces a line range, not a semantically complete unit — and a
//! build error proves nothing about the verifier. *Every* line of a crate is
//! depended on by compilation, so counting "did not compile" as a kill would
//! promote every anchor in every crate at once. That is F1 rebuilt out of exit
//! codes. [`build_failure_markers`] is the list that separates the two, and the
//! classification is biased: an unrecognised failure is
//! [`Inconclusive::UnclassifiableFailure`], never a promotion.

use serde::{Deserialize, Serialize};

use super::super::ladder::ProofLevel;

/// What running one [`super::FalsificationPlan`] established.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FalsificationOutcome {
    /// The verifier passed on the original bytes and failed on the mutant. The
    /// only outcome that promotes: the verifier depends on the anchored lines.
    DependencyShown { mutated_exit: Option<i32> },
    /// The verifier still passed with the anchored region replaced by an abort.
    /// The edge is decoration — whatever the trace showed, this command does not
    /// reach this code. Promotes nothing and is the finding worth reading.
    EdgeIsDecoration { mutated_exit: Option<i32> },
    /// The experiment ran and decided nothing.
    Inconclusive(Inconclusive),
    /// The experiment was not attempted. Named, because "we refused" and "we
    /// tried and learned nothing" are different facts about the same edge.
    Refused(RefusedToRun),
}

impl FalsificationOutcome {
    /// The level this edge holds after the experiment.
    ///
    /// Guarded on `current` rather than assuming it: a plan only exists at
    /// `Exercised` today, and an outcome that could promote an edge which never
    /// had a passing verifier would be a second, weaker route to the top of the
    /// ladder. There is no such route.
    pub fn level_after(&self, current: ProofLevel) -> ProofLevel {
        match self {
            FalsificationOutcome::DependencyShown { .. } if current >= ProofLevel::Exercised => {
                ProofLevel::Falsifiable
            }
            _ => current,
        }
    }

    /// One line, in the words of whichever check produced it.
    pub fn describe(&self) -> String {
        match self {
            FalsificationOutcome::DependencyShown { mutated_exit } => format!(
                "FALSIFIABLE: the verifier passed on the original bytes and failed (exit {}) \
                 with the anchored lines replaced by an abort. It depends on them.",
                exit(*mutated_exit)
            ),
            FalsificationOutcome::EdgeIsDecoration { mutated_exit } => format!(
                "DECORATION: the verifier still passed (exit {}) with the anchored region \
                 replaced by an abort, so it never reaches that region. The edge does not \
                 promote; Exercised is its ceiling and the anchor is a citation to re-derive.",
                exit(*mutated_exit)
            ),
            FalsificationOutcome::Inconclusive(why) => {
                format!("INCONCLUSIVE: {}. No promotion.", why.describe())
            }
            FalsificationOutcome::Refused(why) => {
                format!("NOT RUN: {}. No promotion.", why.describe())
            }
        }
    }
}

/// The experiment ran but the result decides nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Inconclusive {
    /// A compiler/parser error marker appeared in the mutated run's output.
    MutantDidNotBuild { marker: String },
    /// The mutated run failed and the language has no marker list, so a test
    /// failure and a build failure are indistinguishable from here.
    UnclassifiableFailure { language: String },
    /// The mutated run was killed at the deadline. A verifier that never
    /// finishes has not failed; it has not answered.
    TimedOut { seconds: u64 },
    /// The verifier process could not be started at all.
    VerifierNotLaunchable { reason: String },
}

impl Inconclusive {
    pub fn describe(&self) -> String {
        match self {
            Inconclusive::MutantDidNotBuild { marker } => format!(
                "the mutated tree did not build (`{marker}` in the output). Every line of a \
                 crate is depended on by compilation, so a build error is not evidence that \
                 this verifier depends on these lines"
            ),
            Inconclusive::UnclassifiableFailure { language } => format!(
                "the mutated run failed and `{language}` has no recorded build-failure marker, \
                 so a failing test and a failing build cannot be told apart"
            ),
            Inconclusive::TimedOut { seconds } => {
                format!("the mutated run was killed after {seconds}s without answering")
            }
            Inconclusive::VerifierNotLaunchable { reason } => {
                format!("the verifier could not be launched: {reason}")
            }
        }
    }
}

/// Why an experiment was refused before anything was written.
///
/// Every variant is a fail-closed refusal, not a warning: the run stops and the
/// edge keeps the level it had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusedToRun {
    /// The target file has uncommitted changes. There is no safe restore for a
    /// file whose "original" is already modified: writing the pre-mutation bytes
    /// back is correct, but the reviewer cannot tell that from a tree that was
    /// dirty to begin with, and a mutation mistaken for an edit is data loss.
    DirtyFile { file_path: String, status: String },
    /// Cleanliness could not be established — no repository, no `git`, or an
    /// error from it. Unknown is treated as dirty.
    CleanlinessUnknown { file_path: String, reason: String },
    /// The declared verifier is a workspace-wide run. NFR-004 forbids it, and it
    /// has exhausted the disk it was run on twice.
    WorkspaceWideCommand { command: String, token: String },
    /// The command needs a shell to mean what it says (a pipe, a redirect, a
    /// `&&`). It is run as an argv, never through a shell, so this is a refusal
    /// rather than a quoting problem to solve.
    NotDirectlyExecutable { command: String, reason: String },
    /// No known abort form for this file's language, so there is no mutation —
    /// the same refusal [`super::MutationKind::replacement_for`] makes.
    NoMutationForLanguage { file_path: String },
    /// The file no longer hashes to what the plan was written against, so the
    /// line range is no longer the anchored code.
    FileChangedSincePlan { file_path: String },
    /// A previous run left a backup beside the target and did not reconcile it.
    /// Refusing is the only safe move: whether the working file is a stranded
    /// mutation or a later human edit cannot be determined from here.
    UnreconciledBackup { backup_path: String },
    /// The verifier did not pass on the *original* bytes. There is nothing to
    /// falsify: a command that was already failing fails while mutated too, and
    /// counting that as a kill would promote on an unrelated breakage.
    BaselineDidNotPass {
        command: String,
        exit_code: Option<i32>,
    },
    /// The anchored range could not be replaced.
    UnusableRange { file_path: String, reason: String },
    /// The file could not be read.
    FileUnreadable { file_path: String, reason: String },
}

impl RefusedToRun {
    pub fn describe(&self) -> String {
        match self {
            RefusedToRun::DirtyFile { file_path, status } => format!(
                "{file_path} has uncommitted changes (`{status}`); a file whose original is \
                 already modified cannot be safely restored after a mutation"
            ),
            RefusedToRun::CleanlinessUnknown { file_path, reason } => format!(
                "could not establish whether {file_path} is clean ({reason}); unknown is \
                 treated as dirty"
            ),
            RefusedToRun::WorkspaceWideCommand { command, token } => format!(
                "`{command}` is a workspace-wide run (`{token}`), which NFR-004 forbids; \
                 declare a scoped verifier in `## Focused Tests`"
            ),
            RefusedToRun::NotDirectlyExecutable { command, reason } => format!(
                "`{command}` needs a shell to mean what it says ({reason}); verifiers are run \
                 as an argv, never through a shell"
            ),
            RefusedToRun::NoMutationForLanguage { file_path } => format!(
                "no known abort form for {file_path}; a guessed mutation that does not compile \
                 would prove nothing"
            ),
            RefusedToRun::FileChangedSincePlan { file_path } => format!(
                "{file_path} no longer hashes to what the plan was written against; the line \
                 range is not the anchored code any more"
            ),
            RefusedToRun::UnreconciledBackup { backup_path } => format!(
                "{backup_path} is left over from an earlier run that did not finish. Compare it \
                 with the working file, restore by hand, delete the backup, and re-run"
            ),
            RefusedToRun::BaselineDidNotPass { command, exit_code } => format!(
                "`{command}` did not pass on the unmodified tree (exit {}); a verifier that is \
                 already failing cannot be broken by a mutation",
                exit(*exit_code)
            ),
            RefusedToRun::UnusableRange { file_path, reason } => {
                format!("{file_path}: {reason}")
            }
            RefusedToRun::FileUnreadable { file_path, reason } => {
                format!("{file_path} could not be read ({reason})")
            }
        }
    }
}

/// Substrings that mean "the mutant did not build", per language.
///
/// Literal compiler and interpreter prefixes, listed rather than inferred, so
/// the classification is auditable by grep in the same way
/// `ERROR_SEVERITY_PHRASES` is. A language absent from this list yields
/// [`Inconclusive::UnclassifiableFailure`] rather than a verdict — the same
/// "no guess" rule [`super::MutationKind::replacement_for`] follows.
///
/// The list only ever costs promotions. A marker that matches a genuine test
/// failure downgrades a real kill to inconclusive, which understates the
/// evidence; there is no arrangement of it that overstates the evidence.
pub fn build_failure_markers(language: &str) -> Option<&'static [&'static str]> {
    Some(match language {
        "rust" => &[
            "error[E",
            "error: could not compile",
            "error: aborting due to",
        ],
        "python" => &["SyntaxError", "IndentationError", "TabError"],
        "typescript" | "javascript" => &["SyntaxError", "TS1", "TS2"],
        "go" => &[
            "syntax error:",
            "[build failed]",
            "undefined:",
            "cannot use",
        ],
        _ => return None,
    })
}

fn exit(code: Option<i32>) -> String {
    code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
}

#[cfg(test)]
mod tests;
