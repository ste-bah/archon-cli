//! Replace a guessed write scope with one bound to evidence.
//!
//! # The root cause this attacks
//!
//! An item declares the files it will change BEFORE anything has read the code,
//! and `write_mode::plan` then builds "disjoint" waves out of those guesses.
//! Every gate downstream exists to cope with that one bad input: the ownership
//! rejection that discards a correct patch for one unlisted path, the scope
//! grant that hands it back, the overlap guard, and the stale-baseline recheck.
//! Two items wanting the same undeclared file is not an edge case, it is the
//! guaranteed consequence of planning from prophecy.
//!
//! `write_scope_extension`'s own header says it plainly — "the declared scope
//! cannot be right in advance" — and the answer to that is to stop declaring in
//! advance, not to grade the prophecy more kindly.
//!
//! # Why this is not simply another guess
//!
//! A discovery pass is still an agent, and an agent that over-declares is now
//! WORSE than one that under-declares: an inflated scope contests paths across
//! the wave, and one live item declared 69 files and collided. So a discovered
//! path is accepted only against evidence:
//!
//! - it was READ during the pass — the agent opened it, so it exists and was
//!   examined; or
//! - it does not exist but its parent directory does — a new file in a real
//!   location, which is the one case reading cannot cover and which
//!   `write_scope_extension` names as its own non-guarantee.
//!
//! Anything else is a path the pass invented, and it is dropped.
//!
//! Deliverables declared by the task's contract are always kept, whatever the
//! pass says: those are the files the task exists to produce, and an earlier
//! failure lost a 455-line source file precisely by dropping one.
//!
//! # What it does not fix
//!
//! Over-declaration within what was read is still possible: an agent can open
//! sixty files and claim all sixty. Read-evidence bounds the claim to reality,
//! not to necessity. The scope grant remains the backstop for the other
//! direction — it should now fire rarely rather than routinely.

use std::collections::BTreeSet;
use std::path::Path;

/// What a read-only discovery pass reported.
#[derive(Debug, Clone, Default)]
pub(super) struct DiscoveredScope {
    /// Files the pass says it will change.
    pub declared: Vec<String>,
    /// Files the pass actually opened.
    pub read: Vec<String>,
}

/// The scope to plan with, or `None` to keep the guess.
///
/// `None` is the fail-safe answer and is returned whenever the pass produced
/// nothing usable: no declaration, or every declaration unevidenced. Keeping
/// the guess reproduces today's behaviour exactly, which is the correct
/// fallback for a stage that is an optimisation rather than a gate.
pub(super) fn accepted_scope(
    discovered: &DiscoveredScope,
    contract_required: &[String],
    repository_root: Option<&Path>,
) -> Option<Vec<String>> {
    // Both sides normalised before they are compared. A pass may report the
    // same file as `src/lib.rs`, `<root>/src/lib.rs` or `src\\lib.rs`, and a raw
    // string comparison would call a file it definitely read "unevidenced" —
    // the same two-path-languages mistake that makes an owned path look
    // unclaimed elsewhere in this layer.
    let read: BTreeSet<String> = discovered
        .read
        .iter()
        .filter_map(|path| canonical_form(path, repository_root))
        .collect();
    let read: BTreeSet<&str> = read.iter().map(String::as_str).collect();

    let mut accepted: BTreeSet<String> = discovered
        .declared
        .iter()
        .filter_map(|path| canonical_form(path, repository_root))
        .filter(|path| path_is_evidenced(path, &read, repository_root))
        .collect();
    if accepted.is_empty() {
        return None;
    }
    // Contract deliverables are not the pass's to drop.
    accepted.extend(contract_required.iter().cloned());
    Some(accepted.into_iter().collect())
}

/// One repository-relative spelling of `path`, or `None` if it is not a path
/// this repository can name at all.
fn canonical_form(path: &str, repository_root: Option<&Path>) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Some(root) = repository_root else {
        return Some(trimmed.to_string());
    };
    archon_write_plan::normalize_target(trimmed, root)
        .ok()
        .map(|normalized| normalized.as_str().to_string())
}

/// Whether a declared path is backed by evidence rather than invention.
///
/// `path` is already canonical, so containment has been decided by
/// `normalize_target` — which rejects `..`, empty segments and absolute
/// escapes, and carries a Windows rooted-path case a hand-rolled
/// `Path::starts_with` check had no idea about. A first draft used exactly that
/// lexical check and accepted `../sibling/new.rs` whenever the sibling
/// directory happened to exist.
fn path_is_evidenced(path: &str, read: &BTreeSet<&str>, repository_root: Option<&Path>) -> bool {
    if read.contains(path) {
        return true;
    }
    // A file that does not exist yet cannot have been read. Accept it only
    // where it would actually live: an existing directory inside the
    // repository. Without a root there is nothing to resolve against.
    let Some(root) = repository_root else {
        return false;
    };
    let absolute = root.join(path);
    if absolute.exists() {
        // It exists but was never opened: the pass is claiming a file it did
        // not look at, which is the guess this stage replaces.
        return false;
    }
    absolute.parent().is_some_and(Path::is_dir)
}

/// The read-only brief. Deliberately says nothing about any particular task or
/// repository: the item's own input carries that, and a task-specific word here
/// would be a PRD baked into the engine.
const SCOPE_DISCOVERY_TASK: &str = "Read-only scope discovery. Do NOT modify, create or delete any file. \
Read the repository until you know exactly which files this item must change to satisfy its acceptance criteria, \
then report that set. Record every file you opened in files_read, and report the files you will change as a JSON \
array of repository-relative paths in data.planned_target_files. Declare a file ONLY if you opened it and it must \
change, or if it does not exist yet and must be created; a path you did not read and that already exists will be \
discarded. Declaring files you will not change is as harmful as omitting files you will: an inflated scope collides \
with the other items running beside this one. Return a normal result envelope with no files_changed.";

