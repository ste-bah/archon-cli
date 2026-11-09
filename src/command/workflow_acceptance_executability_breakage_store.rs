//! Run caching independent of pair ownership; attribution is recomputed.
use super::*;
use crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR;
use crate::command::workflow_task_set::passability::evidence::Redactor;
use archon_workflow::base_snapshot::key_path;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CachedRun {
    pub schema: u32,
    pub key: String,
    pub control: ControlRecord,
    pub outcome: RunOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched: Vec<String>,
    pub evidence: Evidence,
}
pub(super) fn has_evidence(record: &MutantRun) -> bool {
    let Some(control) = record.control_key.get(..16) else {
        return false;
    };
    let base = format!("evidence/{control}/{}", record.mutant_id);
    record.evidence.patch == format!("{base}.patch")
        && record
            .evidence
            .output
            .as_ref()
            .is_none_or(|p| p == &format!("{base}.out"))
}
pub(super) fn root(project: &Path, commit: &str) -> Option<PathBuf> {
    (!commit.is_empty() && commit.bytes().all(|b| b.is_ascii_hexdigit())).then(|| {
        project
            .join(key_path(FREEZE_CACHE_DIR))
            .join("breakage")
            .join(commit)
    })
}
pub(super) async fn keys(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    id: &str,
    patch_digest: &str,
    min_deadline_secs: u64,
) -> Option<(String, String)> {
    let inputs = run::inputs(probe);
    let (contract, id, patch_digest) = (contract.clone(), id.to_string(), patch_digest.to_string());
    tokio::task::spawn_blocking(move || {
        key_pair(&inputs, &contract, &id, &patch_digest, min_deadline_secs)
    })
    .await
    .ok()?
}
fn key_pair(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    id: &str,
    patch_digest: &str,
    min_deadline_secs: u64,
) -> Option<(String, String)> {
    let tree = probe.baseline.as_ref()?;
    let data = sites::data_state(probe, tree).ok()?;
    let memo = sites::reuse::memo_key(probe, tree, &data, contract, id)?;
    let control = content_digest(
        &serde_json::to_vec(&(
            BREAKAGE_LOGIC,
            memo,
            probe.check_bound_secs(),
            min_deadline_secs,
        ))
        .ok()?,
    );
    let run = content_digest(
        &serde_json::to_vec(&(
            &control,
            patch_digest,
            probe.check_bound_secs(),
            min_deadline_secs,
        ))
        .ok()?,
    );
    Some((control, run))
}
pub(super) fn reusable(outcome: &RunOutcome) -> bool {
    match outcome {
        RunOutcome::Killed { exit_code, .. } => *exit_code != 0,
        RunOutcome::Survived { .. }
        | RunOutcome::Unviable { .. }
        | RunOutcome::TimedOut { .. }
        | RunOutcome::Inapplicable { .. } => true,
        _ => false,
    }
}
pub(super) fn read(
    root: &Path,
    key: &str,
    digest: &str,
    id: &str,
) -> Result<Option<CachedRun>, String> {
    let bytes = match std::fs::read(root.join("runs").join(format!("{key}.json"))) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let record: CachedRun = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if record.schema != BREAKAGE_SCHEMA {
        return Err("incompatible schema".into());
    }
    Ok((record.key == key
        && record.control.check_id == id
        && record.control.stable()
        && record.control.check_digest == digest
        && reusable(&record.outcome))
    .then_some(record))
}
pub(super) fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("missing storage parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(uuid::Uuid::new_v4().to_string());
    if !resume::write_owner_only(&tmp, bytes) {
        return Err("writing private evidence".into());
    }
    let result = std::fs::rename(&tmp, path).map_err(|e| e.to_string());
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}
pub(super) fn save(
    root: &Path,
    cached: &CachedRun,
    patch: &[u8],
    output: Option<&[u8]>,
    control: &str,
    mutant: &str,
) -> Result<Evidence, String> {
    let dir = format!("evidence/{}/{}", &control[..16], mutant);
    let evidence = Evidence {
        patch: format!("{dir}.patch"),
        output: output.map(|_| format!("{dir}.out")),
    };
    atomic(&root.join(key_path(&evidence.patch)), patch)?;
    if let Some(output) = output {
        atomic(
            &root.join(key_path(evidence.output.as_ref().expect("output evidence"))),
            output,
        )?;
    }
    let mut cached = cached.clone();
    cached.evidence = evidence.clone();
    atomic(
        &root.join("runs").join(format!("{}.json", cached.key)),
        &serde_json::to_vec_pretty(&cached).map_err(|e| e.to_string())?,
    )?;
    Ok(evidence)
}
pub(super) fn summary_unreadable(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => !serde_json::from_slice::<BreakageProofs>(&bytes)
            .is_ok_and(|p| p.schema == BREAKAGE_SCHEMA),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}
pub(super) fn summary_path(root: &Path, digest: &str) -> Option<PathBuf> {
    (digest.len() >= 16 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| root.join(&digest[..16]).join("summary.json"))
}
pub(super) fn output(probe: &HostProbe, result: &CheckResult) -> Vec<u8> {
    let (env, forwarded) = sites::site_environment(probe);
    let redactor = Redactor::for_environment(env, &forwarded);
    let mut bytes = Vec::new();
    for stream in [&result.stdout, &result.stderr] {
        bytes.extend(redactor.redact_bytes(&stream[stream.len().saturating_sub(32 * 1024)..]));
    }
    bytes
}

pub(super) fn evidence_text(
    report: &BreakageReport,
    run: &MutantRun,
) -> Result<(String, Option<String>), String> {
    evidence_text_at(report.store.as_deref(), run)
}
pub(super) fn evidence_text_at(
    root: Option<&Path>,
    run: &MutantRun,
) -> Result<(String, Option<String>), String> {
    let read = |text: &str| match root {
        None => Ok(text.to_string()),
        Some(root) if !archon_workflow::base_snapshot::outside_key(text) => {
            std::fs::read(root.join(key_path(text)))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|e| e.to_string())
        }
        Some(_) => Err("evidence_path_outside_store".into()),
    };
    Ok((
        read(&run.evidence.patch)?,
        run.evidence.output.as_deref().map(read).transpose()?,
    ))
}
