//! In-process reuse for the live acceptance round. Only a literal filesystem
//! predicate has a read set the host can prove; other commands are volatile.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceCriterion, TrustedCwd};

use super::StageContext;

#[derive(Clone, serde::Serialize)]
pub(super) struct Decision {
    pub(super) reused: bool,
    pub(super) why: String,
    pub(super) evidence: Option<String>,
}

struct Cached {
    result: CheckResult,
    evidence: String,
}

fn cache() -> &'static Mutex<BTreeMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn key(context: &StageContext, criterion: &AcceptanceCriterion) -> Option<String> {
    let AcceptanceCheck::Command {
        command,
        cwd: TrustedCwd::RepoRoot,
    } = &criterion.check
    else {
        return None;
    };
    let path =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::bounded_path(command)?;
    let tree = tree_closure(&context.repository, path)?;
    let dirty = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(&context.repository)
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            path,
        ])
        .output()
        .ok()?;
    if !dirty.status.success() || !dirty.stdout.is_empty() {
        return None;
    }
    let (version, digest, build) =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()?;
    let host = archon_workflow::acceptance_check_environment::host_environment();
    let environment_policy = match &context.binding {
        Some(binding) => {
            archon_workflow::acceptance_check_environment::CheckPolicy::configured(&binding.policy)
        }
        None => archon_workflow::acceptance_check_environment::CheckPolicy::default_for(&host),
    };
    let mut environment = archon_workflow::acceptance_check_environment::site_variables(
        &host,
        &environment_policy,
        &[],
    );
    environment.extend(
        archon_workflow::acceptance_check_environment::forwarded_values(
            &host,
            &environment_policy.forwarded,
        )
        .ok()?,
    );
    let policy = match &context.binding {
        Some(binding) => serde_json::to_value(&binding.policy).ok()?,
        None => serde_json::Value::Null,
    };
    let input = serde_json::json!([
        context.run_id,
        context.repository,
        criterion.id,
        criterion.criterion.as_bytes(),
        criterion.covers,
        command.as_bytes(),
        path,
        tree,
        version,
        digest,
        build,
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::shell_binary_digest(
            environment.get("PATH").map(String::as_str),
        )?,
        environment,
        policy,
    ]);
    Some(archon_workflow::task_set_contract::content_digest(
        input.to_string().as_bytes(),
    ))
}

/// The bounded path entries are exact in the tree object, including mode and
/// blob digest. A symlink in the path makes the read closure volatile.
fn tree_closure(repository: &Path, path: &str) -> Option<String> {
    let head = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !head.status.success() {
        return None;
    }
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let entries = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-r", "-t", "-z", &commit, "--", path])
        .output()
        .ok()?;
    if !entries.status.success() {
        return None;
    }
    let mut closure = Vec::from(path.as_bytes());
    for entry in entries.stdout.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let tab = entry.iter().position(|byte| *byte == b'\t')?;
        let mode = entry[..tab].split(|byte| *byte == b' ').next()?;
        let found = std::str::from_utf8(&entry[tab + 1..]).ok()?;
        if mode == b"120000"
            && (path == found
                || path
                    .strip_prefix(found)
                    .is_some_and(|rest| rest.starts_with('/')))
        {
            return None;
        }
        closure.extend_from_slice(entry);
        closure.push(0);
    }
    Some(archon_workflow::task_set_contract::content_digest(&closure))
}

pub(super) fn take(
    context: &StageContext,
    criterion: &AcceptanceCriterion,
) -> (Option<CheckResult>, Decision, Option<String>) {
    let Some(key) = key(context, criterion) else {
        return (
            None,
            Decision {
                reused: false,
                why: "check read closure is unbounded or repository state is unavailable".into(),
                evidence: None,
            },
            None,
        );
    };
    match cache().lock().ok().and_then(|cache| {
        cache
            .get(&key)
            .map(|cached| (cached.result.clone(), cached.evidence.clone()))
    }) {
        Some((result, evidence)) => (
            Some(result),
            Decision {
                reused: true,
                why: "check, repository closure, logic, and environment are identical".into(),
                evidence: Some(evidence),
            },
            Some(key),
        ),
        None => (
            None,
            Decision {
                reused: false,
                why: "no prior verdict for the complete input closure".into(),
                evidence: None,
            },
            Some(key),
        ),
    }
}

pub(super) fn save(key: String, result: &CheckResult, evidence: String) {
    if result.operational_error.is_some() {
        return;
    }
    if let Ok(mut cache) = cache().lock() {
        cache.insert(
            key,
            Cached {
                result: result.clone(),
                evidence,
            },
        );
    }
}

pub(super) fn audit_bytes(
    decisions: &BTreeMap<String, Decision>,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(decisions)
}
