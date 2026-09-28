//! A failing acceptance check is remediated by tasks that can WRITE the fix.
//!
//! The acceptance stage routed a failing check only to the tasks whose
//! `implements` list names it (`acceptance_stage::owning_tasks`), plus the
//! landing its regression search blamed (`acceptance_regression`). Neither
//! says who may write the file that broke: under the owner rule (Issue-121)
//! a routed implementer is refused another task's file, and a file no task
//! declares is outside every implementer's scope. Live on wf-0ddadd81 three
//! checks regressed in a file no task declares and were routed to tasks
//! whose scope roots did not reach it: two of the three units could not act.
//!
//! So each failing check also names the repository files its failure
//! implicates -- the `path:line` references in its own output, read from the
//! end (where a failure reports itself) and bounded, and every file the
//! landing it regressed at changed -- and, for each, who can write it:
//!
//! - a file a task declares (`residual_paths::owners`) routes the
//!   remediation to that task too, so the unit holds it;
//! - a file no task declares, PROVEN so (`provably_unowned`), not protected,
//!   and not forbidden to the unit's tasks, is granted to the unit (the
//!   Issue-117 expansion rule, without the residual round's exact lift: a
//!   granted file becomes a declared target, and a declared target overrides
//!   a forbidden entry, so a forbidden file is never granted);
//! - anything else is recorded as unwritable, with why, on the check --
//!   and so is the check's own source (what its frozen command runs: its
//!   program, the script or test it executes, a target it names by path or
//!   stem): a remediation fixes the implementation, never the check.
//!
//! A grant needs a unit: with no implementer, no blamed landing and no owner,
//! the tasks whose own text names the file (`TaskTexts::naming`) write it.
//! Everything here is read from the host's records, the task universe and
//! the repository; agent text supplies path candidates only.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::acceptance_stage::AcceptanceRoundRecordV1;
use super::script::residual_paths::{
    TaskTexts, is_repo_file, owners, protected, provably_unowned, residual_forbidden,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// Most `path:line` references one check's output contributes.
pub const MAX_OUTPUT_FILES: usize = 6;
/// Most implicated files one check routes.
pub const MAX_IMPLICATED: usize = 16;

/// Who can write what a failing check implicates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceRoutingV1 {
    /// Repository files the failure implicates, in routing order.
    pub implicated_files: Vec<String>,
    /// Tasks the remediation is routed to so that it can write an
    /// implicated file: its owners, or for a granted file the tasks naming it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writer_tasks: Vec<String>,
    /// Implicated files no task declares, granted to the remediation unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub granted_files: Vec<String>,
    /// Implicated files no unit may be given, each with the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unwritable: Vec<(String, String)>,
}

impl AcceptanceRoutingV1 {
    /// Whether this routing gives the remediation somewhere to go.
    pub fn routes(&self) -> bool {
        !self.writer_tasks.is_empty()
    }

    /// The routing as a gap clause: `; it implicates ...`.
    pub fn describe(&self) -> String {
        let mut text = format!("; it implicates {}", self.implicated_files.join(", "));
        if !self.writer_tasks.is_empty() {
            text.push_str(&format!(
                "; routed also to {}, who can write them",
                self.writer_tasks.join(", ")
            ));
        }
        if !self.granted_files.is_empty() {
            text.push_str(&format!(
                "; granted {}, which no task declares",
                self.granted_files.join(", ")
            ));
        }
        for (file, why) in &self.unwritable {
            text.push_str(&format!("; {file} cannot be given to any unit: {why}"));
        }
        text
    }
}

