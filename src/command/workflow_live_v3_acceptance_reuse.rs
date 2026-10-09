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

pub(super) struct Snapshot {
    key: String,
    operator: String,
    state: serde_json::Value,
}

impl Snapshot {
    #[cfg(test)]
    pub(super) fn key(&self) -> &str {
        &self.key
    }
}

fn cache() -> &'static Mutex<BTreeMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn snapshot(
    context: &StageContext,
    criterion: &AcceptanceCriterion,
) -> Option<Snapshot> {
    let AcceptanceCheck::Command { command, cwd } = &criterion.check else {
        return None;
    };
    let (root, cwd_name) = match cwd {
        TrustedCwd::RepoRoot => (&context.repository, "repository"),
        TrustedCwd::ProjectRoot => (&context.project, "project"),
    };
    let (operator, path) = literal_test(command)?;
    let (filesystem, state) = filesystem_closure(root, path)?;
    let tree = if *cwd == TrustedCwd::RepoRoot {
        tree_closure(root, path)?
    } else {
        "project-root-filesystem-snapshot".to_string()
    };
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
        cwd_name,
        root.display().to_string(),
        path,
        filesystem,
        tree,
        version,
        digest,
        build,
        environment,
        policy,
    ]);
    Some(Snapshot {
        key: archon_workflow::task_set_contract::content_digest(input.to_string().as_bytes()),
        operator: operator.to_owned(),
        state,
    })
}

#[cfg(test)]
pub(super) fn key(context: &StageContext, criterion: &AcceptanceCriterion) -> Option<String> {
    snapshot(context, criterion).map(|snapshot| snapshot.key)
}

fn literal_test(command: &str) -> Option<(&str, &str)> {
    let path =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::bounded_path(command)?;
    let words: Vec<_> = command.split(' ').filter(|word| !word.is_empty()).collect();
    Some((*words.get(1)?, path))
}

pub(super) fn is_literal_test(command: &str) -> bool {
    literal_test(command).is_some()
}

pub(super) fn evaluate_snapshot(
    snapshot: &Snapshot,
    criterion: &AcceptanceCriterion,
) -> Option<CheckResult> {
    let passed = match snapshot.operator.as_str() {
        "-L" => snapshot.state.get("kind")?.as_str()? == "symlink",
        "-e" => object_kind(&snapshot.state) != "missing",
        "-f" => object_kind(&snapshot.state) == "file",
        "-d" => object_kind(&snapshot.state) == "directory",
        "-s" => {
            let sized_state =
                if snapshot.state.get("kind").and_then(|kind| kind.as_str()) == Some("symlink") {
                    snapshot.state.get("resolved")?
                } else {
                    &snapshot.state
                };
            sized_state.get("size")?.as_u64()? > 0
        }
        "-r" | "-w" | "-x" => {
            #[cfg(unix)]
            {
                snapshot
                    .state
                    .get("access")?
                    .get(snapshot.operator.as_str())?
                    .as_bool()?
            }
            #[cfg(not(unix))]
            {
                // Access predicates follow the shell's platform-specific
                // semantics; the snapshot cannot prove them on this target.
                return None;
            }
        }
        _ => return None,
    };
    Some(CheckResult {
        acceptance_id: criterion.id.clone(),
        exit_code: Some(if passed { 0 } else { 1 }),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        environment_note: Some("host-evaluated from filesystem snapshot".into()),
        operational_error: None,
        classification: None,
    })
}

fn object_kind(state: &serde_json::Value) -> &str {
    if state.get("kind").and_then(serde_json::Value::as_str) == Some("symlink") {
        state
            .get("resolved")
            .and_then(|value| value.get("kind"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("missing")
    } else {
        state
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("missing")
    }
}

/// Snapshot every component on the literal path and its immediate directory
/// entries. This is the proof for reuse; the Git tree below is only an extra
/// key input. Recursive predicates are not accepted by `bounded_path`.
fn filesystem_closure(repository: &Path, path: &str) -> Option<(String, serde_json::Value)> {
    let mut current = repository.to_path_buf();
    let mut entries = Vec::new();
    let mut final_state = serde_json::json!({"kind": "missing"});
    let mut reached = true;
    let components: Vec<_> = Path::new(path).components().collect();
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return None;
        };
        let name = name.to_str()?;
        current.push(name);
        let state = filesystem_entry(&current)?;
        if index + 1 == components.len() {
            final_state = state.clone();
        }
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
            if index + 1 < components.len() {
                reached = false;
            }
            break;
        }
    }
    let encoded = serde_json::to_vec(&entries).ok()?;
    if !reached {
        final_state = serde_json::json!({"kind": "missing"});
    }
    Some((
        archon_workflow::task_set_contract::content_digest(&encoded),
        final_state,
    ))
}

/// Capture a path's lstat type, symlink target, followed file bytes, or
/// one-level sorted directory listing. Failure to inspect any relevant state
/// keeps this check volatile.
fn filesystem_entry(path: &Path) -> Option<serde_json::Value> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(
                serde_json::json!({"kind": "missing", "size": 0, "access": {"-r": false, "-w": false, "-x": false}}),
            );
        }
        Err(_) => return None,
    };
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path).ok()?.to_str()?.to_owned();
        let followed = match std::fs::metadata(path) {
            Ok(metadata) => filesystem_object(path, &metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({"kind": "missing", "size": 0})
            }
            Err(_) => return None,
        };
        return Some(serde_json::json!({
            "kind": "symlink",
            "target": target,
            "resolved": followed,
            "access": access_state(path),
        }));
    }
    let mut state = filesystem_object(path, &metadata)?;
    if let Some(object) = state.as_object_mut() {
        object.insert("access".into(), access_state(path));
    }
    Some(state)
}

fn access_state(path: &Path) -> serde_json::Value {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok();
        let check = |mode| {
            path.as_ref()
                .is_some_and(|path| unsafe { libc::access(path.as_ptr(), mode) == 0 })
        };
        serde_json::json!({"-r": check(libc::R_OK), "-w": check(libc::W_OK), "-x": check(libc::X_OK)})
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        serde_json::json!({"-r": false, "-w": false, "-x": false})
    }
}

fn filesystem_object(path: &Path, metadata: &std::fs::Metadata) -> Option<serde_json::Value> {
    if metadata.is_file() {
        let bytes = std::fs::read(path).ok()?;
        return Some(serde_json::json!({
            "kind": "file",
            "size": metadata.len(),
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
        return Some(
            serde_json::json!({"kind": "directory", "size": metadata.len(), "entries": children}),
        );
    }
    Some(serde_json::json!({"kind": "other", "size": metadata.len()}))
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
    let Some(snapshot) = snapshot(context, criterion) else {
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
    take_snapshot(&snapshot)
}

pub(super) fn take_snapshot(
    snapshot: &Snapshot,
) -> (Option<CheckResult>, Decision, Option<String>) {
    let key = snapshot.key.clone();
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
