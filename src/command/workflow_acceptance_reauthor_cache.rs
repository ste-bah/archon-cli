//! Atomic records for candidates that cleared every re-author gate.

use archon_workflow::task_set_contract::{AcceptanceCriterion, JudgeDecision};
use serde::{Deserialize, Serialize};

use super::*;

const RECORDS: &str = "acceptance-reauthor-candidates";
pub(crate) const CONSUMED: &str = "acceptance-reauthor-candidates-consumed";

pub(crate) fn passability_audit_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(CONSUMED).join("passability")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Key {
    check_id: String,
    prd_path: String,
    prd_digest: String,
    criterion: String,
    covers: Vec<String>,
    check: serde_json::Value,
    tree_identity: String,
    baseline_commit: Option<String>,
    judge_model: String,
    judge_provider: Option<String>,
    #[serde(default)]
    prompt_and_judge_source_digest: String,
    #[serde(default)]
    named_file_digest: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    key_digest: String,
    key: Key,
    candidate: AcceptanceCriterion,
}

fn digest(bytes: &[u8]) -> String {
    content_digest(bytes)
}

pub(crate) fn acceptance_logic_identity(
    logic: (u32, String, String),
    model: &str,
    provider: Option<&str>,
) -> serde_json::Value {
    let (version, digest, _this_build) = logic;
    serde_json::json!([version, digest, model, provider])
}

pub(in crate::command::workflow_task_set) fn stamp_judgment(
    entry: &mut AcceptanceCriterion,
    model: &str,
    provider: Option<&str>,
    roots: &super::judge::JudgeRoots,
) {
    stamp_judgment_with_content_digest(entry, model, provider, roots);
}

pub(in crate::command::workflow_task_set) fn stamp_judgment_with_content_digest(
    entry: &mut AcceptanceCriterion,
    model: &str,
    provider: Option<&str>,
    roots: &super::judge::JudgeRoots,
) {
    let identity =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()
            .map(|logic| acceptance_logic_identity(logic, model, provider));
    let named_digest = super::judge::named_files_content_digest(entry, roots);
    let sampling = entry
        .judgment
        .sampling
        .get_or_insert_with(|| serde_json::json!({}));
    if !sampling.is_object() {
        *sampling = serde_json::json!({});
    }
    let fields = sampling.as_object_mut().expect("sampling is an object");
    if let Some(identity) = identity.filter(|_| !fields.contains_key("logic_identity")) {
        fields.insert("logic_identity".into(), identity);
    }
    fields
        .entry("named_file_digest")
        .or_insert_with(|| serde_json::json!(named_digest));
}

/// The complete source closure for the static author prompt and candidate
/// judge instruction. `author.rs` assembles the prompt, `reauthor.rs` owns
/// `ENTRY_SHAPES`, and `live_root.rs` supplies the shared root guidance.
/// `task_set_judge.rs` assembles the adversarial judge instruction, and its
/// named-file evidence helper supplies the contents included in that prompt.
pub(crate) const PROMPT_AND_JUDGE_SOURCES: &[(&str, &[u8])] = &[
    (
        "workflow_acceptance_reauthor_author.rs",
        include_bytes!("workflow_acceptance_reauthor_author.rs"),
    ),
    (
        "workflow_acceptance_reauthor.rs",
        include_bytes!("workflow_acceptance_reauthor.rs"),
    ),
    (
        "workflow_acceptance_passability.rs",
        include_bytes!("workflow_acceptance_passability.rs"),
    ),
    (
        "workflow_acceptance_passability_judge.rs",
        include_bytes!("workflow_acceptance_passability_judge.rs"),
    ),
    (
        "workflow_acceptance_passability_audit.rs",
        include_bytes!("workflow_acceptance_passability_audit.rs"),
    ),
    (
        "workflow_acceptance_passability_evidence.rs",
        include_bytes!("workflow_acceptance_passability_evidence.rs"),
    ),
    (
        "workflow_acceptance_live_root.rs",
        include_bytes!("workflow_acceptance_live_root.rs"),
    ),
    (
        "workflow_task_set_judge.rs",
        include_bytes!("workflow_task_set_judge.rs"),
    ),
    (
        "workflow_task_set_judge_rounds.rs",
        include_bytes!("workflow_task_set_judge_rounds.rs"),
    ),
    (
        "workflow_task_set_judge_named_files.rs",
        include_bytes!("workflow_task_set_judge_named_files.rs"),
    ),
    (
        "workflow_task_set_judge_incomplete.rs",
        include_bytes!("workflow_task_set_judge_incomplete.rs"),
    ),
];

pub(crate) fn prompt_and_judge_source_digest(sources: &[(&str, &[u8])]) -> String {
    let mut closure = Vec::new();
    for (name, source) in sources {
        closure.extend_from_slice(name.as_bytes());
        closure.push(0);
        closure.extend_from_slice(source);
        closure.push(0);
    }
    digest(&closure)
}

