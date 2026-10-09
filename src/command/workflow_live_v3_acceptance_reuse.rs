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
    let filesystem = filesystem_closure(&context.repository, path)?;
    let tree = tree_closure(&context.repository, path)?;
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
        filesystem,
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

/// Snapshot every component on the literal path and its immediate directory
/// entries. This is the proof for reuse; the Git tree below is only an extra
/// key input. Recursive predicates are not accepted by `bounded_path`.
fn filesystem_closure(repository: &Path, path: &str) -> Option<String> {
    let mut current = repository.to_path_buf();
    let mut entries = Vec::new();
    for component in Path::new(path).components() {
        let std::path::Component::Normal(name) = component else {
            return None;
        };
        let name = name.to_str()?;
        current.push(name);
        let state = filesystem_entry(&current)?;
        let kind = state.get("kind")?.as_str()?.to_owned();
        let traversal_kind = if kind == "symlink" {
            state.get("resolved")?.get("kind")?.as_str()?.to_owned()
        } else {
            kind
        };
        entries.push((name, state));
        if traversal_kind != "directory" {
            // A missing or non-directory ancestor makes all remaining path
            // components unreachable to this literal predicate.
            break;
        }
    }
    let encoded = serde_json::to_vec(&entries).ok()?;
    Some(archon_workflow::task_set_contract::content_digest(&encoded))
}

/// Capture a path's lstat type, symlink target, followed file bytes, or
/// one-level sorted directory listing. Failure to inspect any relevant state
/// keeps this check volatile.
fn filesystem_entry(path: &Path) -> Option<serde_json::Value> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(serde_json::json!({"kind": "missing"}));
        }
        Err(_) => return None,
    };
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path).ok()?.to_str()?.to_owned();
        let followed = std::fs::metadata(path).ok()?;
        return Some(serde_json::json!({
            "kind": "symlink",
            "target": target,
            "resolved": filesystem_object(path, &followed)?,
        }));
    }
    filesystem_object(path, &metadata)
}

fn filesystem_object(path: &Path, metadata: &std::fs::Metadata) -> Option<serde_json::Value> {
    if metadata.is_file() {
        let bytes = std::fs::read(path).ok()?;
        return Some(serde_json::json!({
            "kind": "file",
            "content": archon_workflow::task_set_contract::content_digest(&bytes),
        }));
    }
    if metadata.is_dir() {
        let mut children = Vec::new();
        for entry in std::fs::read_dir(path).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().to_str()?.to_owned();
            let child = std::fs::symlink_metadata(entry.path()).ok()?;
            let kind = if child.file_type().is_symlink() {
                "symlink"
            } else if child.is_file() {
                "file"
            } else if child.is_dir() {
                "directory"
            } else {
                "other"
            };
            let target = child
                .file_type()
                .is_symlink()
                .then(|| {
                    std::fs::read_link(entry.path())
                        .ok()?
                        .to_str()
                        .map(str::to_owned)
                })
                .flatten();
            if child.file_type().is_symlink() && target.is_none() {
                return None;
            }
            children.push((name, kind, target));
        }
        children.sort_by(|left, right| left.0.cmp(&right.0));
        return Some(serde_json::json!({"kind": "directory", "entries": children}));
    }
    Some(serde_json::json!({"kind": "other"}))
}

/// Add the committed Git entries as a secondary key factor. Filesystem state
/// above remains the proof for uncommitted and empty-directory inputs.
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
