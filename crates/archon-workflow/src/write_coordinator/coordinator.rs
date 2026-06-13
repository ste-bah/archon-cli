//! TASK-WC-007 — Coordinated implementation fanout (PRD-012 §5, §7, §16).
//!
//! The single integration point. Builds per-item WritePlans, schedules
//! non-conflicting waves, runs each item in an isolated worktree (the agent is
//! redirected there via `input["target_repository_root"]`), detects canonical
//! mutation, then applies + verifies validated patches under ONE repo lock.

use std::collections::BTreeMap;
use std::path::Path;

pub use super::WriteBoundaryProbe;
use super::conflict_graph::{WaveCaps, build_schedule};
use super::patch_apply::{
    ApplyRecord, ApplyResumeStatus, VerifyResult, apply_wave, resume_status, run_wave_verify,
    with_repo_lock,
};
use super::patch_manifest::{
    ManifestStatus, PatchManifest, capture_patch, persist_manifest, validate_patch,
};
use super::worktree_isolation::{
    CanonicalBaseline, ItemWorkspace, WorkspaceStatus, capture_canonical_baseline,
    cleanup_workspace, create_item_workspace,
};
use super::write_plan::{
    NormalizedPath, TargetFilesSource, WritePlan, normalize_target, resource_keys_for_targets,
};
use super::{
    ItemId, SerialFallbackReason, WaveId, WriteCoordinatorConfig, WriteCoordinatorRuntime,
};

use crate::fanout::FanoutItem;
use crate::policy::WorkflowPolicy;
use crate::run::WorkflowRun;
use crate::runner::WorkflowStageRunner;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

mod errors;
mod run_agents;
pub use errors::FanoutError;

/// Borrowed execution context for one implementation fanout stage.
pub struct FanoutCtx<'a> {
    pub store: &'a WorkflowStore,
    pub run: &'a WorkflowRun,
    pub policy: &'a WorkflowPolicy,
    pub stage: &'a StageSpec,
    pub run_root: std::path::PathBuf,
    pub item_deps: BTreeMap<ItemId, std::collections::BTreeSet<ItemId>>,
    pub verify_inputs: Vec<NormalizedPath>,
}

/// One fanout item's declared targets + the raw FanoutItem used to build its
/// StageRunRequest.
pub struct PlanInput {
    pub item: FanoutItem,
    pub target_files: Vec<String>,
}

#[derive(Debug)]
pub struct WaveOutcome {
    pub wave_id: WaveId,
    pub items: Vec<ItemId>,
    pub apply_record: Option<ApplyRecord>,
    pub verify: Option<VerifyResult>,
    pub failure: Option<String>,
}

/// One item's plan summary for status/events/learning (TASK-WC-008).
#[derive(Debug, Clone)]
pub struct PlanRecord {
    pub item_id: ItemId,
    pub wave_id: WaveId,
    pub target_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub post_hashes: BTreeMap<String, String>,
    pub patch_bytes_len: usize,
}

#[derive(Debug, Default)]
pub struct CoordinatedOutcome {
    pub run_id: String,
    pub stage_id: String,
    pub waves: Vec<WaveOutcome>,
    pub serial_fallback: Option<SerialFallbackReason>,
    pub item_status: BTreeMap<ItemId, ManifestStatus>,
    pub plans: Vec<PlanRecord>,
}

impl CoordinatedOutcome {
    fn new(run_id: &str, stage_id: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            stage_id: stage_id.to_string(),
            ..Self::default()
        }
    }

    fn fallback(run_id: &str, stage_id: &str, reason: SerialFallbackReason) -> Self {
        Self {
            serial_fallback: Some(reason),
            ..Self::new(run_id, stage_id)
        }
    }
}

/// Per-item working state for a wave.
struct ItemState<'a> {
    plan: &'a WritePlan,
    input: &'a PlanInput,
    baseline: CanonicalBaseline,
    workspace: ItemWorkspace,
}

