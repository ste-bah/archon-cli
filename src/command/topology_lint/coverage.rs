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
use regex::Regex;

use crate::command::topology_task_graph::{
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

/// An obligation stated in a table row rather than a bullet.
///
/// # Why a second pattern exists
///
/// The bullet pattern above only ever saw `REQ-` ids on bullet lines, and a PRD
/// states obligations in more than one place. One observed live declared nine
/// acceptance criteria as table rows — `| AC-DL-003 | Native OHLCV ingestion
/// stores … a validation report … |` — and every one of them was invisible to
/// this check. Nothing anywhere asked whether a task had claimed them, so an
/// obligation the PRD makes could go through a whole decomposition with no
/// owner at all, which is exactly what happened: four tasks do ingestion and
/// not one declares the validation report AC-DL-003 demands.
///
/// # Why the prefix is detected rather than listed
///
/// Hardcoding `AC-` would fix one corpus and miss the next, and PRDs in the
/// wild use `AC-`, `BR-`, `NFR-`, `SC-` and more. So any `<PREFIX>-<AREA>-<NNN>`
/// or `<PREFIX>-<NNN>` id in a leading table cell counts, and the families are
/// reported separately — `REQ` coverage is the gate the guide already defines,
/// and the rest are reported beside it rather than folded in.
fn table_obligation_pattern() -> Regex {
    Regex::new(r"^[ \t]*\|[ \t]*([A-Z][A-Z0-9]{1,}(?:-[A-Z0-9]+)?-[0-9]{3})[ \t]*\|")
        .expect("table obligation id pattern is a literal and compiles")
}

/// Headings whose contents state what will NOT be done.
///
/// A non-goal with no owning task is the correct state, not a gap. Reported as
/// one, it is pure noise — and the first run of this check produced six such
/// lines against five real findings, which is how a lint stops being read.
/// Matched on ordinary English rather than an id prefix: `NG-` means non-goal
/// in one corpus and nothing in the next, but a heading that says "non-goals"
/// says it in any of them.
const EXCLUDED_HEADINGS: [&str; 5] = [
    "non-goal",
    "out of scope",
    "excluded",
    "deviation",
    "anti-goal",
];

fn heading_excludes_obligations(line: &str) -> bool {
    let lower = line.trim_start_matches('#').trim().to_ascii_lowercase();
    EXCLUDED_HEADINGS
        .iter()
        .any(|excluded| lower.contains(excluded))
}

/// Every obligation id the PRD states in a table row, grouped by family.
///
/// `REQ` is excluded: the bullet pattern owns it, and counting the same id
/// twice would make one obligation look like two. Rows under a heading that
/// negates — see [`EXCLUDED_HEADINGS`] — are skipped entirely.
///
/// A single-letter prefix (`G-AHDM-001`) is deliberately not matched. Goals are
/// framing, not obligations a task claims, and the two-character minimum is
/// what keeps them out without naming them.
pub(super) fn table_obligation_families(prd: &str) -> BTreeMap<String, BTreeSet<String>> {
    let pattern = table_obligation_pattern();
    let mut families: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut excluded = false;
    let mut header: Option<bool> = None;
    for line in prd.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            excluded = heading_excludes_obligations(line);
            header = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            // A table ends at the first line that is not a row.
            header = None;
            continue;
        }
        // The first row of a table is its header, and it says what the table is
        // FOR. Everything after it inherits that verdict until the table ends.
        let states_obligations = *header.get_or_insert_with(|| header_states_obligations(line));
        if excluded || !states_obligations {
            continue;
        }
        let Some(caps) = pattern.captures(line) else {
            continue;
        };
        let id = caps[1].to_string();
        let Some(family) = id.split('-').next().map(str::to_string) else {
            continue;
        };
        if family == "REQ" {
            continue;
        }
        families.entry(family).or_default().insert(id);
    }
    families
}

/// Words that mean a table row is something the product MUST do.
///
/// A PRD tabulates plenty that is not an obligation — symbol universes,
/// timeframes, provider matrices — and those rows carry ids too. The first real
/// run reported five timeframes as unowned obligations, which is the same noise
/// the non-goal exclusion had just removed. What separates them is not the id
/// prefix but the column header: an obligation table says so at the top.
const OBLIGATION_HEADER_WORDS: [&str; 6] = [
    "criterion",
    "criteria",
    "requirement",
    "obligation",
    "acceptance",
    "must",
];

fn header_states_obligations(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    OBLIGATION_HEADER_WORDS
        .iter()
        .any(|word| lower.contains(word))
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
    let families = table_obligation_families(prd);
    if families.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (family, ids) in families {
        let uncited: Vec<&String> = ids.iter().filter(|id| !claimed.contains(id)).collect();
        if uncited.is_empty() {
            out.push_str(&format!(
                "  {} {family}-* obligation(s) stated in tables; every one is cited by a task.\n",
                ids.len()
            ));
            continue;
        }
        out.push_str(&format!(
            "  {} of {} {family}-* obligation(s) stated in tables are cited by NO task. An \
             obligation nothing claims has no owner, and nothing downstream will notice it \
             was never delivered:\n",
            uncited.len(),
            ids.len()
        ));
        for id in uncited {
            out.push_str(&format!("    {id}\n"));
        }
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