/// The repository-relative file a token names with a line location
/// (`path:12`, `path:12:4`, `--> path:12`), when it exists under `root`.
fn located_file(token: &str, root: &Path) -> Option<String> {
    let token = token.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | '`' | ',' | ';'
        )
    });
    let (head, tail) = token.split_once(':')?;
    let line = tail.split(':').next().unwrap_or_default();
    if line.is_empty() || !line.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let root_text = root.to_string_lossy();
    let relative = head
        .strip_prefix(root_text.as_ref())
        .map(|rest| rest.trim_start_matches('/'))
        .unwrap_or(head);
    let relative = relative.strip_prefix("./").unwrap_or(relative);
    let parts: Vec<&str> = relative.split('/').collect();
    let clean = |parts: &[&str]| {
        !parts.is_empty()
            && parts
                .iter()
                .all(|p| !p.is_empty() && *p != "." && *p != "..")
    };
    if !relative.starts_with('/') {
        return (clean(&parts) && is_repo_file(root, relative)).then(|| relative.to_string());
    }
    // An absolute path under some other copy of the repository (a scratch
    // checkout the check ran in): its longest tail of two or more
    // components that is a repository file.
    (1..parts.len().saturating_sub(1)).find_map(|at| {
        let tail = &parts[at..];
        let candidate = tail.join("/");
        (clean(tail) && is_repo_file(root, &candidate)).then_some(candidate)
    })
}

/// The distinct repository files `text` names by `path:line`, last first.
pub fn located_files(text: &str, root: &Path, limit: usize) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines().rev() {
        for token in line.split_whitespace().rev() {
            if found.len() >= limit {
                return found;
            }
            if let Some(file) = located_file(token, root)
                && !found.contains(&file)
            {
                found.push(file);
            }
        }
    }
    found
}

/// Whether `file` is the check's own source rather than what it checks,
/// read from the frozen command's structure alone: the program it runs, the
/// first argument after the program (a script or test an interpreter or
/// runner executes), or a flag's value naming it by path or stem (a test or
/// script target, `--flag name`). A file the command only reads or searches
/// (a pattern comes first) is what the check inspects, never excluded. A
/// remediation fixes the implementation; it is never handed the check.
fn own_source(file: &str, command: &str) -> bool {
    let stem = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let names = |token: &str| {
        let token = token.trim_matches(|c| c == '\'' || c == '"');
        token.strip_prefix("./").unwrap_or(token) == file
    };
    let mut tokens = command
        .split_whitespace()
        .skip_while(|token| token.contains('=') && !token.starts_with('-'));
    let Some(program) = tokens.next() else {
        return false;
    };
    if names(program) {
        return true;
    }
    let rest: Vec<&str> = tokens.collect();
    if rest
        .iter()
        .find(|token| !token.starts_with('-'))
        .is_some_and(|t| names(t))
    {
        return true;
    }
    rest.windows(2).any(|pair| {
        pair[0].starts_with('-')
            && !pair[1].starts_with('-')
            && (names(pair[1]) || (stem.len() >= 3 && pair[1] == stem))
    })
}

/// The files one check implicates: its output's located files (stderr
/// first), then its blamed landing's changed files, bounded; less the
/// check's own sources, which are recorded as unwritable.
fn implicated(
    check: &super::acceptance_stage::AcceptanceCheckRecordV1,
    root: &Path,
    command: &str,
    own: &mut Vec<(String, String)>,
) -> Vec<String> {
    let mut files = Vec::new();
    let located = located_files(&check.stderr_tail, root, MAX_OUTPUT_FILES)
        .into_iter()
        .chain(located_files(&check.stdout_tail, root, MAX_OUTPUT_FILES));
    for file in located {
        if files.len() >= MAX_OUTPUT_FILES || files.contains(&file) {
            continue;
        }
        if own_source(&file, command) {
            own.push((
                file,
                "the check's own source, never the remediation's to change".into(),
            ));
        } else {
            files.push(file);
        }
    }
    if let Some(regression) = &check.regressed_by {
        for file in &regression.changed_files {
            if files.contains(file) || !is_repo_file(root, file) {
                continue;
            }
            if own_source(file, command) {
                let why = "the check's own source, never the remediation's to change";
                own.push((file.clone(), why.into()));
            } else {
                files.push(file.clone());
            }
        }
    }
    files.truncate(MAX_IMPLICATED);
    files
}