fn key(
    frozen: &AcceptanceCriterion,
    candidate: &AcceptanceCriterion,
    scope: &AuthorScope,
    judge_model: &str,
    judge_provider: Option<&str>,
    probe: &dyn ExecutabilityProbe,
) -> Result<Key> {
    let prd = scope.prd_path.canonicalize()?;
    let prd_bytes = std::fs::read(&prd)?;
    let proof_root = scope.repository_root.canonicalize()?;
    let roots = super::judge::JudgeRoots {
        project: Some(scope.project_root.clone()),
        repository: Some(scope.repository_root.clone()),
        task_root: Some(scope.task_root.clone()),
        repository_commit: scope
            .baseline
            .as_ref()
            .map(|baseline| baseline.commit.clone()),
    };
    Ok(Key {
        check_id: frozen.id.clone(),
        prd_path: prd.display().to_string(),
        prd_digest: digest(&prd_bytes),
        criterion: frozen.criterion.clone(),
        covers: frozen.covers.clone(),
        check: serde_json::to_value(&candidate.check)?,
        tree_identity: digest(proof_root.as_os_str().to_string_lossy().as_bytes()),
        baseline_commit: probe.baseline_commit(),
        judge_model: judge_model.to_string(),
        judge_provider: judge_provider.map(str::to_string),
        prompt_and_judge_source_digest: prompt_and_judge_source_digest(PROMPT_AND_JUDGE_SOURCES),
        named_file_digest: super::judge::named_files_content_digest(candidate, &roots),
    })
}

fn record_path(state_dir: &Path, id: &str, key_digest: &str) -> PathBuf {
    state_dir.join(RECORDS).join(format!(
        "{}.{}.{}.json",
        digest(id.as_bytes()),
        key_digest,
        uuid::Uuid::new_v4()
    ))
}

pub(crate) fn load(
    state_dir: &Path,
    frozen: &AcceptanceCriterion,
    scope: &AuthorScope,
    judge_model: &str,
    judge_provider: Option<&str>,
    probe: &dyn ExecutabilityProbe,
) -> Result<Option<AcceptanceCriterion>> {
    let directory = state_dir.join(RECORDS);
    if !directory.exists() {
        return Ok(None);
    }
    let id_prefix = format!("{}.", digest(frozen.id.as_bytes()));
    let mut mismatch = None;
    for item in std::fs::read_dir(&directory)? {
        let path = item?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(&id_prefix) || !name.ends_with(".json") {
            continue;
        }
        let record: Record = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("reading saved acceptance candidate {}", path.display()))?;
        if record.key.check_id != frozen.id {
            mismatch.get_or_insert("check identity changed");
            continue;
        }
        let expected = key(
            frozen,
            &record.candidate,
            scope,
            judge_model,
            judge_provider,
            probe,
        )?;
        // The filename and key_digest are based on the serialized key. Older
        // records include ignored logic_version/logic_digest fields, so their
        // digest differs even when every current key field still matches.
        if record.key != expected {
            mismatch.get_or_insert(
                if record.key.criterion != expected.criterion
                    || record.key.covers != expected.covers
                {
                    "criterion or coverage changed"
                } else if record.key.judge_model != expected.judge_model
                    || record.key.judge_provider != expected.judge_provider
                {
                    "judge changed"
                } else if record.key.prompt_and_judge_source_digest
                    != expected.prompt_and_judge_source_digest
                {
                    "author prompt or judge instruction changed"
                } else if record.key.named_file_digest != expected.named_file_digest {
                    "named judge file content changed"
                } else if record.key.tree_identity != expected.tree_identity {
                    "proof tree changed"
                } else if record.key.baseline_commit != expected.baseline_commit {
                    "proof baseline changed"
                } else {
                    "candidate or PRD identity changed"
                },
            );
            continue;
        }
        if record.candidate.id != frozen.id
            || record.candidate.judgment.verdict != JudgeDecision::Accepted
        {
            mismatch.get_or_insert("record does not contain an accepted candidate for this check");
            continue;
        }
        if let Some(reason) = mismatch {
            eprintln!(
                "acceptance re-author ignored saved check {}: {reason}",
                frozen.id
            );
        }
        return Ok(Some(record.candidate));
    }
    if let Some(reason) = mismatch {
        eprintln!(
            "acceptance re-author ignored saved check {}: {reason}",
            frozen.id
        );
    }
    Ok(None)
}

pub(crate) fn store(
    state_dir: &Path,
    candidate: &AcceptanceCriterion,
    contract: &AcceptanceContract,
    scope: &AuthorScope,
    judge_model: &str,
    judge_provider: Option<&str>,
    probe: &dyn ExecutabilityProbe,
) -> Result<()> {
    let frozen = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .find(|entry| entry.id == candidate.id)
        .ok_or_else(|| anyhow!("candidate is absent from its source contract"))?;
    let key = key(frozen, candidate, scope, judge_model, judge_provider, probe)?;
    let mut saved_candidate = candidate.clone();
    stamp_judgment(
        &mut saved_candidate,
        &key.judge_model,
        key.judge_provider.as_deref(),
        &super::judge::JudgeRoots {
            project: Some(scope.project_root.clone()),
            repository: Some(scope.repository_root.clone()),
            task_root: Some(scope.task_root.clone()),
            repository_commit: scope
                .baseline
                .as_ref()
                .map(|baseline| baseline.commit.clone()),
        },
    );
    let record = Record {
        key_digest: digest(&serde_json::to_vec(&key)?),
        key,
        candidate: saved_candidate,
    };
    let target = record_path(state_dir, &candidate.id, &record.key_digest);
    let bytes = serde_json::to_vec(&record)?;
    write_atomic(&target, &bytes)
}

