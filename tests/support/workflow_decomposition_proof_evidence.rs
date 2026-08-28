#![allow(dead_code)]
use super::*;
use std::collections::BTreeMap;

pub fn validate_decomposition_log(path: &Path) -> Result<(), String> {
    let bytes = read_regular_nofollow(path)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("decomposition log {} is not UTF-8: {error}", path.display()))?;
    if text.trim().is_empty() {
        return Err("decomposition log is empty".into());
    }
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = BTreeMap::new();
        for field in line.split_whitespace() {
            let (key, value) = field.split_once('=').ok_or_else(|| {
                format!(
                    "decomposition log line {} has unstructured content",
                    index + 1
                )
            })?;
            if value.is_empty() {
                return Err(format!(
                    "decomposition log line {} has empty field '{key}'",
                    index + 1
                ));
            }
            if fields.insert(key, value).is_some() {
                return Err(format!(
                    "decomposition log line {} repeats field '{key}'",
                    index + 1
                ));
            }
        }
        if fields.contains_key("event") {
            validate_marker_fields(index + 1, &fields)?;
        } else {
            validate_progress_fields(index + 1, &fields)?;
        }
    }
    Ok(())
}

fn validate_marker_fields(line: usize, fields: &BTreeMap<&str, &str>) -> Result<(), String> {
    let expected = BTreeSet::from([
        "event",
        "run_id",
        "binary_revision",
        "script_digest",
        "catalog_digest",
    ]);
    require_exact_log_keys(line, fields, &expected)?;
    if !matches!(
        fields["event"],
        "run_started" | "resume" | "ui_terminal_delivery_deferred"
    ) || !fields["run_id"].starts_with("wf-")
        || !fields["script_digest"]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !fields["catalog_digest"]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "decomposition log marker line {line} has invalid typed values"
        ));
    }
    Ok(())
}

fn validate_progress_fields(line: usize, fields: &BTreeMap<&str, &str>) -> Result<(), String> {
    let has_subject = fields.contains_key("subject");
    let has_digest = fields.contains_key("subject_digest");
    if has_subject == has_digest {
        return Err(format!(
            "decomposition log progress line {line} requires exactly one subject field"
        ));
    }
    let mut expected = BTreeSet::from([
        "event_id",
        "phase",
        "attempt",
        "disposition",
        "findings",
        "status",
        "reused",
    ]);
    expected.insert(if has_subject {
        "subject"
    } else {
        "subject_digest"
    });
    require_exact_log_keys(line, fields, &expected)?;
    if fields["event_id"].parse::<u64>().is_err()
        || (fields["attempt"] != "none" && fields["attempt"].parse::<u32>().is_err())
        || fields["findings"].parse::<usize>().is_err()
        || !matches!(fields["reused"], "true" | "false")
        || !matches!(
            fields["phase"],
            "identity"
                | "acceptance"
                | "skeleton"
                | "bodies"
                | "set_gates"
                | "reconciliation"
                | "completed"
        )
        || !matches!(
            fields["disposition"],
            "none"
                | "pending"
                | "accepted"
                | "accepted_with_shadow_findings"
                | "failed"
                | "blocked"
                | "interrupted"
        )
        || !matches!(
            fields["status"],
            "running" | "accepted" | "noop" | "needs_review" | "failed" | "blocked" | "cancelled"
        )
        || (has_digest
            && (fields["phase"] != "bodies"
                || fields["subject_digest"].len() != 64
                || !fields["subject_digest"]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())))
        || (has_subject && fields["phase"] == "bodies")
    {
        return Err(format!(
            "decomposition log progress line {line} has invalid typed values"
        ));
    }
    Ok(())
}

fn require_exact_log_keys(
    line: usize,
    fields: &BTreeMap<&str, &str>,
    expected: &BTreeSet<&str>,
) -> Result<(), String> {
    let actual = fields.keys().copied().collect::<BTreeSet<_>>();
    if &actual != expected {
        return Err(format!(
            "decomposition log line {line} field set differs: expected={expected:?} actual={actual:?}"
        ));
    }
    Ok(())
}