/// The routing of one failing check whose unit so far is `unit` (its
/// implementers and blamed landing's tasks), or `None` when it implicates no
/// repository file.
pub fn route_check(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    texts: &TaskTexts,
    check: &super::acceptance_stage::AcceptanceCheckRecordV1,
    command: &str,
) -> Option<AcceptanceRoutingV1> {
    let mut unwritable = Vec::new();
    let files = implicated(check, root, command, &mut unwritable);
    if files.is_empty() && unwritable.is_empty() {
        return None;
    }
    let mut unit: BTreeSet<String> = check.owning_tasks.iter().cloned().collect();
    if let Some(regression) = &check.regressed_by {
        unit.extend(regression.tasks.iter().cloned());
    }
    let mut writers = BTreeSet::new();
    let mut unowned = Vec::new();
    for file in &files {
        let declared = owners(universe, file, root);
        if !declared.is_empty() {
            writers.extend(declared);
        } else if protected(file) {
            unwritable.push((file.clone(), "a protected path no unit is opened".into()));
        } else if !provably_unowned(universe, file, root) {
            unwritable.push((
                file.clone(),
                "no task declares it, but a task declaration cannot be read, so it is not provably unowned".into(),
            ));
        } else {
            unowned.push(file.clone());
        }
    }
    let mut granted = Vec::new();
    for file in unowned {
        // The unit that will hold it: everyone routed so far, else the tasks
        // whose own text names it.
        let mut holders: BTreeSet<String> = unit.union(&writers).cloned().collect();
        if holders.is_empty() {
            holders = texts.naming(&file, root);
            writers.extend(holders.iter().cloned());
        }
        if holders.is_empty() {
            unwritable.push((file, "no task declares it and none names it".into()));
            continue;
        }
        let ids: Vec<String> = holders.into_iter().collect();
        if residual_forbidden(universe, &ids, &[]).matches(&file) {
            unwritable.push((file, format!("forbidden to {}", ids.join(", "))));
        } else {
            granted.push(file);
        }
    }
    Some(AcceptanceRoutingV1 {
        implicated_files: files,
        writer_tasks: writers.into_iter().collect(),
        granted_files: granted,
        unwritable,
    })
}

/// The command a frozen check runs, as written; empty for a declarative
/// floor.
pub fn check_command(criterion: &crate::task_set_contract::AcceptanceCriterion) -> String {
    use crate::task_set_contract::AcceptanceCheck;
    match &criterion.check {
        AcceptanceCheck::Command { command, .. } => command.clone(),
        AcceptanceCheck::Floor { contract } => {
            contract.typed_verifier_command.clone().unwrap_or_default()
        }
    }
}

/// A check's routing as a gap clause; empty when it has none.
pub fn clause(check: &super::acceptance_stage::AcceptanceCheckRecordV1) -> String {
    check
        .routing
        .as_ref()
        .map_or(String::new(), AcceptanceRoutingV1::describe)
}

/// Route every failing, non-defect check of `record` (see the module docs)
/// against the frozen `criteria` it ran: what a check's command runs is the
/// check itself.
pub fn route_failures(
    universe: Option<&WorkflowV2TaskUniverse>,
    root: &Path,
    criteria: &[&crate::task_set_contract::AcceptanceCriterion],
    record: &mut AcceptanceRoundRecordV1,
) {
    let Some(universe) = universe else {
        return;
    };
    let commands: std::collections::BTreeMap<String, String> = criteria
        .iter()
        .map(|criterion| (criterion.id.clone(), check_command(criterion)))
        .collect();
    let texts = TaskTexts::read(universe, root);
    for check in &mut record.checks {
        if !check.failing() || check.contract_defect {
            continue;
        }
        let text = commands
            .get(&check.check_id)
            .map(String::as_str)
            .unwrap_or_default();
        check.routing = route_check(universe, root, &texts, check, text);
    }
}

#[cfg(test)]
#[path = "acceptance_routing_tests.rs"]
mod tests;
