fn sanitize_call_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

impl WorkflowV2ResultStore {
    pub(crate) fn superseded_branch_outcomes_for_call(
        &self,
        call_id: &str,
    ) -> Vec<(std::time::SystemTime, WorkflowV2BranchOutcome)> {
        let dirs = [
            self.branch_call_dir(call_id),
            self.root.join("branches").join(sanitize_call_id(call_id)),
        ];
        let mut seen = std::collections::BTreeSet::new();
        let mut outcomes = Vec::new();
        for call_dir in dirs {
            if !seen.insert(call_dir.clone()) {
                continue;
            }
            for path in branch_archive_entries(&call_dir.join("superseded")) {
                let Ok(written) = fs::symlink_metadata(&path).and_then(|meta| meta.modified()) else { continue; };
                let Some(raw) = read_store_file_or_report(&path, &call_dir) else { continue; };
                if let Ok(outcome) = serde_json::from_slice(&raw) {
                    outcomes.push((written, outcome));
                }
            }
        }
        outcomes
    }

    pub fn run_id(&self) -> String {
        if self.root.file_name().and_then(|name| name.to_str()) == Some("v2") {
            return self.root.parent().and_then(|path| path.file_name()).and_then(|name| name.to_str()).unwrap_or("unknown-run").to_string();
        }
        self.root.file_name().and_then(|name| name.to_str()).unwrap_or("unknown-run").to_string()
    }

    pub fn rejected_output_path(&self, branch_id: &str) -> PathBuf {
        self.root.join("rejected-outputs").join(format!("{}.json", sanitize_call_id(branch_id)))
    }

    pub fn append_rejected_output(&self, branch_id: &str, record: WorkflowV2RejectedOutput) -> WorkflowResult<PathBuf> {
        self.with_session_write_lock(|| {
            let path = self.rejected_output_path(branch_id);
            let mut log = load_rejected_output_log(&path)?;
            log.branch_id = branch_id.to_string();
            log.rejections.push(record);
            write_json(&path, &log)?;
            Ok(path)
        })
    }
}

fn branch_component(raw: &str) -> String {
    let digest = blake3::hash(raw.as_bytes()).to_hex();
    format!("{}-{}", sanitize_call_id(raw), &digest[..16])
}

fn legacy_branch_outcome_path(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
) -> PathBuf {
    store.root.join("branches").join(sanitize_call_id(call_id)).join(format!("{}.json", sanitize_call_id(item_id)))
}

fn migrate_legacy_branch_outcome(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    target: &Path,
) -> WorkflowResult<()> {
    let legacy = legacy_branch_outcome_path(store, call_id, item_id);
    if legacy == target || !legacy.exists() || target.exists() { return Ok(()); }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
    }
    fs::rename(&legacy, target).map_err(|err| WorkflowError::io(target, err))
}

fn migrate_legacy_branch_archive(
    store: &WorkflowV2ResultStore,
    call_id: &str,
) -> WorkflowResult<()> {
    let call_dirs = [
        store.branch_call_dir(call_id),
        store.root.join("branches").join(sanitize_call_id(call_id)),
    ];
    for call_dir in call_dirs {
        let archive = call_dir.join("superseded");
        let entries = store_dir_entries(&archive);
        // A link can point at a sibling record. Moving only its target would
        // leave the audit link broken, so keep this flat archive readable
        // until a scan can migrate it as a whole.
        if entries.iter().any(|path| {
            fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
        }) {
            continue;
        }
        for path in entries {
            if !path.is_file() { continue; }
            let raw = read_store_file(&path).map_err(|err| WorkflowError::io(&path, err))?;
            let Ok(outcome) = serde_json::from_slice::<WorkflowV2BranchOutcome>(&raw) else { continue; };
            let target_dir = archive.join(branch_component(&outcome.item_id));
            fs::create_dir_all(&target_dir).map_err(|err| WorkflowError::io(&target_dir, err))?;
            let target = target_dir.join(path.file_name().unwrap_or_default());
            fs::rename(&path, &target).map_err(|err| WorkflowError::io(&target, err))?;
        }
    }
    Ok(())
}

fn branch_archive_entries(root: &Path) -> Vec<PathBuf> {
    let mut entries = store_dir_entries(root);
    for child in store_dir_entries(root) {
        if fs::symlink_metadata(&child).is_ok_and(|meta| meta.is_dir()) {
            entries.extend(store_dir_entries(&child));
        }
    }
    entries
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
    fs::rename(&tmp, path).map_err(|err| WorkflowError::io(path, err))
}

/// `value` as a later load returns it: one serde round trip, nothing else.
/// Authoritative records are never log-redacted (Issue-245).
fn as_persisted<T>(value: &T) -> WorkflowResult<T>
where
    T: Serialize + DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value)?).map_err(Into::into)
}
