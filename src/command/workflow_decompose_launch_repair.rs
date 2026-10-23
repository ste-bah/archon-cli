//! Re-authoring the frozen checks a launch's rules refuse (Issue 387, item 3).
//! Kept outside `workflow_decompose_frozen_chain`, so the verify-frozen
//! capabilities' logic closure does not take in the re-author.
//!
//! The workflow repairs, it does not refuse: a frozen check this build's
//! acceptance entry rules refuse goes through the sanctioned per-check
//! repair (`reauthor_and_republish`, the path `freeze-acceptance --reauthor`
//! takes) with each refusal as its author's first finding. Only those checks
//! are re-authored and re-judged, by the freeze-time judge; every other entry
//! stays byte-identical, and the chain (contract, lock, skeleton, skeleton
//! lock, pin) is republished in one transaction. The launch then reads the
//! frozen chain again and verifies it in place like any other frozen stage.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::{WorkflowEventKind, WorkflowEventLog, WorkflowLlmClient, WorkflowStore};

use crate::command::workflow_decompose_frozen_chain::{
    FrozenChainSnapshot, LaunchAcceptance, frozen_chain_snapshot,
};
use crate::command::workflow_task_set::executability::ExecutabilityProbe;
use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate};
use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};

#[path = "workflow_decompose_launch_repair_pending.rs"]
mod pending;
#[cfg(test)]
pub(super) use pending::set_after_reauthor_published_hook;
#[cfg(test)]
use pending::take_after_reauthor_published_hook;

/// The event detail that records a launch repair once the run exists.
pub(crate) const REAUTHORED_EVENT: &str = "frozen_acceptance_checks_reauthored";

#[cfg(test)]
#[path = "workflow_decompose_launch_repair_tests.rs"]
mod tests;

fn compare_repaired_chain(
    launched: &FrozenChainSnapshot,
    republished: &FrozenChainSnapshot,
    old_skeleton_bytes: Option<&[u8]>,
    new_skeleton_bytes: Option<&[u8]>,
    new_acceptance_digest: &str,
    old_bodies: &std::collections::BTreeMap<String, Vec<u8>>,
    new_bodies: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let old_files: std::collections::BTreeMap<_, _> =
        launched.preserved_digests.iter().cloned().collect();
    let new_files: std::collections::BTreeMap<_, _> =
        republished.preserved_digests.iter().cloned().collect();
    for (file, old_digest) in &old_files {
        if file == archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE
            || file == archon_workflow::task_set_contract::TASK_SKELETON_FILE
        {
            continue;
        }
        let Some(new_digest) = new_files.get(file) else {
            return Err(format!(
                "{file} disappeared after repair (old digest {old_digest}, new missing)"
            ));
        };
        if old_digest != new_digest {
            return Err(format!(
                "{file} changed after repair (old digest {old_digest}, new digest {new_digest})"
            ));
        }
    }
    for file in new_files.keys() {
        if !old_files.contains_key(file) {
            return Err(format!(
                "{file} appeared after repair (old missing, new digest {})",
                new_files[file]
            ));
        }
    }
    if launched.skeleton != republished.skeleton {
        return Err(format!(
            "frozen skeleton state changed after repair (old {}, new {})",
            launched.skeleton, republished.skeleton
        ));
    }
    if launched.skeleton && old_bodies != new_bodies {
        let file = old_bodies
            .keys()
            .chain(new_bodies.keys())
            .find(|file| old_bodies.get(*file) != new_bodies.get(*file))
            .cloned()
            .unwrap_or_else(|| "<task body inventory>".to_string());
        let old = old_bodies
            .get(&file)
            .map_or_else(|| "<missing>".to_string(), |bytes| diagnostic_digest(bytes));
        let new = new_bodies
            .get(&file)
            .map_or_else(|| "<missing>".to_string(), |bytes| diagnostic_digest(bytes));
        return Err(format!(
            "{file} body digest changed after repair (old digest {old}, new digest {new})"
        ));
    }
    if launched.skeleton {
        let old_skeleton: serde_json::Value = serde_json::from_slice(
            old_skeleton_bytes.ok_or("task-skeleton.json before repair was not captured")?,
        )
        .map_err(|error| format!("task-skeleton.json before repair is invalid: {error}"))?;
        let new_skeleton: serde_json::Value = serde_json::from_slice(
            new_skeleton_bytes.ok_or("task-skeleton.json after repair was not captured")?,
        )
        .map_err(|error| format!("task-skeleton.json after repair is invalid: {error}"))?;
        let mut old_skeleton_fields = old_skeleton
            .as_object()
            .cloned()
            .ok_or("task-skeleton.json before repair is not an object")?;
        let mut new_skeleton_fields = new_skeleton
            .as_object()
            .cloned()
            .ok_or("task-skeleton.json after repair is not an object")?;
        let old_skeleton_binding = old_skeleton_fields
            .get("acceptance_digest")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<missing>");
        let new_skeleton_binding = new_skeleton_fields
            .get("acceptance_digest")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<missing>");
        if new_skeleton_binding != new_acceptance_digest {
            return Err(format!(
                "task-skeleton.json.acceptance_digest does not match the republished acceptance contract (old digest {}, new digest {})",
                diagnostic_digest(old_skeleton_binding.as_bytes()),
                diagnostic_digest(new_skeleton_binding.as_bytes())
            ));
        }
        old_skeleton_fields.remove("acceptance_digest");
        new_skeleton_fields.remove("acceptance_digest");
        if old_skeleton_fields != new_skeleton_fields {
            let field = old_skeleton_fields
                .keys()
                .chain(new_skeleton_fields.keys())
                .find(|field| old_skeleton_fields.get(*field) != new_skeleton_fields.get(*field))
                .map(String::as_str)
                .unwrap_or("<structure>");
            let old = old_skeleton_fields.get(field);
            let new = new_skeleton_fields.get(field);
            let old_text = old.map_or_else(|| "<missing>".into(), serde_json::Value::to_string);
            let new_text = new.map_or_else(|| "<missing>".into(), serde_json::Value::to_string);
            return Err(format!(
                "task-skeleton.json.{field} changed after repair (old digest {}, new digest {})",
                diagnostic_digest(old_text.as_bytes()),
                diagnostic_digest(new_text.as_bytes())
            ));
        }
    }
    if launched.to_argument() != republished.to_argument() {
        return Err(format!(
            "frozenChain stages/subjects/bodies changed (old digest {}, new digest {})",
            diagnostic_digest(launched.to_argument().to_string().as_bytes()),
            diagnostic_digest(republished.to_argument().to_string().as_bytes())
        ));
    }
    Ok(())
}

