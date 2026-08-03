//! Requirement coverage — the PRD's requirement IDs against the tasks' claims.
//!
//! The PRD authoring guide states a Decomposition Completeness Gate: every
//! requirement is claimed by at least one task's `implements:` list, and no task
//! cites an ID the PRD does not define. Both are pure set operations over two
//! lists of strings, so they run here rather than in an LLM: extract
//! `REQ-<AREA>-<NNN>` from the PRD by regex, union the `implements:` lists from
//! the task files, and print the two differences.
//!
//! # Why it stays advisory
//!
//! A requirement no task claims is a decomposition gap, and an ID no PRD
//! defines is a typo or a stale reference. Both are questions for the author.
//! Neither is a reason to refuse to lint the rest of the graph, so this section
//! reports and returns, exactly like the three lints beside it.
//!
//! # What "cannot resolve the PRD" means here
//!
//! The check needs the PRD, and `archon workflow lint` is given a task
//! directory. §3.1 of the task-spec guide fixes the relationship: the PRD file
//! sits beside the task directory, named for the same PRD id, and each task
//! names it in `prd:`. So the PRD is looked for at the paths those two rules
//! predict, and when none of them exists the section says which paths it tried
//! and stops. It does not fall back to scanning for any markdown file that
//! happens to contain requirement IDs — a coverage report computed against the
//! wrong document is worse than no coverage report, because it looks like one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::command::workflow_live::workflow_live_task_universe::{
    TaskRequirementClaims, task_requirement_claims_from_root,
};

/// A normative requirement: a line whose first non-space content is `- ` or
/// `* ` followed immediately by an ID. The guide requires one requirement per
/// line start, which is what makes this regex sufficient and what makes an ID
/// buried mid-paragraph invisible to it — deliberately, since an invisible ID
/// would otherwise pass the coverage check by never being counted.
fn requirement_line_pattern() -> Regex {
    Regex::new(r"(?m)^[ \t]*[-*][ \t]+(REQ-[A-Z0-9]+-[0-9]{3})\b")
        .expect("requirement id pattern is a literal and compiles")
}

/// The `## requirement coverage` section, for whichever source was linted.
///
/// `None` — a `--spec-file` or `--graph` run — says so rather than staying
/// silent: a missing section is indistinguishable from a clean one.
pub(super) fn section(tasks_root: Option<&Path>) -> String {
    let mut out = String::from("\n## requirement coverage\n");
    let Some(root) = tasks_root else {
        out.push_str(
            "  only computed for --tasks: a spec or a recorded graph carries no \
             `implements:` claims and no PRD to check them against.\n",
        );
        return out;
    };
    let claims = match task_requirement_claims_from_root(root) {
        Ok(claims) => claims,
        Err(error) => {
            out.push_str(&format!("  could not read task claims: {error}\n"));
            return out;
        }
    };
    let Some(prd_path) = resolve_prd(root, &claims) else {
        out.push_str(&format!(
            "  no PRD found beside {}; skipped. Tried: {}.\n",
            root.display(),
            render_candidates(&prd_candidates(root, &claims))
        ));
        return out;
    };
    let prd = match fs::read_to_string(&prd_path) {
        Ok(prd) => prd,
        Err(error) => {
            out.push_str(&format!(
                "  could not read {}: {error}; skipped.\n",
                prd_path.display()
            ));
            return out;
        }
    };
    out.push_str(&render(&prd_path, &prd, &claims));
    out
}

fn render(prd_path: &Path, prd: &str, claims: &[TaskRequirementClaims]) -> String {
    let defined = requirement_ids(prd);
    let claimed_by = claimed_by_task(claims);
    let claimed: BTreeSet<&String> = claimed_by.keys().collect();

    let mut out = format!(
        "  {} requirement(s) in {}, {} claimed across {} task(s).\n",
        defined.len(),
        prd_path.display(),
        claimed.len(),
        claims.len()
    );

    let unclaimed: Vec<&String> = defined
        .iter()
        .filter(|id| !claimed_by.contains_key(*id))
        .collect();
    if unclaimed.is_empty() {
        out.push_str("  every requirement is claimed by at least one task.\n");
    } else {
        out.push_str(&format!(
            "  {} requirement(s) claimed by no task — a decomposition gap. Either \
             a task's `implements:` is missing an ID, or the work is undecomposed \
             and needs a task:\n",
            unclaimed.len()
        ));
        for id in unclaimed {
            out.push_str(&format!("    {id}\n"));
        }
    }

    let unknown: Vec<(&String, &BTreeSet<String>)> = claimed_by
        .iter()
        .filter(|(id, _)| !defined.contains(*id))
        .collect();
    if unknown.is_empty() {
        out.push_str("  every ID cited by a task is defined in the PRD.\n");
        return out;
    }
    out.push_str(&format!(
        "  {} ID(s) cited by a task but not defined in the PRD — a typo, or a \
         requirement that was renumbered or deleted under the task:\n",
        unknown.len()
    ));
    for (id, tasks) in unknown {
        out.push_str(&format!(
            "    {id} cited by {}\n",
            tasks.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    out
}

/// Requirement IDs the PRD defines, in the bullet form §3.3 requires.
pub(super) fn requirement_ids(prd: &str) -> BTreeSet<String> {
    requirement_line_pattern()
        .captures_iter(prd)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// Cited ID → the tasks citing it, so an unknown ID names its source file.
fn claimed_by_task(claims: &[TaskRequirementClaims]) -> BTreeMap<String, BTreeSet<String>> {
    let mut claimed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for claim in claims {
        for id in &claim.implements {
            claimed
                .entry(id.trim().to_string())
                .or_default()
                .insert(claim.task_id.clone());
        }
    }
    claimed.remove("");
    claimed
}

fn resolve_prd(root: &Path, claims: &[TaskRequirementClaims]) -> Option<PathBuf> {
    prd_candidates(root, claims)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// Where the PRD can be, in the order §3.1 predicts: the id each task declares
/// in `prd:`, then the task directory's own name. Both resolved against the
/// directory's parent, because the PRD is its sibling.
fn prd_candidates(root: &Path, claims: &[TaskRequirementClaims]) -> Vec<PathBuf> {
    let Some(parent) = root.parent() else {
        return Vec::new();
    };
    let mut names: Vec<String> = declared_prd_ids(claims);
    if let Some(stem) = root.file_name().and_then(|name| name.to_str()) {
        names.push(stem.to_string());
    }
    let mut seen = BTreeSet::new();
    names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .map(|name| parent.join(format!("{name}.md")))
        .collect()
}

/// The `prd:` values the task files declare.
///
/// Read as a plain first-field scan rather than through the task parser: `prd`
/// is informational to a run and deliberately not a required key, so a task set
/// that declares none of them still lints — it falls through to the directory
/// name below.
fn declared_prd_ids(claims: &[TaskRequirementClaims]) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for claim in claims {
        let Ok(raw) = fs::read_to_string(&claim.source_path) else {
            continue;
        };
        if let Some(id) = raw.lines().find_map(|line| {
            line.trim()
                .strip_prefix("prd:")
                .map(str::trim)
                .map(|value| value.trim_matches(|ch| matches!(ch, '"' | '\'')))
                .filter(|value| !value.is_empty())
        }) {
            ids.insert(id.to_string());
        }
    }
    ids.into_iter().collect()
}

fn render_candidates(candidates: &[PathBuf]) -> String {
    if candidates.is_empty() {
        return "nothing — the task directory has no parent".to_string();
    }
    candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