pub fn copy_decomposition_log(
    project: &Path,
    run_id: &str,
    destination: &Path,
) -> Result<(), String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json"))
            .map_err(|error| format!("reading fixed state: {error}"))?,
    )
    .map_err(|error| format!("parsing fixed state: {error}"))?;
    let task_root = PathBuf::from(
        state["identity"]["task_root_identity"]
            .as_str()
            .ok_or_else(|| "fixed state has no task-root identity".to_string())?,
    )
    .canonicalize()
    .map_err(|error| format!("canonicalizing fixed task root: {error}"))?;
    let path = task_root.join(".decompose.log");
    let persisted = PathBuf::from(
        state["log_path"]
            .as_str()
            .ok_or_else(|| "fixed state has no log_path".to_string())?,
    );
    let persisted_parent = persisted
        .parent()
        .ok_or_else(|| "fixed log path has no parent".to_string())?
        .canonicalize()
        .map_err(|error| format!("canonicalizing fixed log parent: {error}"))?;
    if persisted_parent != task_root
        || persisted.file_name() != Some(std::ffi::OsStr::new(".decompose.log"))
    {
        return Err("fixed log path differs from task-root identity".into());
    }
    validate_decomposition_log(&path)?;
    let bytes = read_regular_nofollow(&path)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(destination, bytes)
        .map_err(|error| format!("copying decomposition log {}: {error}", path.display()))
}

pub fn validate_evidence_manifest(
    root: &Path,
    expected_digest: &str,
) -> Result<EvidenceManifest, String> {
    let path = root.join("manifest.json");
    let manifest: EvidenceManifest = serde_json::from_slice(&read_regular_nofollow(&path)?)
        .map_err(|error| format!("parsing evidence manifest {}: {error}", path.display()))?;
    if manifest.schema_version != 1 || manifest.digest != expected_digest {
        return Err("synthetic evidence manifest identity differs from clearance".into());
    }
    let recomputed = digest_json(&(
        manifest.package.as_str(),
        &manifest.identity,
        &manifest.entries,
    ))?;
    if recomputed != manifest.digest {
        return Err("synthetic evidence manifest digest is invalid".into());
    }
    let declared = manifest
        .entries
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let mut actual_paths = Vec::new();
    collect_regular_files(root, root, &mut actual_paths)?;
    let actual = actual_paths
        .into_iter()
        .map(|path| slash_path(&path))
        .filter(|path| path != "manifest.json" && path != "clearance.json")
        .collect::<BTreeSet<_>>();
    if actual != declared {
        return Err(format!(
            "evidence manifest membership differs: declared={declared:?} actual={actual:?}"
        ));
    }
    for entry in &manifest.entries {
        let relative = Path::new(&entry.relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "evidence manifest path is not confined: {}",
                entry.relative_path
            ));
        }
        let bytes = read_regular_nofollow(&root.join(relative))?;
        if bytes.len() as u64 != entry.byte_len || sha256(&bytes) != entry.sha256 {
            return Err(format!(
                "evidence manifest entry changed: {}",
                entry.relative_path
            ));
        }
    }
    Ok(manifest)
}

pub fn validate_independent_review_artifact(
    root: &Path,
    review: &IndependentReviewReceipt,
) -> Result<(), String> {
    let relative = Path::new(&review.artifact_path);
    if review.artifact_path.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || matches!(
            review.artifact_path.as_str(),
            "review.json" | "manifest.json" | "clearance.json"
        )
    {
        return Err("independent review artifact path is not confined".into());
    }
    let bytes = read_regular_nofollow(&root.join(relative))?;
    if bytes_sha256(&bytes) != review.artifact_sha256 {
        return Err("independent review artifact differs from its receipt".into());
    }
    Ok(())
}