pub async fn run_coordinated_implementation_fanout(
    ctx: &FanoutCtx<'_>,
    plans_input: Vec<PlanInput>,
    wc_runtime: &WriteCoordinatorRuntime,
    cfg: &WriteCoordinatorConfig,
    runner: &dyn WorkflowStageRunner,
) -> Result<CoordinatedOutcome, FanoutError> {
    let run_id = ctx.run.id.as_str();
    let stage_id = ctx.stage.id.as_str();
    let canonical = match wc_runtime {
        WriteCoordinatorRuntime::Enabled { canonical_root } => canonical_root.clone(),
        WriteCoordinatorRuntime::Disabled { reason } => {
            return Ok(CoordinatedOutcome::fallback(run_id, stage_id, *reason));
        }
    };
    if !runner.supports_workspace_boundary() {
        return Ok(CoordinatedOutcome::fallback(
            run_id,
            stage_id,
            SerialFallbackReason::BoundaryUnavailable,
        ));
    }
    let (plans_input, mut outcome) = filter_resumable_items(ctx, plans_input);
    if plans_input.is_empty() {
        return Ok(outcome);
    }
    let plans: Vec<WritePlan> = plans_input
        .iter()
        .map(|pi| build_write_plan(pi, &canonical, ctx))
        .collect::<Result<_, _>>()?;
    let caps = WaveCaps::from_sources(
        ctx.run.spec.max_parallelism,
        ctx.policy.max_parallelism,
        ctx.stage.max_parallelism,
        runner.max_concurrency(),
        None,
    );
    let schedule = build_schedule(&ctx.stage.id, &plans, &ctx.item_deps, &caps)
        .map_err(FanoutError::Schedule)?;
    let plan_by_id: BTreeMap<&str, &WritePlan> =
        plans.iter().map(|p| (p.item_id.as_str(), p)).collect();
    let input_by_id: BTreeMap<&str, &PlanInput> = plans_input
        .iter()
        .map(|p| (p.item.id.as_str(), p))
        .collect();

    for wave in &schedule.waves {
        process_wave(
            ctx,
            &canonical,
            cfg,
            runner,
            wave,
            &plan_by_id,
            &input_by_id,
            &caps,
            &mut outcome,
        )
        .await?;
    }
    Ok(outcome)
}

fn filter_resumable_items(
    ctx: &FanoutCtx<'_>,
    plans_input: Vec<PlanInput>,
) -> (Vec<PlanInput>, CoordinatedOutcome) {
    let mut outcome = CoordinatedOutcome::new(&ctx.run.id, &ctx.stage.id);
    let mut pending = Vec::new();
    for input in plans_input {
        match resume_status(&input.item.id, &ctx.run_root, &ctx.stage.id) {
            ApplyResumeStatus::Applied => {
                outcome
                    .item_status
                    .insert(input.item.id.clone(), ManifestStatus::Applied);
            }
            ApplyResumeStatus::IdempotentNoop => {
                outcome
                    .item_status
                    .insert(input.item.id.clone(), ManifestStatus::IdempotentNoop);
            }
            ApplyResumeStatus::Conflicted => {
                outcome
                    .item_status
                    .insert(input.item.id.clone(), ManifestStatus::Conflicted);
            }
            ApplyResumeStatus::Failed(_)
            | ApplyResumeStatus::PendingApply
            | ApplyResumeStatus::NotPersisted => pending.push(input),
        }
    }
    (pending, outcome)
}

fn build_write_plan(
    pi: &PlanInput,
    canonical: &Path,
    ctx: &FanoutCtx<'_>,
) -> Result<WritePlan, FanoutError> {
    let target_files: Vec<NormalizedPath> = pi
        .target_files
        .iter()
        .map(|t| normalize_target(t, canonical))
        .collect::<Result<_, _>>()
        .map_err(FanoutError::Plan)?;
    let resource_keys =
        resource_keys_for_targets(&target_files, canonical, &[]).map_err(FanoutError::Plan)?;
    let isolated_root = ctx
        .run_root
        .join("wc")
        .join("worktrees")
        .join(&ctx.stage.id)
        .join(&pi.item.id);
    Ok(WritePlan {
        run_id: ctx.run.id.clone(),
        stage_id: ctx.stage.id.clone(),
        item_id: pi.item.id.clone(),
        canonical_root: canonical.to_path_buf(),
        isolated_root,
        target_files,
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: ctx.verify_inputs.clone(),
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys,
    })
}

