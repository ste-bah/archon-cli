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
//
// A branch is selected by every task id its outcomes name: their completion
// evidence, and the task ids of their result (`canonical_task_ids` and the
// other shapes `completion_evidence::canonical_task_ids_from_result` reads),
// which is what the landed-task set counts (`write::landed_task_ids`).
//
// A revocation is planned from a complete scan before anything moves. An
// archive that cannot be listed or read fails the restart with the path
// named, and nothing is revoked. A file that reads but does not parse as an
// outcome is left: no reader of the archive can parse it either, so it can
// never be reused. A failure while moving leaves the remaining files where
// they were; repeating the restart selects and moves them.

/// What one revocation moves: each stored outcome file of each branch.
struct RevocationPlan {
    moves: Vec<PathBuf>,
    revoked: Vec<WorkflowV2DeletedBranchOutcome>,
}

impl WorkflowV2ResultStore {
    /// Revoke every stored outcome of `(call_id, item_id)`, current and
    /// superseded. `true` when anything was revoked.
    pub fn revoke_branch_outcome(&self, call_id: &str, item_id: &str) -> WorkflowResult<bool> {
        let dir = self.root.join("branches").join(sanitize_call_id(call_id));
        let files = branch_files_in(&dir, item_id, &stored_outcomes_in(&dir)?)?;
        self.move_revoked(&files)?;
        Ok(!files.is_empty())
    }

    /// The revocation of every branch, of any call, that a stored outcome
    /// (current or superseded) names as doing one of `task_ids`. Nothing
    /// moves until [`Self::execute_revocation`].
    fn plan_revocation_for_tasks(
        &self,
        task_ids: &BTreeSet<String>,
    ) -> WorkflowResult<RevocationPlan> {
        let mut plan = RevocationPlan {
            moves: Vec::new(),
            revoked: Vec::new(),
        };
        let root = self.root.join("branches");
        let calls = match fs::read_dir(&root) {
            Ok(calls) => calls,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(plan),
            Err(err) => return Err(WorkflowError::io(&root, err)),
        };
        for call_dir in calls {
            let call_dir = call_dir.map_err(|err| WorkflowError::io(&root, err))?;
            let dir = call_dir.path();
            if !call_dir
                .file_type()
                .map_err(|err| WorkflowError::io(&dir, err))?
                .is_dir()
            {
                continue;
            }
            // A FIFO, socket or device can never be read safely here, yet a
            // reader could still take outcome bytes from it later: quarantine
            // it into revoked/ unread.
            plan.moves.extend(special_files_in(&dir, false)?);
            plan.moves
                .extend(special_files_in(&dir.join("superseded"), true)?);
            let stored = stored_outcomes_in(&dir)?;
            // One entry per branch: the task ids of all its stored outcomes.
            let mut branches: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
            for (_, outcome) in &stored {
                let call_id = outcome
                    .completion_evidence
                    .iter()
                    .find(|evidence| !evidence.call_id.trim().is_empty())
                    .map(|evidence| evidence.call_id.clone())
                    .unwrap_or_else(|| call_dir.file_name().to_string_lossy().into_owned());
                let entry = branches
                    .entry(outcome.item_id.clone())
                    .or_insert_with(|| (call_id, BTreeSet::new()));
                entry.1.extend(revocation_task_ids(outcome));
            }
            for (item_id, (call_id, branch_tasks)) in branches {
                if branch_tasks.is_disjoint(task_ids) {
                    continue;
                }
                let files = branch_files_in(&dir, &item_id, &stored)?;
                if files.is_empty() {
                    continue;
                }
                plan.moves.extend(files);
                plan.revoked.push(WorkflowV2DeletedBranchOutcome {
                    call_id,
                    item_id,
                    task_ids: branch_tasks.into_iter().collect(),
                });
            }
        }
        Ok(plan)
    }

    fn execute_revocation(
        &self,
        plan: RevocationPlan,
    ) -> WorkflowResult<Vec<WorkflowV2DeletedBranchOutcome>> {
        self.move_revoked(&plan.moves)?;
        Ok(plan.revoked)
    }

