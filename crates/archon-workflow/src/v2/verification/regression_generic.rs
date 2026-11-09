//! Compare frozen commands verbatim by exit status and readable per-test verdicts.
//! Each command runs in an isolated copy at the recorded commit, under
//! the existing input tripwire. An absent exit status, launch failure, timeout,
//! or tripwire failure gives no verdict and excuses nothing. Completed exit
//! verdicts are distinct from readable per-test results. Cached entries that
//! do not attest verbatim execution are rerun rather than reused as evidence.

use std::collections::BTreeMap;
use std::path::Path;

use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_parse::{failing_tests, ignored_tests, passed_tests};
use crate::v2::write::test_baseline_run::run_in_worktree;
use crate::v2::write::test_baseline_run_base::{
    HostRunVerdict, Tree, add_worktree, cache, cached, remove_worktree,
};

/// Each frozen command at `commit`, from the cache or run once in a
/// throwaway worktree. A
/// command without a verdict is absent from the map.
pub(crate) async fn generic_verdicts(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    repository_root: &Path,
    commit: &str,
    commands: &[String],
    frozen: &crate::v2::write::test_baseline_run_base::FrozenCommands,
) -> BTreeMap<String, HostRunVerdict> {
    let mut verdicts = BTreeMap::new();
    let mut pending = Vec::new();
    for reported in commands {
        let command = match frozen.command(reported) {
            Ok(command) => command,
            Err(reason) => {
                tracing::debug!(%reported, %reason, "command was not run");
                continue;
            }
        };
        match cached(store, Tree::RunBase, commit, command).filter(|run| run.verbatim) {
            Some(hit) => {
                verdicts.insert(reported.clone(), hit);
            }
            None => pending.push((reported.clone(), command.to_string())),
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
    for (reported, command) in pending {
        let run = run_in_worktree(dispatch, &scratch, &command, Some(store.run_root())).await;
        if run.error.is_some()
            || run.timed_out
            || run.exit_code.is_none()
            || crate::acceptance_check_shell_unset::never_pass(
                &command,
                run.exit_code.unwrap_or(-1),
                &run.output,
            )
        {
            tracing::warn!(%command, error = ?run.error, timed_out = run.timed_out, exit_code = ?run.exit_code, "the host run did not produce a usable completion verdict");
            continue;
        }
        let mut signatures: BTreeMap<String, String> = failing_tests(&run.output)
            .into_iter()
            .map(|id| {
                let signature = crate::v2::write::test_baseline_run_base::signature_at(
                    &run.output,
                    &id,
                    &scratch,
                );
                (id, signature)
            })
            .collect();
        if let Some(identity) =
            crate::v2::write::test_baseline_parse::failure_identity(&run.output, &scratch)
        {
            signatures.insert(String::new(), identity);
        }
        let verdict = HostRunVerdict {
            command: command.clone(),
            commit: commit.to_string(),
            exit_code: run.exit_code,
            failing_tests: failing_tests(&run.output),
            passed_tests: passed_tests(&run.output),
            ignored_tests: ignored_tests(&run.output),
            ids_kept: true,
            isolated: true,
            verbatim: true,
            per_test_verdict: crate::v2::write::test_baseline_parse::harness_reported(&run.output),
            failed_count: crate::v2::write::test_baseline_parse::failed_count(&run.output),
            signatures,
            failure_files: failing_tests(&run.output)
                .into_iter()
                .map(|id| {
                    let files = crate::v2::write::test_baseline_run_base::panic_files(
                        &run.output,
                        &id,
                        &scratch,
                    );
                    (id, files)
                })
                .collect(),
        };
        cache(store, Tree::RunBase, &verdict);
        verdicts.insert(reported, verdict);
    }
    remove_worktree(repository_root, &scratch);
    verdicts
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(all(test, unix))]
#[path = "regression_generic_tests.rs"]
mod tests;
