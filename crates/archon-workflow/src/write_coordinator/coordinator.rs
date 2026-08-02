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
use super::patch_apply::{ApplyRecord, VerifyResult, apply_wave, run_wave_verify, with_repo_lock};
use super::patch_manifest::{ManifestStatus, PatchManifest, persist_manifest, validate_patch};
use super::shared_append::{
    resolve_shared_append_targets, resource_keys_for_targets_with_shared_append,
};
use super::worktree_isolation::{
    CanonicalBaseline, ItemWorkspace, WorkspaceStatus, capture_canonical_baseline,
    create_item_workspace,
};
use super::write_plan::{NormalizedPath, TargetFilesSource, WritePlan, normalize_target};
use super::{
    ItemId, SerialFallbackReason, WaveId, WriteCoordinatorConfig, WriteCoordinatorRuntime,
};

use crate::fanout::FanoutItem;
use crate::persistence;
use crate::policy::WorkflowPolicy;
use crate::run::WorkflowRun;
use crate::runner::WorkflowStageRunner;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;
use crate::work_unit_coverage;

mod errors;
mod resume;
mod run_agents;
mod target_adoption;
mod validation_failure;
mod wave_failure;
pub use errors::FanoutError;
use resume::filter_resumable_items;

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
    pub work_unit_ids: Vec<String>,
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
        let keep_going = process_wave(
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
        if !keep_going {
            break;
        }
    }
    Ok(outcome)
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
    // Empty unless the item's payload names paths under
    // `shared_append_target_files`, so every target stays exclusive by default
    // and concurrency is opted into, never inherited.
    let shared_append: Vec<NormalizedPath> = resolve_shared_append_targets(&pi.item.payload)
        .map_err(FanoutError::Plan)?
        .iter()
        .map(|t| normalize_target(t, canonical))
        .collect::<Result<_, _>>()
        .map_err(FanoutError::Plan)?;
    let resource_keys =
        resource_keys_for_targets_with_shared_append(&target_files, canonical, &[], &shared_append)
            .map_err(FanoutError::Plan)?;
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
        target_dir_scopes: Vec::new(),
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
) -> Result<bool, FanoutError> {
    let mut items = build_wave_items(ctx, canonical, cfg, wave, plan_by_id, input_by_id)?;
    let bodies =
        match run_agents::run_wave_agents(ctx, canonical, runner, &items, caps.effective()).await {
            Ok(bodies) => bodies,
            Err(reason) => {
                wave_failure::finalize_failed_wave(canonical, cfg, wave, &reason, &items, outcome);
                return Ok(false);
            }
        };
    let (manifests, pre_by_item, records) =
        match capture_and_validate(ctx, cfg, wave.wave_id, &items, &bodies) {
            Ok(result) => result,
            Err(err) => {
                wave_failure::finalize_failed_wave(
                    canonical,
                    cfg,
                    wave,
                    &err.to_string(),
                    &items,
                    outcome,
                );
                return Ok(false);
            }
        };
    let (apply_record, verify) =
        match apply_and_verify(ctx, canonical, wave, &manifests, &pre_by_item) {
            Ok(result) => result,
            Err(err) => {
                wave_failure::finalize_failed_wave(
                    canonical,
                    cfg,
                    wave,
                    &err.to_string(),
                    &items,
                    outcome,
                );
                return Ok(false);
            }
        };
    for manifest in manifests
        .iter()
        .filter(|m| matches!(m.status, ManifestStatus::IdempotentNoop))
    {
        outcome
            .item_status
            .insert(manifest.item_id.clone(), ManifestStatus::IdempotentNoop);
    }
    record_applied(&apply_record, outcome);
    outcome.plans.extend(records);
    let status = wave_status(&apply_record);
    wave_failure::cleanup_all(canonical, cfg, &mut items, status);
    outcome.waves.push(WaveOutcome {
        wave_id: wave.wave_id,
        items: wave.items.clone(),
        apply_record: Some(apply_record),
        verify: Some(verify),
        failure: None,
    });
    Ok(true)
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
    outputs: &run_agents::ItemRunOutputs,
) -> Result<CaptureResult, FanoutError> {
    let mut manifests = Vec::new();
    let mut pre_by_item = BTreeMap::new();
    let mut records = Vec::new();
    for it in items {
        let output = outputs.get(&it.plan.item_id).ok_or_else(|| {
            FanoutError::Workflow(format!("missing output for {}", it.plan.item_id))
        })?;
        let (captured, active_plan) =
            match target_adoption::capture_with_target_adoption(cfg, items.len(), it) {
                Ok(capture) => capture,
                Err(err) => {
                    let reason = err.to_string();
                    validation_failure::persist_capture_error(ctx, it, output, &reason);
                    return Err(err);
                }
            };
        if let Err(err) = validate_patch(&captured, &active_plan, cfg, &output.body) {
            let reason = err.to_string();
            validation_failure::persist(ctx, it, &captured, output, &reason);
            return Err(FanoutError::Patch(err));
        }
        persistence::record_captured_agent_output(
            ctx.store,
            &ctx.run.id,
            &ctx.stage.id,
            &it.plan.item_id,
            output,
        )
        .map_err(|err| FanoutError::Workflow(format!("record captured output: {err}")))?;
        let manifest_status = if captured.patch_bytes.is_empty() {
            ManifestStatus::IdempotentNoop
        } else {
            ManifestStatus::PendingApply
        };
        records.push(PlanRecord {
            item_id: it.plan.item_id.clone(),
            wave_id,
            work_unit_ids: work_unit_coverage::item_required_units(&it.input.item.payload),
            target_files: active_plan
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
            manifest_status,
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
    match apply_record.items_failed.is_empty() {
        true => WorkspaceStatus::Succeeded,
        false => WorkspaceStatus::Failed,
    }
}