#[allow(clippy::too_many_arguments)]
async fn process_wave<'a>(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    cfg: &WriteCoordinatorConfig,
    runner: &dyn WorkflowStageRunner,
    wave: &super::conflict_graph::Wave,
    plan_by_id: &BTreeMap<&str, &'a WritePlan>,
    input_by_id: &BTreeMap<&str, &'a PlanInput>,
    caps: &WaveCaps,
    outcome: &mut CoordinatedOutcome,
) -> Result<(), FanoutError> {
    let mut items = build_wave_items(ctx, canonical, cfg, wave, plan_by_id, input_by_id)?;
    let bodies =
        match run_agents::run_wave_agents(ctx, canonical, runner, &items, caps.effective()).await {
            Ok(bodies) => bodies,
            Err(reason) => {
                finalize_failed_wave(canonical, cfg, wave, &reason, &items, outcome);
                return Ok(());
            }
        };
    let (manifests, pre_by_item, records) =
        capture_and_validate(ctx, cfg, wave.wave_id, &items, &bodies)?;
    let (apply_record, verify) =
        match apply_and_verify(ctx, canonical, wave, &manifests, &pre_by_item) {
            Ok(result) => result,
            Err(err) => {
                finalize_failed_wave(canonical, cfg, wave, &err.to_string(), &items, outcome);
                return Err(err);
            }
        };
    record_applied(&apply_record, outcome);
    outcome.plans.extend(records);
    let status = wave_status(&apply_record);
    cleanup_all(canonical, cfg, &mut items, status);
    outcome.waves.push(WaveOutcome {
        wave_id: wave.wave_id,
        items: wave.items.clone(),
        apply_record: Some(apply_record),
        verify: Some(verify),
        failure: None,
    });
    Ok(())
}

fn build_wave_items<'a>(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    cfg: &WriteCoordinatorConfig,
    wave: &super::conflict_graph::Wave,
    plan_by_id: &BTreeMap<&str, &'a WritePlan>,
    input_by_id: &BTreeMap<&str, &'a PlanInput>,
) -> Result<Vec<ItemState<'a>>, FanoutError> {
    let mut items = Vec::new();
    for id in &wave.items {
        let plan = plan_by_id[id.as_str()];
        let input = input_by_id[id.as_str()];
        let baseline = capture_canonical_baseline(canonical, plan, &ctx.verify_inputs, cfg)
            .map_err(FanoutError::Isolation)?;
        let workspace =
            create_item_workspace(canonical, plan, &baseline).map_err(FanoutError::Isolation)?;
        items.push(ItemState {
            plan,
            input,
            baseline,
            workspace,
        });
    }
    Ok(items)
}

type CaptureResult = (
    Vec<PatchManifest>,
    BTreeMap<ItemId, BTreeMap<String, String>>,
    Vec<PlanRecord>,
);

