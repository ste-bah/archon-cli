// Revocation of stored branch outcomes by an explicit restart (Issue-266).
//
// A restart used to delete only a branch's current outcome
// (`branches/<call>/<item>.json`). Its superseded outcomes stayed in
// `branches/<call>/superseded/`, where the branch cache reads them for the
// landed-task set and for the landing record, ahead of every hash check: a
// superseded `patch_landed` record was restored as the current one and the
// restarted task was never run again.
//
// Revoking a branch moves every stored outcome of it -- the current one and
// each superseded one -- into `branches/<call>/revoked/`. No reuse reader
// looks there: they read the files of the call directory and its
// `superseded/` directory only. The records are kept, for audit; they are
// only out of reach of reuse.

impl WorkflowV2ResultStore {
    /// Revoke every stored outcome of `(call_id, item_id)`, current and
    /// superseded. `true` when anything was revoked.
    pub fn revoke_branch_outcome(&self, call_id: &str, item_id: &str) -> WorkflowResult<bool> {
        let dir = self.root.join("branches").join(sanitize_call_id(call_id));
        revoke_branch_in_dir(&dir, item_id)
    }

    /// Revoke every branch, of any call, that a stored outcome (current or
    /// superseded) records as completing one of `task_ids`.
    pub(super) fn revoke_branch_outcomes_for_tasks(
        &self,
        task_ids: &BTreeSet<String>,
    ) -> WorkflowResult<Vec<WorkflowV2DeletedBranchOutcome>> {
        let root = self.root.join("branches");
        let calls = match fs::read_dir(&root) {
            Ok(calls) => calls,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(WorkflowError::io(&root, err)),
        };
        let mut revoked = Vec::new();
        for call_dir in calls {
            let call_dir = call_dir.map_err(|err| WorkflowError::io(&root, err))?;
            if !call_dir
                .file_type()
                .map_err(|err| WorkflowError::io(call_dir.path(), err))?
                .is_dir()
            {
                continue;
            }
            let dir = call_dir.path();
            let mut stored = Vec::new();
            load_outcomes_from_dir(&dir, &mut stored)?;
            stored.extend(superseded_outcomes_in(&dir));
            // One entry per branch: the task ids of all its stored outcomes.
            let mut branches: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
            for outcome in stored {
                let call_id = outcome
                    .completion_evidence
                    .iter()
                    .find(|evidence| !evidence.call_id.trim().is_empty())
                    .map(|evidence| evidence.call_id.clone())
                    .unwrap_or_else(|| call_dir.file_name().to_string_lossy().into_owned());
                let entry = branches
                    .entry(outcome.item_id.clone())
                    .or_insert_with(|| (call_id, BTreeSet::new()));
                entry.1.extend(branch_outcome_task_ids(&outcome));
            }
            for (item_id, (call_id, branch_tasks)) in branches {
                if branch_tasks.is_disjoint(task_ids) {
                    continue;
                }
                if revoke_branch_in_dir(&dir, &item_id)? {
                    revoked.push(WorkflowV2DeletedBranchOutcome {
                        call_id,
                        item_id,
                        task_ids: branch_tasks.into_iter().collect(),
                    });
                }
            }
        }
        Ok(revoked)
    }
}

/// The readable outcomes in `dir/superseded/`. An unreadable file is
/// skipped, as every reader of the archive skips it.
fn superseded_outcomes_in(dir: &Path) -> Vec<WorkflowV2BranchOutcome> {
    let Ok(entries) = fs::read_dir(dir.join("superseded")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .filter_map(|path| fs::read(path).ok())
        .filter_map(|raw| serde_json::from_slice(&raw).ok())
        .collect()
}

/// Move the current outcome of `item_id` in the call directory `dir`, and
/// each superseded one, into `dir/revoked/`.
fn revoke_branch_in_dir(dir: &Path, item_id: &str) -> WorkflowResult<bool> {
    let revoked = dir.join("revoked");
    let mut moved = false;
    let current = dir.join(format!("{}.json", sanitize_call_id(item_id)));
    if current.is_file() {
        archive_file_into(&current, &revoked)?;
        moved = true;
    }
    let Ok(entries) = fs::read_dir(dir.join("superseded")) else {
        return Ok(moved);
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let names_item = fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<WorkflowV2BranchOutcome>(&raw).ok())
            .is_some_and(|outcome| outcome.item_id == item_id);
        if names_item {
            archive_file_into(&path, &revoked)?;
            moved = true;
        }
    }
    Ok(moved)
}

/// Rename `path` into `dir` under a unique name: its stem, the time, the
/// process and a sequence number. A rename keeps the file's write time,
/// which the superseded readers order by.
fn archive_file_into(path: &Path, dir: &Path) -> WorkflowResult<PathBuf> {
    fs::create_dir_all(dir).map_err(|err| WorkflowError::io(dir, err))?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("record");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let sequence = SUPERSEDED_ARCHIVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let target = dir.join(format!(
        "{stem}-{stamp}-{}-{sequence}.json",
        std::process::id()
    ));
    fs::rename(path, &target).map_err(|err| WorkflowError::io(&target, err))?;
    Ok(target)
}
