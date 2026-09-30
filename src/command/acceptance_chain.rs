//! Host side of the frozen-chain lineage check.
//!
//! The in-run acceptance stage and the run-end observer both decide through
//! [`verify_launch_chain`] whether the current pin was reached from the
//! run's launch pin by sanctioned per-check re-authoring. The operator
//! import files surviving versions of a launch chain into the chain history
//! by digest; it accepts only bytes whose digest the run's launch pin or the
//! current pin's lineage names, and writes nothing under the task root.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::AcceptancePin;
use archon_workflow::task_set_lineage::{
    ChainCheck, ChainHistory, ChainProof, ChainRefusal, LaunchLineage, named_digests,
    unrecorded_under_recording, verify_reached_from,
};
use archon_workflow::{
    FinalizationRecordV1, PortableAcceptanceIdentityV1, RunEndAcceptanceObserverSnapshotV1,
    WorkflowEventKind, WorkflowEventLog, WorkflowStore,
};

use crate::cli_args::WorkflowAction;

/// The operator command that files a run's surviving launch chain versions.
pub(crate) fn import_command(run_id: &str) -> String {
    format!("archon workflow import-chain-history {run_id} --from <file>...")
}

/// The launch lineage a run's launch snapshot records: a snapshot with no
/// lineage marker predates lineage recording.
pub(crate) fn launch_lineage(snapshot: &RunEndAcceptanceObserverSnapshotV1) -> LaunchLineage {
    LaunchLineage::from_marker(snapshot.lineage_recording)
}

/// Whether `pin` (with the files under `task_root`) was reached from
/// `launch`; a refusal names the chain check that failed and its remedy.
/// A run launched recording lineage (`launch_lineage`) never adopts a move
/// its pin's lineage does not record, whatever the chain history holds.
pub(crate) fn verify_launch_chain(
    launch: &PortableAcceptanceIdentityV1,
    launch_lineage: LaunchLineage,
    pin: &AcceptancePin,
    pin_path: &Path,
    task_root: &Path,
    run_id: &str,
) -> std::result::Result<ChainProof, String> {
    // Tampering is named as such, never answered with the import remedy.
    if let Some(refusal) = unrecorded_under_recording(launch, launch_lineage, pin) {
        return Err(refusal.to_string());
    }
    verify_reached_from(
        launch,
        launch_lineage,
        pin,
        task_root,
        &ChainHistory::for_pin(pin_path),
    )
    .map_err(|refusal| describe(&refusal, run_id))
}

fn describe(refusal: &ChainRefusal, run_id: &str) -> String {
    match refusal.check {
        ChainCheck::UnrecordedChange | ChainCheck::PreimageCorrupt => format!(
            "{refusal}; if the launch chain's contract and skeleton survive, file them by digest with `{}`",
            import_command(run_id)
        ),
        ChainCheck::HistoryUnavailable | ChainCheck::CurrentUnbound => refusal.to_string(),
        _ => format!("{refusal}; this is not a sanctioned per-check re-author of the launch chain"),
    }
}

/// The run's launch snapshot: the finalization record's once the run
/// finalized, else the one its generated metadata saved at launch.
pub(crate) fn launch_snapshot(
    store: &WorkflowStore,
    run_id: &str,
) -> Result<RunEndAcceptanceObserverSnapshotV1> {
    let run_dir = store.run_dir(run_id);
    let finalization = run_dir.join("v2/finalization.json");
    if finalization.exists() {
        let record: FinalizationRecordV1 = read_json(&finalization)?;
        if let Some(snapshot) = record.observer_snapshot {
            return Ok(snapshot);
        }
    }
    let metadata = run_dir.join("v2/generated-metadata.json");
    let value: serde_json::Value = read_json(&metadata)?;
    match value.get("observer_snapshot") {
        Some(snapshot) if !snapshot.is_null() => Ok(serde_json::from_value(snapshot.clone())?),
        _ => Err(anyhow!(
            "run {run_id} recorded no launch-time acceptance snapshot, so it has no launch chain"
        )),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

/// One file an import filed.
#[derive(Debug)]
pub(crate) struct Imported {
    pub(crate) source: PathBuf,
    pub(crate) digest: String,
    pub(crate) stored: bool,
}

/// File `sources` into the chain history of `run_id`'s task set. Every
/// source is checked before any is filed: one digest the launch pin or the
/// pin's lineage does not name refuses the whole import.
pub(crate) fn import_history(
    project: &Path,
    run_id: &str,
    sources: &[PathBuf],
) -> Result<Vec<Imported>> {
    if sources.is_empty() {
        return Err(anyhow!(
            "import-chain-history needs at least one --from file"
        ));
    }
    let store = WorkflowStore::project(project);
    let snapshot = launch_snapshot(&store, run_id)?;
    let launch = snapshot.portable_acceptance_identity.ok_or_else(|| {
        anyhow!("run {run_id} recorded no launch pin identity, so it has no launch chain")
    })?;
    let task_root = PathBuf::from(&snapshot.canonical_task_root_identity);
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, &task_root);
    let pin: AcceptancePin = read_json(&pin_path)?;
    let named = named_digests(&launch, &pin);
    let history = ChainHistory::for_pin(&pin_path);
    let mut read = Vec::new();
    for source in sources {
        let bytes =
            std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
        let digest = archon_workflow::task_set_contract::content_digest(&bytes);
        if !named.contains(&digest) {
            let refusal = history
                .import(&bytes, &named)
                .expect_err("an unnamed digest is refused");
            return Err(anyhow!(
                "{}: {refusal}; nothing was imported",
                source.display()
            ));
        }
        read.push((source.clone(), bytes));
    }
    let mut imported = Vec::new();
    for (source, bytes) in read {
        let (digest, stored) = history
            .import(&bytes, &named)
            .map_err(|refusal| anyhow!("{}: {refusal}", source.display()))?;
        store.with_run_lock(run_id, |locked| {
            let seq = locked.next_event_seq(run_id)?;
            WorkflowEventLog::new(locked.clone())
                .emit(
                    run_id,
                    seq,
                    WorkflowEventKind::HostCommandCompleted,
                    serde_json::json!({
                        "event": "acceptance_chain_history_imported",
                        "digest": digest,
                        "source": source.display().to_string(),
                        "stored": stored,
                        "history": history.path(&digest).display().to_string(),
                    }),
                )
                .map(|_| ())
        })?;
        imported.push(Imported {
            source,
            digest,
            stored,
        });
    }
    Ok(imported)
}

/// `import-chain-history` and `observe-run-end`; `false` for any other
/// action.
pub(crate) async fn handle_cli(action: &WorkflowAction, cwd: &Path) -> Result<bool> {
    match action {
        WorkflowAction::ImportChainHistory { run_id, from } => {
            let sources = from
                .iter()
                .map(|path| {
                    if path.is_absolute() {
                        path.clone()
                    } else {
                        cwd.join(path)
                    }
                })
                .collect::<Vec<_>>();
            for imported in import_history(cwd, run_id, &sources)? {
                println!(
                    "{} {} {}",
                    if imported.stored {
                        "imported"
                    } else {
                        "already filed"
                    },
                    imported.digest,
                    imported.source.display()
                );
            }
            Ok(true)
        }
        WorkflowAction::ObserveRunEnd { run_id } => {
            print!(
                "{}",
                crate::command::workflow_live::reobserve::observe_run_end(cwd, run_id).await?
            );
            Ok(true)
        }
        _ => Ok(false),
    }
}
