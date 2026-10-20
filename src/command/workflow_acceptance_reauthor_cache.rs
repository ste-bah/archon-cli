//! Atomic records for candidates that cleared every re-author gate.

use archon_workflow::task_set_contract::{AcceptanceCriterion, JudgeDecision};
use serde::{Deserialize, Serialize};

use super::*;

const RECORDS: &str = "acceptance-reauthor-candidates";

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
    logic_version: u32,
    logic_digest: String,
    judge_model: String,
    judge_provider: Option<String>,
    #[serde(default)]
    prompt_and_judge_source_digest: String,
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

/// The complete source closure for the static author prompt and candidate
/// judge instruction. `author.rs` assembles the prompt, `reauthor.rs` owns
/// `ENTRY_SHAPES`, and `live_root.rs` supplies the shared root guidance.
/// `task_set_judge.rs` assembles the adversarial judge instruction.
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
        "workflow_acceptance_live_root.rs",
        include_bytes!("workflow_acceptance_live_root.rs"),
    ),
    (
        "workflow_task_set_judge.rs",
        include_bytes!("workflow_task_set_judge.rs"),
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
    let logic = crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
        .iter()
        .find(|logic| logic.id == "freeze-acceptance")
        .ok_or_else(|| anyhow!("freeze-acceptance logic is not catalogued"))?;
    Ok(Key {
        check_id: frozen.id.clone(),
        prd_path: prd.display().to_string(),
        prd_digest: digest(&prd_bytes),
        criterion: frozen.criterion.clone(),
        covers: frozen.covers.clone(),
        check: serde_json::to_value(&candidate.check)?,
        tree_identity: digest(proof_root.as_os_str().to_string_lossy().as_bytes()),
        baseline_commit: probe.baseline_commit(),
        logic_version: logic.version,
        logic_digest: logic.sources_digest.to_string(),
        judge_model: judge_model.to_string(),
        judge_provider: judge_provider.map(str::to_string),
        prompt_and_judge_source_digest: prompt_and_judge_source_digest(PROMPT_AND_JUDGE_SOURCES),
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
        let expected_digest = digest(&serde_json::to_vec(&expected)?);
        if record.key != expected || record.key_digest != expected_digest {
            mismatch.get_or_insert(
                if record.key.criterion != expected.criterion
                    || record.key.covers != expected.covers
                {
                    "criterion or coverage changed"
                } else if record.key.logic_version != expected.logic_version
                    || record.key.logic_digest != expected.logic_digest
                {
                    "acceptance logic changed"
                } else if record.key.judge_model != expected.judge_model
                    || record.key.judge_provider != expected.judge_provider
                {
                    "judge changed"
                } else if record.key.prompt_and_judge_source_digest
                    != expected.prompt_and_judge_source_digest
                {
                    "author prompt or judge instruction changed"
                } else if record.key.tree_identity != expected.tree_identity
                    || record.key.baseline_commit != expected.baseline_commit
                {
                    "proof tree changed"
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
    let record = Record {
        key_digest: digest(&serde_json::to_vec(&key)?),
        key,
        candidate: candidate.clone(),
    };
    let target = record_path(state_dir, &candidate.id, &record.key_digest);
    let bytes = serde_json::to_vec(&record)?;
    write_atomic(&target, &bytes)
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
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

pub(crate) fn remove(state_dir: &Path) -> Result<()> {
    let records = state_dir.join(RECORDS);
    if records.exists() {
        std::fs::remove_dir_all(records)?;
    }
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
