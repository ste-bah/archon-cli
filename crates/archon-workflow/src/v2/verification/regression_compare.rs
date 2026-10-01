//! Issue-114 / Batch O2: what a regression comparison finds, per (command,
//! test), from the host's own verdicts at the run base and at a tip.
//!
//! One function both judges use: the final regression gate
//! (`regression_gate`, after the script) and the pre-pass regression check
//! (`regression_slot`, while a residual pass can still route what it finds
//! to its owner). The findings, in the order the gate has always written
//! them:
//!
//! - no verdict at the tip (whatever the base gave) -- blocks;
//! - failures the runner counted but did not name at the tip, more than the
//!   base had -- blocks; as many or fewer -- pre-existing;
//! - a base verdict cached before passed ids were kept -- a warning;
//! - a test that passed at the base and is ignored at the tip (hidden), or
//!   is no longer reported at all (vanished) -- blocks;
//! - a test failing at the tip: failing at the base too -- pre-existing;
//!   else (or the base gave no verdict) -- a NEW failure, which blocks.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::v2::write::test_baseline_run_base::HostRunVerdict;

/// What one comparison found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegressionFinding {
    NoTipVerdict {
        command: String,
        base_had: bool,
    },
    UnnamedNew {
        command: String,
    },
    UnnamedPreExisting {
        command: String,
    },
    /// m3: failing at the base and the tip alike -- the same exit code --
    /// naming no test at either: nothing shows whether the run regressed it.
    NotJudgeable {
        command: String,
        exit_code: Option<i32>,
    },
    OldBaseCache {
        command: String,
    },
    Hidden {
        command: String,
        test: String,
    },
    Vanished {
        command: String,
        test: String,
    },
    PreExisting {
        command: String,
        test: String,
        moved: bool,
    },
    NewFailure {
        command: String,
        test: String,
        base_had: bool,
    },
}

impl RegressionFinding {
    /// Whether the run regressed here (every other finding is a note).
    pub fn blocks(&self) -> bool {
        !matches!(
            self,
            Self::UnnamedPreExisting { .. }
                | Self::NotJudgeable { .. }
                | Self::OldBaseCache { .. }
                | Self::PreExisting { .. }
        )
    }

    pub fn command(&self) -> &str {
        match self {
            Self::NoTipVerdict { command, .. }
            | Self::UnnamedNew { command }
            | Self::UnnamedPreExisting { command }
            | Self::NotJudgeable { command, .. }
            | Self::OldBaseCache { command }
            | Self::Hidden { command, .. }
            | Self::Vanished { command, .. }
            | Self::PreExisting { command, .. }
            | Self::NewFailure { command, .. } => command,
        }
    }

    /// The test it names, when it names one.
    pub fn test(&self) -> Option<&str> {
        match self {
            Self::Hidden { test, .. }
            | Self::Vanished { test, .. }
            | Self::PreExisting { test, .. }
            | Self::NewFailure { test, .. } => Some(test),
            _ => None,
        }
    }
}

/// Every finding over `commands`, from the verdicts at the base and at the
/// tip (a command absent from a map had no verdict there).
pub(crate) fn compare(
    commands: &[String],
    base_runs: &BTreeMap<String, HostRunVerdict>,
    tip_runs: &BTreeMap<String, HostRunVerdict>,
) -> Vec<RegressionFinding> {
    use RegressionFinding as F;
    let mut out = Vec::new();
    for command in commands {
        let at_base = base_runs.get(command);
        let command = command.clone();
        let Some(at_tip) = tip_runs.get(&command) else {
            out.push(F::NoTipVerdict {
                command,
                base_had: at_base.is_some(),
            });
            continue;
        };
        let tip_unnamed = unnamed(at_tip);
        if tip_unnamed > 0 {
            out.push(match at_base {
                Some(base)
                    if names_nothing(base)
                        && names_nothing(at_tip)
                        && base.exit_code == at_tip.exit_code =>
                {
                    F::NotJudgeable {
                        command: command.clone(),
                        exit_code: at_tip.exit_code,
                    }
                }
                Some(base) if unnamed(base) >= tip_unnamed => F::UnnamedPreExisting {
                    command: command.clone(),
                },
                _ => F::UnnamedNew {
                    command: command.clone(),
                },
            });
        }
        if at_base.is_some_and(|run| !run.ids_kept) {
            out.push(F::OldBaseCache {
                command: command.clone(),
            });
        }
        if let Some(run) = at_base.filter(|run| run.ids_kept) {
            for test in &run.passed_tests {
                let test = test.clone();
                if at_tip.ignored_tests.contains(&test) {
                    out.push(F::Hidden {
                        command: command.clone(),
                        test,
                    });
                } else if at_tip.ids_kept
                    && !at_tip.passed_tests.contains(&test)
                    && !at_tip.failing_tests.contains(&test)
                {
                    out.push(F::Vanished {
                        command: command.clone(),
                        test,
                    });
                }
            }
        }
        let before: BTreeSet<&String> = at_base
            .map(|run| run.failing_tests.iter().collect())
            .unwrap_or_default();
        for test in &at_tip.failing_tests {
            if before.contains(test) {
                let moved = at_base
                    .and_then(|run| run.signatures.get(test))
                    .is_some_and(|sig| at_tip.signatures.get(test) != Some(sig));
                out.push(F::PreExisting {
                    command: command.clone(),
                    test: test.clone(),
                    moved,
                });
            } else {
                out.push(F::NewFailure {
                    command: command.clone(),
                    test: test.clone(),
                    base_had: at_base.is_some(),
                });
            }
        }
    }
    out
}

/// A failing run that named no test at all: none failed, passed or ignored.
fn names_nothing(run: &HostRunVerdict) -> bool {
    run.exit_code != Some(0)
        && run.failing_tests.is_empty()
        && run.passed_tests.is_empty()
        && run.ignored_tests.is_empty()
}

/// Failures the runner counted but did not name (one, for a failing run
/// that named and counted nothing).
fn unnamed(run: &HostRunVerdict) -> usize {
    let counted = run.failed_count.unwrap_or(0);
    let named = run.failing_tests.len();
    if run.exit_code != Some(0) && counted == 0 && named == 0 {
        1
    } else {
        counted.saturating_sub(named)
    }
}
