use std::path::Path;

use archon_workflow::WorkflowLearningRecord;

pub(crate) fn read_journal(path: &Path) -> anyhow::Result<Vec<WorkflowLearningRecord>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    text.lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// History is one 16-character key per distinct lesson, scoped to one run.
pub(crate) fn journal_paths(run_dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let learning = run_dir.join("learning");
    (
        learning.join("decomposition-records.jsonl"),
        learning.join("decomposition-pending.jsonl"),
    )
}

fn read_seen_keys(path: &Path) -> anyhow::Result<std::collections::BTreeSet<String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(std::collections::BTreeSet::new());
        }
        Err(error) => return Err(error.into()),
    };
    text.lines()
        .map(|line| {
            if line.len() == 16 && line.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(line.to_string());
            }
            let record: WorkflowLearningRecord = serde_json::from_str(line).map_err(|error| {
                anyhow::anyhow!("invalid decomposition lesson history: {error}")
            })?;
            // Upgrade the previous record-bearing journal in place on the next write.
            Ok(super::digest(&record.stage_id))
        })
        .collect()
}

fn write_seen_keys(path: &Path, keys: &std::collections::BTreeSet<String>) -> anyhow::Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    for key in keys {
        writeln!(file, "{key}")?;
    }
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub(crate) fn stage_journal(
    history_path: &Path,
    pending_path: &Path,
    candidates: Vec<WorkflowLearningRecord>,
) -> anyhow::Result<Vec<WorkflowLearningRecord>> {
    let mut known = read_seen_keys(history_path)?;
    let new_records: Vec<_> = candidates
        .into_iter()
        .filter(|record| known.insert(super::digest(&record.stage_id)))
        .collect();
    let pending = merge_records(read_journal(pending_path)?, new_records.clone());
    // The retry source is durable before the seen journal advances. A crash
    // between the writes can therefore replay a pending record, never lose it.
    write_journal(pending_path, &pending)?;
    write_seen_keys(history_path, &known)?;
    Ok(pending)
}

fn merge_records(
    mut records: Vec<WorkflowLearningRecord>,
    candidates: Vec<WorkflowLearningRecord>,
) -> Vec<WorkflowLearningRecord> {
    let mut known = records
        .iter()
        .map(|record| record.stage_id.clone())
        .collect();
    records.extend(super::filter_new_records(candidates, &mut known));
    records
}

pub(crate) fn write_journal(path: &Path, records: &[WorkflowLearningRecord]) -> anyhow::Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    for record in records {
        writeln!(file, "{}", serde_json::to_string(record)?)?;
    }
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn finish_journal_dispatch(path: &Path, dispatched: bool) -> anyhow::Result<()> {
    if dispatched {
        write_journal(path, &[])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::journal_paths;

    #[test]
    fn learning_decompose_journal_paths_keep_the_learning_directory_layout() {
        let run_dir = std::path::Path::new("run");
        let (history, pending) = journal_paths(run_dir);

        assert_eq!(
            history,
            run_dir.join("learning/decomposition-records.jsonl")
        );
        assert_eq!(
            pending,
            run_dir.join("learning/decomposition-pending.jsonl")
        );
    }
}
