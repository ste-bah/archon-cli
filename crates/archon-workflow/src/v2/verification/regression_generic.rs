//! Compare every declared command at the base and tip in isolated worktrees.
//! Both paths use the shared runner-result adapters, including named verdicts
//! and summary-only cardinality evidence. Unstartable runs supply no verdict.

use std::collections::BTreeMap;
use std::path::Path;

use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_parse::{failing_tests, ignored_tests, passed_tests};
use crate::v2::write::test_baseline_run::run_in_worktree;
use crate::v2::write::test_baseline_run_base::{
    HostRunVerdict, Tree, cache, cached, host_runnable,
};

/// Each declared command of `commands` that is NOT plain (`host_runnable`)
/// at `commit`, from the cache or run once in a throwaway worktree. A
/// command without a verdict is absent from the map.
pub(crate) async fn generic_verdicts(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    repository_root: &Path,
    commit: &str,
    commands: &[String],
) -> BTreeMap<String, HostRunVerdict> {
    let mut verdicts = BTreeMap::new();
    let mut pending = Vec::new();
    for command in commands.iter().filter(|command| !host_runnable(command)) {
        match cached(store, Tree::RunBase, commit, command) {
            Some(hit) => {
                verdicts.insert(command.clone(), hit);
            }
            None => pending.push(command.clone()),
        }
    }
    if pending.is_empty() || commit.starts_with('-') {
        return verdicts;
    }
    let sha: String = commit.chars().take(12).collect();
    let scratch = store
        .root()
        .join("worktrees")
        .join(format!("regression-{}", sanitize(&sha)));
    if !add_worktree(repository_root, &scratch, commit) {
        return verdicts;
    }
    for command in pending {
        let run = run_in_worktree(dispatch, &scratch, &command, Some(store.run_root())).await;
        if run.error.is_some() || run.timed_out || run.exit_code.is_none() {
            continue;
        }
        let verdict = HostRunVerdict {
            command: command.clone(),
            commit: commit.to_string(),
            exit_code: run.exit_code,
            failing_tests: failing_tests(&run.output),
            passed_tests: passed_tests(&run.output),
            passed_count: crate::v2::write::test_baseline_parse::passed_count(&run.output),
            ignored_tests: ignored_tests(&run.output),
            ids_kept: true,
            ..HostRunVerdict::default()
        };
        cache(store, Tree::RunBase, &verdict);
        verdicts.insert(command, verdict);
    }
    remove_worktree(repository_root, &scratch);
    verdicts
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn git(root: &Path, args: &[&std::ffi::OsStr]) -> bool {
    archon_shell::spawn::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn add_worktree(root: &Path, worktree: &Path, commit: &str) -> bool {
    remove_worktree(root, worktree);
    if let Some(parent) = worktree.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    git(
        root,
        &[
            "worktree".as_ref(),
            "add".as_ref(),
            "--force".as_ref(),
            "--detach".as_ref(),
            worktree.as_os_str(),
            commit.as_ref(),
        ],
    )
}

/// Removes only this worktree (and its leftover directory, the host's own
/// scratch).
fn remove_worktree(root: &Path, worktree: &Path) {
    if worktree.exists() {
        let _ = git(
            root,
            &[
                "worktree".as_ref(),
                "remove".as_ref(),
                "--force".as_ref(),
                worktree.as_os_str(),
            ],
        );
    }
    if worktree.exists() {
        let _ = std::fs::remove_dir_all(worktree);
    }
}

#[cfg(test)]
#[path = "regression_generic_result_tests.rs"]
mod result_tests;