fn capture_and_validate(
    ctx: &FanoutCtx<'_>,
    cfg: &WriteCoordinatorConfig,
    wave_id: WaveId,
    items: &[ItemState<'_>],
    bodies: &run_agents::ItemRunBodies,
) -> Result<CaptureResult, FanoutError> {
    let mut manifests = Vec::new();
    let mut pre_by_item = BTreeMap::new();
    let mut records = Vec::new();
    for it in items {
        let captured = capture_patch(&it.workspace, &it.plan.target_files, &it.baseline)
            .map_err(FanoutError::Patch)?;
        let body = bodies
            .get(&it.plan.item_id)
            .map(String::as_str)
            .unwrap_or("");
        validate_patch(&captured, it.plan, cfg, body).map_err(FanoutError::Patch)?;
        records.push(PlanRecord {
            item_id: it.plan.item_id.clone(),
            wave_id,
            target_files: it
                .plan
                .target_files
                .iter()
                .map(NormalizedPath::as_str)
                .collect(),
            changed_files: captured.changed_files.clone(),
            post_hashes: captured.post_hashes.clone(),
            patch_bytes_len: captured.patch_bytes.len(),
        });
        let json_path = persist_manifest(
            &ctx.run_root,
            &ctx.run.id,
            &ctx.stage.id,
            &it.plan.item_id,
            &captured,
            ManifestStatus::PendingApply,
        )
        .map_err(FanoutError::Patch)?;
        manifests.push(load_manifest(&json_path)?);
        // Drop created-file (empty-hash) entries: a created file has no baseline
        // to be stale against, and apply-time hash_file() of a not-yet-created
        // canonical path is None — keeping "" would false-positive as stale.
        let pre: BTreeMap<String, String> = captured
            .pre_hashes
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .collect();
        pre_by_item.insert(it.plan.item_id.clone(), pre);
    }
    Ok((manifests, pre_by_item, records))
}

fn load_manifest(json_path: &Path) -> Result<PatchManifest, FanoutError> {
    let text = std::fs::read_to_string(json_path)
        .map_err(|e| FanoutError::Workflow(format!("read manifest: {e}")))?;
    serde_json::from_str(&text).map_err(|e| FanoutError::Workflow(format!("parse manifest: {e}")))
}

fn apply_and_verify(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    wave: &super::conflict_graph::Wave,
    manifests: &[PatchManifest],
    pre_by_item: &BTreeMap<ItemId, BTreeMap<String, String>>,
) -> Result<(ApplyRecord, VerifyResult), FanoutError> {
    with_repo_lock(canonical, || {
        let apply_record = apply_wave(
            canonical,
            manifests,
            pre_by_item,
            wave.wave_id,
            &ctx.run_root,
            &ctx.run.id,
            &ctx.stage.id,
        )?;
        let verify = run_wave_verify(
            canonical,
            ctx.stage.verify_command.as_deref(),
            wave.wave_id,
            &ctx.run_root,
            &ctx.stage.id,
        )?;
        Ok((apply_record, verify))
    })
    .map_err(FanoutError::Apply)
}

fn record_applied(apply_record: &ApplyRecord, outcome: &mut CoordinatedOutcome) {
    for item in &apply_record.items_applied {
        outcome
            .item_status
            .insert(item.clone(), ManifestStatus::Applied);
    }
    for (item, reason) in &apply_record.items_failed {
        outcome.item_status.insert(
            item.clone(),
            ManifestStatus::Failed {
                reason: reason.clone(),
            },
        );
    }
}

fn wave_status(apply_record: &ApplyRecord) -> WorkspaceStatus {
    if apply_record.items_failed.is_empty() {
        WorkspaceStatus::Succeeded
    } else {
        WorkspaceStatus::Failed
    }
}

fn finalize_failed_wave(
    canonical: &Path,
    cfg: &WriteCoordinatorConfig,
    wave: &super::conflict_graph::Wave,
    mutator: &str,
    items: &[ItemState<'_>],
    outcome: &mut CoordinatedOutcome,
) {
    for it in items {
        outcome.item_status.insert(
            it.plan.item_id.clone(),
            ManifestStatus::Failed {
                reason: format!("wave aborted: {mutator}"),
            },
        );
        outcome.plans.push(PlanRecord {
            item_id: it.plan.item_id.clone(),
            wave_id: wave.wave_id,
            target_files: it
                .plan
                .target_files
                .iter()
                .map(NormalizedPath::as_str)
                .collect(),
            changed_files: vec![],
            post_hashes: BTreeMap::new(),
            patch_bytes_len: 0,
        });
        let _ = cleanup_workspace(
            canonical,
            &it.plan.isolated_root,
            WorkspaceStatus::Failed,
            cfg,
        );
    }
    outcome.waves.push(WaveOutcome {
        wave_id: wave.wave_id,
        items: wave.items.clone(),
        apply_record: None,
        verify: None,
        failure: Some(mutator.to_string()),
    });
}

fn cleanup_all(
    canonical: &Path,
    cfg: &WriteCoordinatorConfig,
    items: &mut [ItemState<'_>],
    status: WorkspaceStatus,
) {
    for it in items.iter() {
        let _ = cleanup_workspace(canonical, &it.plan.isolated_root, status, cfg);
    }
}
