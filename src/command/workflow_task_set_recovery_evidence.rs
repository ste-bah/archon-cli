//! Recovery authority comes from the durable unfreeze log and digest-filed
//! original bytes, rather than a mutable completion receipt alone.
use super::*;
use archon_workflow::task_set_contract::{ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE};

pub(super) fn targets(pin: &Path, tasks: &Path) -> Vec<PathBuf> {
    // Normalize trusted roots once: /var and /private/var (and configured
    // store/task-root aliases) name the same evidence, never a foreign set.
    let tasks = tasks
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| tasks.to_path_buf());
    let parent = pin.parent().map(|parent| {
        parent
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap_or_else(|_| parent.to_path_buf())
    });
    let canonical_pin = match (&parent, pin.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => pin.to_path_buf(),
    };
    let mut paths = vec![
        tasks.join(ACCEPTANCE_LOCK_FILE),
        tasks.join(TASK_SKELETON_LOCK_FILE),
        canonical_pin,
    ];
    if let (Some(parent), Some(name)) = (parent, pin.file_name()) {
        paths.push(parent.join("check-sources").join(name));
    }
    paths
}
fn aside(target: &Path, transaction: &str) -> Result<PathBuf> {
    let name = target
        .file_name()
        .ok_or_else(|| anyhow!("recovery target has no name"))?
        .to_string_lossy();
    Ok(target.with_file_name(format!(".{name}.unverified-{transaction}")))
}

fn archive(history: &ChainHistory, bytes: &[u8]) -> Result<String> {
    let root = history
        .dir()
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("history has no pin store"))?;
    validate_existing_parents(history.dir(), root)?;
    create_dir_all_durably(history.dir())?;
    let (digest, _) = history.put(bytes)?;
    sync_parent(&history.path(&digest))?;
    Ok(digest)
}

