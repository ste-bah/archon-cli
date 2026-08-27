//! Requirement coverage — the PRD's requirement IDs against the tasks' claims.
//!
//! The PRD authoring guide states a Decomposition Completeness Gate: every
//! requirement is claimed by at least one task's `implements:` list, and no task
//! cites an ID the PRD does not define. Both are pure set operations over two
//! lists of strings, so they run here rather than in an LLM. The shared
//! `archon-workflow` extractor supplies the exact union of line-leading REQ
//! bullets and IDs from obligation tables; this module compares that set with
//! the union of the task files' `implements:` lists.
//!
//! # Why the set defects block
//!
//! An obligation no task claims is work nobody does, and an ID no PRD defines
//! is a typo or stale reference. The report renders both before the command
//! returns non-zero, so every author gets the exact ID and edit rather than a
//! silently incomplete decomposition.
//!
//! # What "cannot resolve the PRD" means here
//!
//! The check needs the PRD, and `archon workflow lint` is given a task
//! directory. Two conventions fix where the PRD is, and both are tried.
//!
//! §3.1 of the task-spec guide puts the PRD file beside the task directory,
//! named for the same PRD id. `/workflow-prd` instead writes every PRD under a
//! repository-level `prds/` root, sibling to the `tasks/` root the task
//! directory lives in — so a lint given `tasks/PRD-X/` must look in
//! `prds/PRD-X/`, not only in `tasks/`. Each task also names its PRD in `prd:`,
//! which supplies the id when the directory name does not.
//!
//! The PRD is looked for at the paths those rules predict, and when none of
//! them exists the section says which paths it tried and stops. It does not
//! fall back to scanning for any markdown file that
//! happens to contain requirement IDs — a coverage report computed against the
//! wrong document is worse than no coverage report, because it looks like one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use archon_core::skills::workflow_prd::PRD_ROOT;
use archon_workflow::obligation_ids::obligation_ids;