fn diagnostic_digest(value: &[u8]) -> String {
    archon_workflow::task_set_contract::content_digest(value)
}

fn body_contents(
    task_root: &Path,
    snapshot: &FrozenChainSnapshot,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    snapshot
        .bodies
        .iter()
        .map(|file| {
            std::fs::read(task_root.join(file))
                .with_context(|| format!("reading frozen task body {file}"))
                .map(|bytes| (file.clone(), bytes))
        })
        .collect()
}

/// Re-author every check `decision` routed, with `client` as author and
/// judge and `probe` as the executability gate. Returns the line naming the
/// republished chain, and the event detail recording the repair. A repair
/// that cannot finish is an error naming the checks it was routed for, the
/// rule that refused each one, and why it stopped.
pub(crate) async fn reauthor_refused(
    client: &dyn WorkflowLlmClient,
    probe: &dyn ExecutabilityProbe,
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    decision: &LaunchAcceptance,
) -> Result<(String, serde_json::Value)> {
    let ids: BTreeSet<String> = decision.reauthor.keys().cloned().collect();
    let named = ids.iter().cloned().collect::<Vec<_>>().join(",");
    let scope = AuthorScope::for_task_set(project_root, task_root, prd_path)
        .map_err(|why| anyhow!("re-authoring frozen check(s) {named}: {why}"))?;
    let trigger = format!(
        "workflow decompose launch: this build's acceptance entry rules refused check(s) {named} ({})",
        decision.recorded
    );
    let result = reauthor_and_republish(
        client,
        ReauthorRequest {
            project_root,
            tasks_root: task_root,
            prd_path,
            ids: &ids,
            gate: ReauthorGate {
                probe,
                seeds: &decision.reauthor,
            },
            trigger: &trigger,
        },
        &scope,
    )
    .await
    .with_context(|| {
        format!(
            "the launch routed frozen check(s) {named} to re-authoring because this build's acceptance entry rules refuse them ({}), and the repair did not finish",
            decision.reauthor.values().cloned().collect::<Vec<_>>().join(" | ")
        )
    })?;
    for diagnostic in &result.diagnostics {
        eprintln!("{diagnostic}");
    }
    for (id, reason) in &result.unproven {
        let owner = match result.authored_pending.contains_key(id) {
            true => "its author",
            false => "the host",
        };
        eprintln!("acceptance check {id} remains pending for {owner}: {reason}");
    }
    let published = ids
        .iter()
        .filter(|id| !result.unproven.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let pending = result.unproven.keys().cloned().collect::<Vec<_>>();
    let authored_pending = (pending.iter())
        .filter(|id| result.authored_pending.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let line = format!(
        "frozen_chain_launch acceptance=reauthored published_ids={} pending_ids={} acceptance_digest={} freeze_event={} skeleton_digest={}",
        published.join(","),
        pending.join(","),
        result.acceptance_digest,
        result.freeze_event_id,
        result.skeleton_digest.as_deref().unwrap_or("none")
    );
    let detail = serde_json::json!({
        "event": REAUTHORED_EVENT,
        "ids": ids,
        "published_ids": published,
        "pending_ids": pending,
        "authored_pending_ids": authored_pending,
        "refusals": decision.reauthor,
        "recorded_logic": decision.recorded,
        "acceptance_digest": result.acceptance_digest,
        "freeze_event_id": result.freeze_event_id,
        "skeleton_digest": result.skeleton_digest,
        "unproven": result.unproven,
    });
    Ok((line, detail))
}

/// The launch's acceptance step, run only once the launch holds the claim
/// on its task root (`create_claimed_run`): a chain another run owns is never
/// re-authored. Writes `decision`'s lines to the operator log at `log_path`
/// first, so a repair that then fails is still on record; re-authors what
/// this build's rules refuse with a client from `factory`; records the
/// repair as an event of run `run_id`; and requires the republished chain to
/// be the one the run was launched with (`launched`: the same stages,
/// subjects and bodies). Every line is also shown through `ui`.
pub(crate) async fn after_claim(
    factory: &dyn archon_workflow::WorkflowLlmClientFactory,
    decision: LaunchAcceptance,
    launched: &mut FrozenChainSnapshot,
    (store, run_id, log_path, ui): (
        &WorkflowStore,
        &str,
        &Path,
        &archon_workflow::SharedWorkflowUiSink,
    ),
    (project_root, prd_path, task_root, repository_root): (&Path, &Path, &Path, &Path),
    cargo_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<()> {
    let log =
        |line: &str| crate::command::workflow_decompose_log::append_nofollow_line(log_path, line);
    let mut lines = decision.lines.clone();
    for line in &lines {
        log(line)?;
    }
    if !decision.reauthor.is_empty() {
        let skeleton_path = task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
        let old_skeleton_bytes = if launched.skeleton {
            Some(std::fs::read(&skeleton_path).with_context(|| {
                format!(
                    "reading frozen task skeleton before repair at {}",
                    skeleton_path.display()
                )
            })?)
        } else {
            None
        };
        let old_bodies = if launched.skeleton {
            body_contents(task_root, launched)?
        } else {
            Default::default()
        };
        let repaired = async {
            let client = factory
                .build_client(archon_workflow::WorkflowLlmClientRequest {
                    cwd: project_root.to_path_buf(),
                    origin: "workflow-decompose-launch-reauthor".into(),
                    session_id: format!(
                        "workflow-decompose-launch-reauthor-{}",
                        uuid::Uuid::new_v4()
                    ),
                    read_roots: vec![repository_root.to_path_buf()],
                })
                .await
                .context("building the client that re-authors refused frozen checks")?;
            let probe = crate::command::workflow_task_set::executability::HostProbe::for_task_set_with_limits(
                project_root,
                task_root,
                cargo_limits.clone(),
            );
            reauthor_refused(
                client.as_ref(),
                &probe,
                (project_root, prd_path, task_root),
                &decision,
            )
            .await
        }
        .await;
        let (line, detail) = match repaired {
            Ok(repaired) => repaired,
            Err(error) => {
                // A judge refutation is a content result. Hand it to the
                // normal acceptance author loop in the fixed script; provider,
                // IO, and publication failures remain launch errors.
                // reauthor_refused reaches ReauthorStalled when a judge
                // repeatedly refuses the authored candidate; that typed
                // content result still routes back into the phase author loop.
                if error
                    .downcast_ref::<crate::command::workflow_task_set::reauthor::ReauthorStalled>()
                    .is_some()
                {
                    log(&format!(
                        "frozen_chain_launch acceptance=reauthor_refuted_routed ids={} detail={}",
                        decision
                            .reauthor
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(","),
                        crate::command::workflow_decompose_events::log_field(&format!("{error:#}"))
                    ))?;
                    launched.acceptance = false;
                    launched.skeleton = false;
                    launched.subjects.clear();
                    launched.bodies.clear();
                    launched.body_digests.clear();
                    return Ok(());
                }
                log(&format!(
                    "frozen_chain_launch acceptance=reauthor_failed ids={} error={}",
                    decision
                        .reauthor
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(","),
                    crate::command::workflow_decompose_events::log_field(&format!("{error:#}"))
                ))?;
                return Err(error);
            }
        };
        log(&line)?;
        let seq = store.next_event_seq(run_id)?;
        WorkflowEventLog::new(store.clone()).emit(
            run_id,
            seq,
            WorkflowEventKind::DecompositionPhaseCompleted,
            detail.clone(),
        )?;
        lines.push(line);
        #[cfg(test)]
        if let Some(hook) = take_after_reauthor_published_hook(task_root) {
            hook(task_root);
        }
        // Validate the published chain before clearing the launch snapshot for
        // a pending route; even a partial repair must preserve frozen inputs.
        let republished = frozen_chain_snapshot(project_root, prd_path, task_root)
            .context("reading the frozen chain the launch republished")?;
        let new_skeleton_bytes = if launched.skeleton {
            Some(std::fs::read(&skeleton_path).with_context(|| {
                format!(
                    "reading republished task skeleton at {}",
                    skeleton_path.display()
                )
            })?)
        } else {
            None
        };
        let acceptance_path =
            task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
        let acceptance_bytes = std::fs::read(&acceptance_path).with_context(|| {
            format!(
                "reading republished acceptance contract at {}",
                acceptance_path.display()
            )
        })?;
        let acceptance_digest =
            archon_workflow::task_set_contract::content_digest(&acceptance_bytes);
        let new_bodies = if launched.skeleton {
            body_contents(task_root, &republished)?
        } else {
            Default::default()
        };
        if let Err(difference) = compare_repaired_chain(
            launched,
            &republished,
            old_skeleton_bytes.as_deref(),
            new_skeleton_bytes.as_deref(),
            &acceptance_digest,
            &old_bodies,
            &new_bodies,
        ) {
            return Err(anyhow!(
                "the per-check repair republished a frozen chain under {} that differs from the one this run was launched with: {difference}",
                task_root.display(),
            ));
        }
        let pending = pending::Pending::of(&detail);
        // R7: a check the host could not prove is the host's: the launch
        // stops resumably with the host-unproven report, as a freeze does
        // (`unproven_incomplete`), and no author is asked to change it.
        if !pending.host.is_empty() {
            let line = format!(
                "frozen_chain_launch acceptance=reauthor_host_pending ids={} detail={}",
                pending.host.keys().cloned().collect::<Vec<_>>().join(","),
                crate::command::workflow_decompose_events::log_field(&pending::Pending::reasons(
                    &pending.host
                ))
            );
            log(&line)?;
            lines.push(line);
            for line in &lines {
                ui.emit(archon_workflow::WorkflowUiEvent::Text(format!("{line}\n")))
                    .await
                    .map_err(|error| anyhow!("reporting {line}: {error}"))?;
            }
            let unproven =
                crate::command::workflow_task_set::executability::HostUnproven(pending.host);
            return Err(crate::command::workflow_task_set::unproven_incomplete(
                anyhow::Error::new(unproven),
            ));
        }
        if !pending.authored.is_empty() {
            let routed = format!(
                "frozen_chain_launch acceptance=reauthor_refuted_routed ids={} detail={}",
                pending
                    .authored
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(","),
                crate::command::workflow_decompose_events::log_field(&pending::Pending::reasons(
                    &pending.authored
                ))
            );
            log(&routed)?;
            lines.push(routed);
            launched.acceptance = false;
            launched.skeleton = false;
            launched.subjects.clear();
            launched.bodies.clear();
            launched.body_digests.clear();
            for line in &lines {
                ui.emit(archon_workflow::WorkflowUiEvent::Text(format!("{line}\n")))
                    .await
                    .map_err(|error| anyhow!("reporting {line}: {error}"))?;
            }
            return Ok(());
        }
    }
    for line in &lines {
        ui.emit(archon_workflow::WorkflowUiEvent::Text(format!("{line}\n")))
            .await
            .map_err(|error| anyhow!("reporting {line}: {error}"))?;
    }
    Ok(())
}