    /// Move each file into the `revoked/` directory of its call directory.
    fn move_revoked(&self, files: &[PathBuf]) -> WorkflowResult<()> {
        for file in files {
            let parent = file.parent().unwrap_or_else(|| Path::new("."));
            let call_dir =
                if parent.file_name().and_then(|name| name.to_str()) == Some("superseded") {
                    parent.parent().unwrap_or(parent)
                } else {
                    parent
                };
            archive_file_into(file, &call_dir.join("revoked"), self.durable)?;
        }
        Ok(())
    }
}

/// Every task id an outcome names: its completion evidence and its result.
fn revocation_task_ids(outcome: &WorkflowV2BranchOutcome) -> BTreeSet<String> {
    let mut ids = branch_outcome_task_ids(outcome);
    if let Some(result) = &outcome.result {
        ids.extend(super::completion_evidence::canonical_task_ids_from_result(
            result,
        ));
    }
    ids
}

/// Every stored outcome in the call directory `dir`, current and
/// superseded, with its file. A directory or a file that cannot be listed
/// or read is an error naming it; a superseded file that does not parse is
/// left out (see the module comment).
fn stored_outcomes_in(dir: &Path) -> WorkflowResult<Vec<(PathBuf, WorkflowV2BranchOutcome)>> {
    let mut stored = Vec::new();
    for path in outcome_files_in(dir, false)? {
        if let Some(outcome) = read_store_record(&path)? {
            stored.push((path, outcome));
        }
    }
    for path in outcome_files_in(&dir.join("superseded"), true)? {
        let raw = fs::read(&path).map_err(|err| WorkflowError::io(&path, err))?;
        if let Ok(outcome) = serde_json::from_slice::<WorkflowV2BranchOutcome>(&raw) {
            stored.push((path, outcome));
        }
    }
    Ok(stored)
}

/// Every entry reuse can read, including symlinks. Current outcomes use
/// `.json`; the landing reader accepts every filename in the archive.
/// An unreadable link is included so reading it fails the plan before mutation.
fn outcome_files_in(dir: &Path, archived: bool) -> WorkflowResult<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(WorkflowError::io(dir, err)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| WorkflowError::io(dir, err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| WorkflowError::io(&path, err))?;
        // A FIFO, socket or device is never an outcome and reading one could
        // block while restart holds the run lock; a link is kept unless it
        // resolves to such a file, and an unresolvable link fails the plan.
        let readable = file_type.is_file()
            || (file_type.is_symlink()
                && fs::metadata(&path).map_or(true, |target| target.is_file()));
        if readable
            && (archived || path.extension().and_then(|value| value.to_str()) == Some("json"))
        {
            files.push(path);
        }
    }
    Ok(files)
}

/// Entries reuse could name but this scan cannot read without blocking: a
/// FIFO, socket or device, or a link resolving to one.
fn special_files_in(dir: &Path, archived: bool) -> WorkflowResult<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(WorkflowError::io(dir, err)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| WorkflowError::io(dir, err))?;
        let path = entry.path();
        let named = archived || path.extension().and_then(|value| value.to_str()) == Some("json");
        let special = fs::metadata(&path).is_ok_and(|target| !target.is_file() && !target.is_dir());
        if named && special {
            files.push(path);
        }
    }
    Ok(files)
}

/// The stored files of `item_id` in `dir`: its current outcome file, read
/// or not, and each superseded outcome naming it.
fn branch_files_in(
    dir: &Path,
    item_id: &str,
    stored: &[(PathBuf, WorkflowV2BranchOutcome)],
) -> WorkflowResult<Vec<PathBuf>> {
    let current = dir.join(format!("{}.json", sanitize_call_id(item_id)));
    let mut files = match fs::symlink_metadata(&current) {
        Ok(_) => vec![current.clone()],
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(WorkflowError::io(&current, err)),
    };
    files.extend(
        stored
            .iter()
            .filter(|(path, outcome)| *path != current && outcome.item_id == item_id)
            .map(|(path, _)| path.clone()),
    );
    Ok(files)
}

/// Rename `path` into `dir` under a unique name: its stem, the time, the
/// process and a sequence number. A rename keeps the file's write time,
/// which the superseded readers order by. Durable: both directories are
/// synced after the rename.
fn archive_file_into(path: &Path, dir: &Path, durable: bool) -> WorkflowResult<PathBuf> {
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
    if durable {
        crate::durable_io::sync_dir(dir)?;
        if let Some(source) = path.parent() {
            crate::durable_io::sync_dir(source)?;
        }
    }
    Ok(target)
}
