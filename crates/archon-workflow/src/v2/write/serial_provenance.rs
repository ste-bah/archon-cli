//! Batch K (I1) for the serial write mode: a serial branch writes straight
//! into the canonical checkout, so there is no landing to refuse. What it
//! wrote under the acceptance policy's project inputs is judged all the
//! same, from a snapshot taken before the call: repository test material
//! there becomes a HIGH finding on the branch, the bytes are kept as
//! evidence, and the verdict is `needs_review`. The bytes themselves stay in
//! the working tree for the remediation to remove -- serial mode keeps no
//! pre-image to put back.

use std::collections::BTreeMap;
use std::path::Path;

use crate::v2::{WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status};
use crate::write_coordinator::fixture_provenance::{FixtureIndex, refuse_test_material};
use crate::write_coordinator::project_inputs::{ProjectInputPolicy, file_state};
use crate::write_coordinator::worktree_isolation::run_git;

/// Every path under the project inputs git reports changed or untracked in
/// `repo`, with its state.
pub(super) fn snapshot(repo: &Path, run_root: &Path) -> Option<BTreeMap<String, String>> {
    let policy = ProjectInputPolicy::for_run(run_root)?;
    let output = run_git(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        repo,
    )
    .ok()?;
    let mut paths = BTreeMap::new();
    let mut entries = output.stdout.split(|b| *b == 0);
    while let Some(entry) = entries.next() {
        let entry = String::from_utf8_lossy(entry);
        if entry.len() < 4 {
            continue;
        }
        if entry.starts_with('R') || entry.starts_with('C') {
            entries.next();
        }
        let rel = entry[3..].to_string();
        if policy.covers(&rel) {
            paths.insert(rel.clone(), file_state(&repo.join(&rel)));
        }
    }
    Some(paths)
}

/// Judge what the branch changed under the project inputs since `before`.
pub(super) fn flag_test_material(
    repo: &Path,
    run_root: &Path,
    ids: (&str, &str),
    before: Option<&BTreeMap<String, String>>,
    result: &mut WorkflowV2Result,
) {
    let (Some(before), Some(after)) = (before, snapshot(repo, run_root)) else {
        return;
    };
    let files: Vec<(String, std::path::PathBuf)> = (after.iter())
        .filter(|(rel, state)| before.get(*rel) != Some(*state) && *state != "absent")
        .map(|(rel, _)| (rel.clone(), repo.join(rel)))
        .collect();
    if files.is_empty() {
        return;
    }
    let inputs = ProjectInputPolicy::for_run(run_root)
        .map(|policy| policy.inputs)
        .unwrap_or_default();
    let index = FixtureIndex::load(repo, &inputs);
    let mut findings = Vec::new();
    if refuse_test_material(run_root, &index, ids, &files, &mut findings).is_none() {
        return;
    }
    result.status = WorkflowV2Status::NeedsReview;
    for (n, finding) in findings.into_iter().enumerate() {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!(
                "{}{}_{n}",
                super::super::project_inputs_report::PROJECT_INPUT_FIXTURE_GAP_PREFIX,
                ids.1
            ),
            description: finding,
            severity: Some("high".to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARS: &str =
        "date,open,high,low,close,volume\n2026-01-02,1,2,0.5,1.5,100\n2026-01-03,1.5,2.5,1,2,120\n";

    fn git(repo: &Path, args: &[&str]) {
        run_git(args, repo).expect("git");
    }

    #[test]
    fn a_serial_branch_copying_a_fixture_into_project_inputs_is_a_high_finding() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("tests/fixtures")).unwrap();
        std::fs::create_dir_all(repo.join("data")).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("tests/fixtures/daily.csv"), BARS).unwrap();
        std::fs::write(repo.join("data/prices.csv"), "date,close\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "base",
            ],
        );
        let run_root = repo.join(".archon/workflows/run1");
        std::fs::create_dir_all(&run_root).unwrap();
        crate::write_coordinator::project_inputs::write_test_policy(&run_root, &repo, &["data"]);
        let before = snapshot(&repo, &run_root);
        // What the serial agent wrote in the checkout.
        std::fs::write(repo.join("data/new.csv"), BARS).unwrap();
        std::fs::write(repo.join("data/prices.csv"), "date,close\n2026-01-02,1.5\n").unwrap();
        let mut result = WorkflowV2Result::accepted("done");
        flag_test_material(
            &repo,
            &run_root,
            ("impl", "impl-0"),
            before.as_ref(),
            &mut result,
        );
        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert_eq!(result.residual_gaps.len(), 1, "{:?}", result.residual_gaps);
        assert!(result.residual_gaps[0].description.starts_with(
            "repository test fixture landed as project data: data/new.csv from tests/fixtures/daily.csv"
        ));
        assert_eq!(result.residual_gaps[0].severity.as_deref(), Some("high"));
    }
}