/// Replace each branch's guessed write scope with an evidence-bound one.
///
/// Best-effort by construction. A branch whose pass fails, times out, or
/// returns nothing usable keeps the scope it already had, so the worst outcome
/// is today's behaviour plus one wasted read-only turn.
#[allow(clippy::too_many_arguments)]
pub(super) async fn discover_write_scopes(
    branches: &mut [crate::WorkflowV2FanoutItem],
    target_repository_root: Option<&str>,
    execution: &crate::WorkflowV2CallExecution,
    adapter: &crate::v2::agent_adapter::WorkflowV2AgentAdapter,
    dispatch: &dyn crate::WorkflowAgentDispatch,
    v2_store: &crate::v2::result_store::WorkflowV2ResultStore,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) {
    // Only a write-capable fanout has a scope worth discovering. The guard is
    // cheap insurance rather than a known case: this entry point is reached for
    // every Fanout and Parallel call, and running a read-only turn per branch
    // of a read-only fanout would double the cost of the stages that do the
    // reading in the first place.
    if execution.call.write_mode.is_none() {
        return;
    }
    let repository_root = target_repository_root.map(Path::new);
    let artifact_roots =
        crate::v2::project_artifacts::project_artifact_context_from_v2_root(v2_store.root())
            .artifact_roots;

    // Concurrent, bounded by the same limit the wave itself runs under. Serial
    // discovery would add one full turn per item ahead of any writing, which on
    // a fifteen-item PRD is an hour of wall clock spent before the first line
    // is written.
    let limit = dispatch.fanout_parallelism(execution.call.options.max_parallelism);
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(limit.max(1)));
    let passes = branches.iter().enumerate().map(|(index, branch)| {
        let semaphore = semaphore.clone();
        async move {
            let _permit = semaphore.acquire_owned().await.ok()?;
            let discovered = run_scope_pass(
                branch,
                target_repository_root,
                execution,
                adapter,
                dispatch,
                v2_store,
                task_universe,
            )
            .await?;
            Some((index, discovered))
        }
    });
    let discovered: Vec<Option<(usize, DiscoveredScope)>> =
        futures_util::future::join_all(passes).await;

    for (index, discovered) in discovered.into_iter().flatten() {
        let branch = &mut branches[index];
        let contract_required = task_universe
            .zip(branch.input.get("item"))
            .map(|(universe, item)| {
                crate::v2::contract_code_targets::contract_code_targets_for_item(
                    universe,
                    item,
                    &artifact_roots,
                )
            })
            .unwrap_or_default();
        let Some(accepted) = accepted_scope(&discovered, &contract_required, repository_root)
        else {
            continue;
        };
        branch.call.options.target_files = accepted.clone();
        if let Some(object) = branch
            .input
            .get_mut("item")
            .and_then(serde_json::Value::as_object_mut)
        {
            object.insert("target_files".to_string(), serde_json::json!(accepted));
        }
    }
}

/// One branch's read-only pass, or `None` if it produced nothing usable.
#[allow(clippy::too_many_arguments)]
async fn run_scope_pass(
    branch: &crate::WorkflowV2FanoutItem,
    target_repository_root: Option<&str>,
    execution: &crate::WorkflowV2CallExecution,
    adapter: &crate::v2::agent_adapter::WorkflowV2AgentAdapter,
    dispatch: &dyn crate::WorkflowAgentDispatch,
    v2_store: &crate::v2::result_store::WorkflowV2ResultStore,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) -> Option<DiscoveredScope> {
    let scope_execution = crate::WorkflowV2CallExecution {
        call: scope_discovery_call(branch),
        input: branch.input.clone(),
        depends_on: vec![execution.call.id.clone()],
    };
    let result = dispatch
        .run_call(
            SCOPE_DISCOVERY_TASK,
            target_repository_root.map(str::to_string),
            &scope_execution,
            adapter,
            Some(v2_store),
            task_universe,
        )
        .await
        .ok()?;
    let declared = planned_target_files(&result)?;
    Some(DiscoveredScope {
        declared,
        read: result.files_read.iter().map(|f| f.path.clone()).collect(),
    })
}

/// The read-only call this branch's discovery pass runs as.
///
/// Extracted so the read-only guarantee is testable without a dispatcher. Both
/// halves matter and neither implies the other: `write_mode = None` is what
/// `is_write_capable` reads, and the method must stop being `Implementation` —
/// a branch call carries that whether or not it can write, and leaving it would
/// describe a read-only pass as an implementation to anything that keys off the
/// method rather than the mode.
pub(super) fn scope_discovery_call(
    branch: &crate::WorkflowV2FanoutItem,
) -> crate::WorkflowV2HostCall {
    let mut call = branch.call.clone();
    call.id = format!("{}-scope-discovery", branch.id);
    call.write_mode = None;
    call.method = crate::WorkflowV2HostMethod::Agent;
    call.options.target_files = Vec::new();
    call.options.target_files_from_item = false;
    call.options.task = Some(SCOPE_DISCOVERY_TASK.to_string());
    call.options.extra.remove("target_ownership_scopes");
    call.options.extra.remove("wave_claims");
    call
}

/// The pass's declared scope, under either spelling it might use.
fn planned_target_files(result: &crate::WorkflowV2Result) -> Option<Vec<String>> {
    for key in ["planned_target_files", "plannedTargetFiles", "target_files"] {
        if let Some(values) = result.data.get(key).and_then(serde_json::Value::as_array) {
            let paths: Vec<String> = values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect();
            if !paths.is_empty() {
                return Some(paths);
            }
        }
    }
    None
}