pub(crate) fn write_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = target.parent().expect("cache path has parent");
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        target.file_name().unwrap().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Forget every saved candidate of check `id`: a gate refused it.
pub(crate) fn forget(state_dir: &Path, id: &str) -> Result<()> {
    let directory = state_dir.join(RECORDS);
    if !directory.exists() {
        return Ok(());
    }
    let prefix = format!("{}.", digest(id.as_bytes()));
    let mut forgotten = Vec::new();
    for item in std::fs::read_dir(&directory)? {
        let path = item?.path();
        if (path.file_name().and_then(|name| name.to_str()))
            .is_some_and(|name| name.starts_with(&prefix))
        {
            forgotten.push(path);
        }
    }
    if forgotten.is_empty() {
        return Ok(());
    }
    let archive = state_dir
        .join(CONSUMED)
        .join(format!("forgotten.{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&archive)?;
    for path in forgotten {
        std::fs::rename(
            &path,
            archive.join(path.file_name().expect("record filename")),
        )?;
    }
    Ok(())
}

/// Whether any candidate is saved under `state_dir`.
pub(crate) fn saved(state_dir: &Path) -> bool {
    std::fs::read_dir(state_dir.join(RECORDS)).is_ok_and(|mut entries| entries.next().is_some())
}

#[cfg(test)]
pub(crate) fn remove(state_dir: &Path) -> Result<()> {
    let records = state_dir.join(RECORDS);
    if records.exists() {
        std::fs::remove_dir_all(records)?;
    }
    Ok(())
}

pub(crate) fn archive(state_dir: &Path, publish_digest: &str) -> Result<()> {
    let records = state_dir.join(RECORDS);
    if !records.exists() {
        return Ok(());
    }
    let archive_root = state_dir.join(CONSUMED);
    std::fs::create_dir_all(&archive_root)?;
    let destination = archive_root.join(format!(
        "{publish_digest}.{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::rename(records, destination)?;
    Ok(())
}

#[cfg(test)]
#[test]
fn atomic_candidate_replacement_never_exposes_partial_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("candidate.json");
    let first = serde_json::to_vec(&"a".repeat(256 * 1024)).unwrap();
    let second = serde_json::to_vec(&"b".repeat(256 * 1024)).unwrap();
    write_atomic(&target, &first).unwrap();
    let reader_path = target.clone();
    let reader = std::thread::spawn(move || {
        for _ in 0..200 {
            let bytes = std::fs::read(&reader_path).unwrap();
            assert!(serde_json::from_slice::<String>(&bytes).is_ok());
        }
    });
    for index in 0..20 {
        write_atomic(&target, if index % 2 == 0 { &second } else { &first }).unwrap();
    }
    reader.join().unwrap();
}

#[cfg(test)]
#[test]
fn archive_twice_with_the_same_publish_digest_preserves_both_records() {
    let dir = tempfile::tempdir().unwrap();
    for marker in ["first", "second"] {
        let records = dir.path().join(RECORDS);
        std::fs::create_dir_all(&records).unwrap();
        std::fs::write(records.join("record"), marker).unwrap();
        archive(dir.path(), "same-digest").unwrap();
    }
    let archives = std::fs::read_dir(dir.path().join(CONSUMED))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(archives.len(), 2);
    let contents = archives
        .iter()
        .map(|archive| std::fs::read_to_string(archive.join("record")).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        contents,
        ["first", "second"].into_iter().map(str::to_owned).collect()
    );
}

#[cfg(test)]
#[test]
fn forgetting_a_check_archives_its_saved_accepted_judgment() {
    let dir = tempfile::tempdir().unwrap();
    let records = dir.path().join(RECORDS);
    std::fs::create_dir_all(&records).unwrap();
    let path = records.join(format!("{}.key.json", digest(b"AC-X-001")));
    std::fs::write(
        &path,
        serde_json::json!({"verdict": "accepted", "check_id": "AC-X-001"}).to_string(),
    )
    .unwrap();

    forget(dir.path(), "AC-X-001").unwrap();

    assert_eq!(
        std::fs::read_dir(&records).unwrap().count(),
        0,
        "forgotten judgments leave the active cache"
    );
    let archived = dir.path().join(CONSUMED);
    let record = std::fs::read_dir(archived)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_to_string(record.join(path.file_name().unwrap())).unwrap(),
        serde_json::json!({"verdict": "accepted", "check_id": "AC-X-001"}).to_string()
    );
}