use crate::command::topology_task_graph::{
    TaskRequirementClaims, task_requirement_claims_tolerant,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CoveragePolicyFinding {
    pub(super) text: String,
    pub(super) subject: String,
    pub(super) source_path: PathBuf,
}

/// Requirements the PRD defines that no task claims, for a caller that blocks.
///
/// # Why this blocks rather than warns
///
/// A requirement nobody claims is work nobody does, and nothing downstream ever
/// notices: the run finishes, every task it knows about passes, and the feature
/// simply is not there. Observed live — a decomposition numbered its tasks
/// `010, 020, 040, 050`, left the `030` slot empty, and never wrote the Pine
/// artifacts task at all. Four requirements were orphaned by that one omission
/// and the lint reported them into a summary that exited zero.
///
/// It is a fact, not a judgement: the ids come from the PRD's own bullets and
/// the claims from the tasks' own `implements:`. Nothing here knows what a
/// requirement means, so it holds for any PRD in any domain.
pub(super) fn policy_findings(tasks_root: Option<&Path>) -> Vec<CoveragePolicyFinding> {
    let Some(root) = tasks_root else {
        return Vec::new();
    };
    let Ok((claims, _skipped)) = task_requirement_claims_tolerant(root) else {
        return Vec::new();
    };
    let Some(prd_path) = resolve_prd(root, &claims) else {
        return Vec::new();
    };
    let Ok(prd) = std::fs::read_to_string(&prd_path) else {
        return Vec::new();
    };
    let defined = obligation_ids(&prd);
    let claimed = claimed_by_task(&claims);
    let mut findings = archon_workflow::obligation_ids::malformed_obligation_ids(&prd)
        .into_iter()
        .map(|id| CoveragePolicyFinding {
            text: archon_workflow::obligation_ids::malformed_obligation_finding(&id),
            subject: id,
            source_path: prd_path.clone(),
        })
        .collect::<Vec<_>>();
    findings.extend(
        archon_workflow::obligation_ids::duplicate_obligation_ids(&prd)
            .into_iter()
            .map(|id| CoveragePolicyFinding {
                text: archon_workflow::obligation_ids::duplicate_obligation_finding(&id),
                subject: id,
                source_path: prd_path.clone(),
            }),
    );
    findings.extend(
        defined
            .iter()
            .filter(|id| !claimed.contains_key(*id))
            .map(|id| CoveragePolicyFinding {
                text: format!(
                    "{id}: defined in the PRD but claimed by no task — add it to at least one TASK file's implements list or remove/correct the PRD obligation"
                ),
                subject: id.clone(),
                source_path: prd_path.clone(),
            }),
    );
    for claim in &claims {
        for cited in &claim.implements {
            if !defined.contains(cited) {
                findings.push(CoveragePolicyFinding {
                    text: format!(
                        "task '{}' cites unknown obligation '{}' in {}; remove it from that TASK file's implements list or correct it to an ID defined by the PRD",
                        claim.task_id, cited, claim.source_path
                    ),
                    subject: claim.task_id.clone(),
                    source_path: PathBuf::from(&claim.source_path),
                });
            }
        }
    }
    findings.sort_by(|left, right| {
        (&left.text, &left.source_path).cmp(&(&right.text, &right.source_path))
    });
    findings.dedup();
    findings
}

pub(super) fn unclaimed_requirements(tasks_root: Option<&Path>) -> Vec<String> {
    let Some(root) = tasks_root else {
        return Vec::new();
    };
    let Ok((claims, _skipped)) = task_requirement_claims_tolerant(root) else {
        return Vec::new();
    };
    let Some(prd_path) = resolve_prd(root, &claims) else {
        return Vec::new();
    };
    let Ok(prd) = std::fs::read_to_string(&prd_path) else {
        return Vec::new();
    };
    let claimed = claimed_by_task(&claims);
    obligation_ids(&prd)
        .into_iter()
        .filter(|id| !claimed.contains_key(id))
        .collect()
}

pub(super) fn section(tasks_root: Option<&Path>) -> String {
    let mut out = String::from("\n## requirement coverage\n");
    let Some(root) = tasks_root else {
        out.push_str(
            "  only computed for --tasks: a spec or a recorded graph carries no \
             `implements:` claims and no PRD to check them against.\n",
        );
        return out;
    };
    // Tolerant, deliberately: one malformed spec out of nineteen used to take
    // this whole section down, so nothing was reported about the eighteen that
    // parsed. What was skipped is named, because a partial answer that looks
    // complete is worse than no answer at all.
    let (claims, skipped) = match task_requirement_claims_tolerant(root) {
        Ok(pair) => pair,
        Err(error) => {
            out.push_str(&format!("  could not read the task directory: {error}\n"));
            return out;
        }
    };
    if !skipped.is_empty() {
        out.push_str(&format!(
            "  {} task file(s) did not parse and were EXCLUDED from every count below:\n",
            skipped.len()
        ));
        for reason in &skipped {
            out.push_str(&format!("    {reason}\n"));
        }
    }
    if claims.is_empty() {
        out.push_str("  no task file parsed; nothing to check coverage against.\n");
        return out;
    }
    let Some(prd_path) = resolve_prd(root, &claims) else {
        out.push_str(&format!(
            "  no PRD found for {}; skipped. Tried: {}.\n",
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
    let defined = obligation_ids(prd);
    let claimed_by = claimed_by_task(claims);
    let claimed: BTreeSet<&String> = claimed_by.keys().collect();

    let mut out = format!(
        "  {} obligation(s) in {}, {} claimed across {} task(s).\n",
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
        out.push_str("  every obligation is claimed by at least one task.\n");
    } else {
        out.push_str(&format!(
            "  {} obligation(s) claimed by no task — a decomposition gap. Either a \
             task's `implements:` is missing an ID, or the work is undecomposed and needs \
             a task. This EXITS NON-ZERO: work nobody claims is work nobody does:\n",
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
        out.push_str(&render_table_obligations(prd, &claimed));
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
    out.push_str(&render_table_obligations(prd, &claimed));
    out
}

/// Obligations the PRD states in table rows, and whether any task cites them.
///
/// Reported beside requirement coverage rather than folded into it: `REQ`
/// coverage is the gate the authoring guide defines, and an acceptance
/// criterion is a different kind of claim. Both answer the same question — does
/// anything own this — and until now only one of them was ever asked.
fn render_table_obligations(prd: &str, claimed: &BTreeSet<&String>) -> String {
    let mut families: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for id in obligation_ids(prd)
        .into_iter()
        .filter(|id| !id.starts_with("REQ-"))
    {
        if let Some(family) = id.split('-').next() {
            families.entry(family.to_string()).or_default().insert(id);
        }
    }
    if families.is_empty() {
        return String::new();
    }
    // A COUNT, not a second list. These ids are part of the one claim space
    // above, so anything unclaimed has already been named there — printing it
    // twice made one gap look like two and buried the requirement ids among
    // the acceptance criteria.
    let mut out = String::new();
    for (family, ids) in families {
        let owned = ids.iter().filter(|id| claimed.contains(id)).count();
        out.push_str(&format!(
            "  {family}-*: {owned} of {} obligation(s) stated in tables are claimed by a task\n",
            ids.len()
        ));
    }
    out
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

/// Where the PRD can be, for each candidate id: the id each task declares in
/// `prd:` first, then the task directory's own name.
///
/// Two layouts, tried in that order per id:
///
/// - **Beside the task directory** (§3.1) — `<parent>/<id>.md`.
/// - **Under the `prds/` root** written by `/workflow-prd`, which is a sibling
///   of the `tasks/` root rather than of the task directory, so it is resolved
///   from the grandparent. Three shapes, because the two pipelines name the
///   file differently: `prds/<id>/<id>.md` (the workflow path),
///   `prds/<id>.md` (a flat root), and `prds/<id>/PRD.md` (the skills chain's
///   fixed filename).
///
/// Order matters only for which path wins when several exist; §3.1 stays first
/// so a task set that already had an adjacent PRD keeps resolving to it.
fn prd_candidates(root: &Path, claims: &[TaskRequirementClaims]) -> Vec<PathBuf> {
    let Some(parent) = root.parent() else {
        return Vec::new();
    };
    let mut names: Vec<String> = declared_prd_ids(claims);
    if let Some(stem) = root.file_name().and_then(|name| name.to_str()) {
        names.push(stem.to_string());
    }
    let prds_root = parent.parent().map(|base| base.join(PRD_ROOT));
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for name in names.into_iter().filter(|name| seen.insert(name.clone())) {
        candidates.push(parent.join(format!("{name}.md")));
        if let Some(prds_root) = prds_root.as_ref() {
            candidates.push(prds_root.join(&name).join(format!("{name}.md")));
            candidates.push(prds_root.join(format!("{name}.md")));
            candidates.push(prds_root.join(&name).join("PRD.md"));
        }
    }
    candidates
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