pub fn copy_committed_receipt_entry(
    source: &Path,
    expected_len: u64,
    expected_blake3: &str,
    target: &Path,
) -> Result<(), String> {
    let bytes = read_regular_nofollow(source)?;
    if bytes.len() as u64 != expected_len
        || archon_workflow::task_set_contract::content_digest(&bytes) != expected_blake3
    {
        return Err(format!(
            "receipt destination {} differs from committed bytes",
            source.display()
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if let Ok(metadata) = std::fs::symlink_metadata(target)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(format!(
            "evidence destination {} is not a regular non-symlink file",
            target.display()
        ));
    }
    std::fs::write(target, &bytes).map_err(|error| error.to_string())?;
    let copied = read_regular_nofollow(target)?;
    if copied != bytes
        || copied.len() as u64 != expected_len
        || archon_workflow::task_set_contract::content_digest(&copied) != expected_blake3
    {
        return Err(format!(
            "evidence copy {} differs from committed bytes",
            target.display()
        ));
    }
    Ok(())
}

pub fn collect_host_command_receipts(
    project: &Path,
    run_id: &str,
    destination: &Path,
) -> Result<BTreeSet<String>, String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let mut capabilities = BTreeSet::new();
    for record in v2.load_call_records().map_err(|error| error.to_string())? {
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand
            || !matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Accepted
                    | archon_workflow::WorkflowV2Status::Noop
            )
        {
            continue;
        }
        let outcome: archon_workflow::HostCommandResult =
            serde_json::from_value(record.result.data).map_err(|error| {
                format!("parsing HostCommand result {}: {error}", record.call.id)
            })?;
        let receipt = outcome.publication_receipt.ok_or_else(|| {
            format!(
                "accepted HostCommand {} has no publication receipt",
                record.call.id
            )
        })?;
        if !outcome
            .postcondition
            .as_ref()
            .is_some_and(|postcondition| postcondition.satisfied)
        {
            return Err(format!(
                "accepted HostCommand {} has no satisfied postcondition",
                record.call.id
            ));
        }
        capabilities.insert(receipt.command_id.clone());
        write_json(
            &destination
                .join("receipts")
                .join(format!("{}.json", receipt.call_id)),
            &receipt,
        )?;
        for entry in receipt.entries {
            let source = PathBuf::from(&entry.destination_path);
            let name = format!(
                "{}-{}",
                &entry.blake3[..16],
                source
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("artifact")
            );
            let target = destination.join("published-artifacts").join(name);
            copy_committed_receipt_entry(&source, entry.byte_len, &entry.blake3, &target)?;
        }
    }
    Ok(capabilities)
}

pub fn copy_evidence(project: &Path, run_id: &str, destination: &Path) -> Result<(), String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let run_dir = store.run_dir(run_id);
    copy_if_exists(&run_dir.join("state.json"), &destination.join("state.json"))?;
    copy_if_exists(
        &store.events_path(run_id),
        &destination.join("events.jsonl"),
    )?;
    for relative in [
        "decomposition/state.json",
        "decomposition/command-catalog.json",
        "decomposition/arguments.json",
        "decomposition/provider-route.json",
        "decomposition/interactive-owner.json",
        "v2/checkpoint.json",
        "v2/finalization.json",
        "observer/run-end-acceptance.jsonl",
    ] {
        copy_if_exists(&run_dir.join(relative), &destination.join(relative))?;
    }
    copy_tree_if_exists(&run_dir.join("v2/results"), &destination.join("v2/results"))?;
    Ok(())
}

fn copy_if_exists(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("reading metadata {}: {error}", source.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "evidence source {} is not a regular non-symlink file",
            source.display()
        ));
    }
    let bytes = read_regular_nofollow(source)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(destination, bytes)
        .map_err(|error| format!("copying {}: {error}", source.display()))
}

fn copy_tree_if_exists(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("reading metadata {}: {error}", source.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "evidence tree source {} is not a directory",
            source.display()
        ));
    }
    for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            return Err(format!("evidence tree refuses symlink {}", path.display()));
        }
        if file_type.is_dir() {
            copy_tree_if_exists(&path, &target)?;
        } else if file_type.is_file() {
            copy_if_exists(&path, &target)?;
        } else {
            return Err(format!("evidence tree refuses non-file {}", path.display()));
        }
    }
    Ok(())
}