pub(super) fn capture(pin: &Path, tasks: &Path, record: &mut Recovery) -> Result<()> {
    let history = ChainHistory::for_pin(pin);
    let targets = targets(pin, tasks);
    let canonical_pin = &targets[2];
    for target in &targets {
        let retained = aside(target, &record.transaction)?;
        let source = if retained.exists() { &retained } else { target };
        if std::fs::symlink_metadata(source).is_ok() {
            let root = if target.starts_with(&record.task_root) {
                record.task_root.as_path()
            } else {
                canonical_pin
                    .parent()
                    .ok_or_else(|| anyhow!("pin has no parent"))?
            };
            validate_existing_parents(source, root)?;
        }
        match std::fs::read(source) {
            Ok(bytes) => {
                let digest = archive(&history, &bytes)?;
                record.evidence.insert(target.clone(), digest);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    // Keep only preimages named by an authenticated prior pin or launch.
    // Other bytes cannot become an authority for the launch-contract checks.
    let anchors = record
        .prior
        .iter()
        .map(AcceptancePin::identity)
        .chain(record.runs.values().cloned())
        .collect::<Vec<_>>();
    for name in [ACCEPTANCE_CONTRACT_FILE, TASK_SKELETON_FILE] {
        let Ok(bytes) = std::fs::read(tasks.join(name)) else {
            continue;
        };
        let digest = content_digest(&bytes);
        if anchors.iter().any(|anchor| {
            anchor.acceptance_digest == digest || anchor.skeleton_digest.as_ref() == Some(&digest)
        }) {
            archive(&history, &bytes)?;
            if name == TASK_SKELETON_FILE {
                record.skeleton = serde_json::from_slice(&bytes).ok();
            }
        }
    }
    Ok(())
}

pub(crate) fn authority(pin: &Path, transaction: &str) -> Result<Option<serde_json::Value>> {
    let (records, _) = read(pin)?;
    let Some(record) = records
        .into_iter()
        .find(|record| record.transaction == transaction)
    else {
        return Ok(None);
    };
    let mut record = record;
    record.completed = None;
    Ok(Some(serde_json::to_value(record)?))
}

pub(super) fn validate(record: &Recovery, pin: &Path, tasks: &Path) -> Result<()> {
    if record.task_root != tasks.canonicalize().map(archon_shell::paths::plain)?
        || !super::super::publish::valid_recovery_transaction(&record.transaction)
    {
        return Err(anyhow!(
            "recovery authority has a different task root or invalid transaction"
        ));
    }
    let mut anchor = record.clone();
    anchor.completed = None;
    let expected = serde_json::to_value(&anchor)?;
    let log = std::fs::read_to_string(pin.with_extension("publish-recovery.log"))?;
    let logged = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|event| {
            event["event"] == "task_set_publish_recovered"
                && event["source"] == "legacy"
                && event["outcome"] == "unfrozen for re-freeze"
                && event["transaction"] == record.transaction
                && event.get("authority") == Some(&expected)
        });
    if !logged {
        return Err(anyhow!(
            "recovery authority is not bound to a durable unfreeze log entry"
        ));
    }
    let history = ChainHistory::for_pin(pin);
    let allowed = targets(pin, tasks);
    for (target, digest) in &record.evidence {
        if !allowed.contains(target) {
            return Err(anyhow!("recovery evidence names a foreign target"));
        }
        let bytes = history
            .get(digest)?
            .ok_or_else(|| anyhow!("recovery evidence preimage is missing"))?;
        let retained = aside(target, &record.transaction)?;
        match std::fs::read(&retained) {
            Ok(retained) if content_digest(&retained) != *digest => {
                return Err(anyhow!("moved-aside recovery digest changed"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if target == &allowed[2] && record.prior.is_some() {
            let prior: AcceptancePin = serde_json::from_slice(&bytes)?;
            if serde_json::to_value(&prior)? != serde_json::to_value(&record.prior)? {
                return Err(anyhow!(
                    "recovery prior pin does not match its moved-aside digest"
                ));
            }
        }
    }
    if record.prior.is_some() && !record.evidence.contains_key(&allowed[2]) {
        return Err(anyhow!("recovery prior has no moved-aside pin digest"));
    }
    Ok(())
}

pub(crate) fn cleanup_adopted(pin: &Path, tasks: &Path) -> Result<()> {
    let (records, _) = read(pin)?;
    if !pin.exists() {
        return Ok(());
    }
    let live: AcceptancePin = serde_json::from_slice(&std::fs::read(pin)?)?;
    for record in records {
        match &record.completed {
            Some(done) if live.lineage.starts_with(&done.lineage) => {}
            None if record.prior.is_none() && record.runs.is_empty() => {}
            _ => continue,
        }
        super::super::publish::verify_recovered_chain(pin, tasks)
            .map_err(|error| anyhow!(error))?;
        validate(&record, pin, tasks)?;
        for target in targets(pin, tasks) {
            let retained = aside(&target, &record.transaction)?;
            if std::fs::symlink_metadata(&retained).is_err() {
                continue;
            }
            let parent = retained
                .parent()
                .ok_or_else(|| anyhow!("recovery target has no parent"))?;
            validate_destination(&retained, &[(parent.to_path_buf(), None)])?;
            match std::fs::remove_file(&retained) {
                Ok(()) => sync_parent(&retained)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

/// A recovery keeps the authenticated contract's fields and check ordering;
/// authoring replaces its checks, rather than inventing new launch policy.
pub(crate) fn refreeze_base(
    pin: &Path,
    tasks: &Path,
) -> Result<Option<archon_workflow::task_set_contract::AcceptanceContract>> {
    let (records, _) = read(pin)?;
    let Some(record) = records
        .iter()
        .rev()
        .find(|record| record.completed.is_none())
    else {
        return Ok(None);
    };
    if let Err(error) = validate(record, pin, tasks) {
        tracing::warn!(%error, "recovery template deferred; authority retained for retry");
        return Ok(None);
    }
    let anchor = record
        .prior
        .as_ref()
        .map(AcceptancePin::identity)
        .or_else(|| record.runs.values().next().cloned());
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    let Some(bytes) = ChainHistory::for_pin(pin).get(&anchor.acceptance_digest)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}
