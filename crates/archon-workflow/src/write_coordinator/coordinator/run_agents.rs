use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::Semaphore;

use super::{FanoutCtx, ItemState};
use crate::request::fanout_item_request;
use crate::runner::WorkflowStageRunner;
use crate::write_coordinator::ItemId;
use crate::write_coordinator::worktree_isolation::detect_canonical_mutation;
use crate::write_coordinator::write_plan::NormalizedPath;

pub(super) type ItemRunBodies = BTreeMap<ItemId, String>;

pub(super) async fn run_wave_agents(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    runner: &dyn WorkflowStageRunner,
    items: &[ItemState<'_>],
    width: usize,
) -> Result<ItemRunBodies, String> {
    let sema = Arc::new(Semaphore::new(width.max(1)));
    let futures = items.iter().map(|it| {
        let sema = sema.clone();
        async move {
            let _permit = sema.acquire().await.expect("semaphore open");
            run_one_agent(ctx, canonical, runner, it).await
        }
    });
    let mut bodies = BTreeMap::new();
    for result in futures_util::future::join_all(futures).await {
        let (item_id, body) = result?;
        bodies.insert(item_id, body);
    }
    Ok(bodies)
}

async fn run_one_agent(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    runner: &dyn WorkflowStageRunner,
    it: &ItemState<'_>,
) -> Result<(ItemId, String), String> {
    let req = build_item_request(ctx, canonical, it);
    let output = runner
        .run_stage(req)
        .await
        .map_err(|err| format!("AgentFailed: {} ({err})", it.plan.item_id))?;
    detect_canonical_mutation(
        canonical,
        &it.baseline,
        &it.plan.target_files,
        &ctx.verify_inputs,
    )
    .map_err(|err| format!("CanonicalMutation: {} ({err})", it.plan.item_id))?;
    Ok((it.plan.item_id.clone(), output.body))
}

fn build_item_request(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    it: &ItemState<'_>,
) -> crate::runner::StageRunRequest {
    let mut req = fanout_item_request(ctx.store, ctx.run, ctx.stage, &it.input.item)
        .unwrap_or_else(|_| panic!("fanout_item_request for {}", it.plan.item_id));
    if !req.input.is_object() {
        req.input = json!({});
    }
    let declared: Vec<String> = it
        .plan
        .target_files
        .iter()
        .map(NormalizedPath::as_str)
        .collect();
    let obj = req.input.as_object_mut().expect("input is object");
    obj.insert(
        "target_repository_root".into(),
        json!(it.plan.isolated_root.to_string_lossy()),
    );
    obj.insert(
        "write_coordination".into(),
        json!({
            "enabled": true,
            "canonical_repository_root": canonical.to_string_lossy(),
            "declared_target_files": declared,
        }),
    );
    req
}
