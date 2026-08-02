//! The consumer half of the workflow learning bridge (L3).
//!
//! `archon-workflow` writes one record stream per run and stops there — it
//! depends on exactly one Archon crate and cannot reach `LearningIntegration`.
//! This module is the other half: it reads that stream and dispatches into the
//! learning stack. It lives beside the fold for the same reason the fold does —
//! the binary is the only layer that can see both crates.
//!
//! # `learning_hooks` is the routing selector
//!
//! [`WorkflowLearningRecord::hooks`] carries the spec's `learning_hooks`
//! verbatim. This module decides what each name means; the thin crate does not.
//! **An empty hook list dispatches nothing** — a spec that named no subsystem
//! asked for no learning, and inventing one would make the field decorative
//! again.
//!
//! Not every hook a spec can name has a consumer. `LearningIntegration` exposes
//! SONA trajectories, a ReasoningBank query, and the DESC episode store;
//! `world_model`, `jepa`, `rlm` and `reflexion` have no write-side entry point
//! on it. Those names are counted as unrouted and reported, not silently
//! dropped — a hook that quietly does nothing is how `learning_hooks` became
//! dead in the first place.
//!
//! # Why `on_agent_start` is called too
//!
//! `LearningIntegration::on_agent_complete` is a no-op on its own: it reads
//! `active_trajectories` and `active_agent_contexts`, both of which are
//! populated only by `on_agent_start`. Calling completion alone would dispatch
//! nothing at all. So each record is replayed as a start/complete pair against
//! the same instance, which is what actually produces a SONA feedback signal
//! and a DESC episode.

use std::collections::BTreeSet;
use std::path::Path;

use archon_pipeline::learning::integration::{LearningIntegration, LearningIntegrationConfig};
use archon_workflow::{WorkflowLearningRecord, WorkflowStore};

/// Hook names that name a subsystem reachable through `LearningIntegration`.
///
/// Compared after [`normalize_hook`], so `"reasoning-bank"`, `"ReasoningBank"`
/// and `"reasoning_bank"` are one name.
const INTEGRATION_HOOKS: &[&str] = &["sona", "reasoningbank", "desc"];

/// One replayed agent outcome.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DispatchCall {
    pub agent: String,
    pub phase: String,
    pub task: String,
    pub pipeline_id: String,
    pub quality: f64,
    pub summary: String,
}

/// What a record stream routes to, before anything is dispatched.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DispatchPlan {
    pub calls: Vec<DispatchCall>,
    /// Records whose spec named no hook this module can route.
    pub skipped_unhooked: usize,
    /// Records for a stage that never reached a terminal state.
    pub skipped_incomplete: usize,
    /// Hook names with no consumer on `LearningIntegration`.
    pub unrouted_hooks: BTreeSet<String>,
}

/// What one dispatch did.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LearningDispatchOutcome {
    pub records_read: usize,
    pub dispatched: usize,
    pub skipped_unhooked: usize,
    pub skipped_incomplete: usize,
    pub unrouted_hooks: BTreeSet<String>,
    /// True when records wanted dispatch but no learning stack could be built.
    pub integration_unavailable: bool,
}

/// The whole bridge for one finished run: write the record stream, then route
/// it.
///
/// Both halves live here because the binary is the composition root — it is
/// the only layer that can see a workflow run *and* the learning stack — and
/// because the write must precede the read. Best-effort throughout.
pub(crate) fn bridge_workflow_learning(
    project_root: &Path,
    store: &WorkflowStore,
    run_id: &str,
) -> LearningDispatchOutcome {
    write_run_records(store, run_id);
    fold_workflow_learning(project_root, store, run_id)
}

/// Write the run's learning record stream from its final persisted state.
///
/// This call is what makes [`archon_workflow::WorkflowLearningSink`] run at
/// all: the type had no construction site anywhere in the tree.
fn write_run_records(store: &WorkflowStore, run_id: &str) {
    let run = match store.load_state(run_id) {
        Ok(run) => run,
        Err(error) => {
            tracing::debug!(%error, %run_id, "workflow state unreadable; no learning records");
            return;
        }
    };
    match archon_workflow::WorkflowLearningSink::new(store.clone()).record(&run) {
        Ok(summary) => tracing::debug!(
            %run_id,
            records = summary.records,
            durable = summary.durable_records,
            "workflow learning records written"
        ),
        Err(error) => tracing::debug!(%error, %run_id, "workflow learning records not written"),
    }
}

/// Fold a run's learning records into the learning stack.
///
/// Best-effort throughout: an unreadable stream, an unopenable store, or an
/// absent learning stack all return an outcome rather than an error. Recording
/// an observation must never change what a user's run reports.
pub(crate) fn fold_workflow_learning(
    project_root: &Path,
    store: &WorkflowStore,
    run_id: &str,
) -> LearningDispatchOutcome {
    let records = match archon_workflow::read_learning_records(store, run_id) {
        Ok(records) => records,
        Err(error) => {
            tracing::debug!(%error, %run_id, "workflow learning records unreadable");
            return LearningDispatchOutcome::default();
        }
    };
    let plan = plan_dispatch(&records);
    let mut outcome = LearningDispatchOutcome {
        records_read: records.len(),
        dispatched: 0,
        skipped_unhooked: plan.skipped_unhooked,
        skipped_incomplete: plan.skipped_incomplete,
        unrouted_hooks: plan.unrouted_hooks.clone(),
        integration_unavailable: false,
    };
    if !outcome.unrouted_hooks.is_empty() {
        tracing::debug!(
            hooks = ?outcome.unrouted_hooks,
            %run_id,
            "learning_hooks named subsystems with no consumer on LearningIntegration"
        );
    }
    if plan.calls.is_empty() {
        return outcome;
    }

    let Some(mut learning) = build_fold_learning(project_root) else {
        tracing::debug!(%run_id, "learning stack unavailable; workflow records not dispatched");
        outcome.integration_unavailable = true;
        return outcome;
    };
    outcome.dispatched = apply_dispatch(&plan, &mut learning);
    outcome
}

/// Route a record stream. Pure; no I/O and no learning stack.
pub(crate) fn plan_dispatch(records: &[WorkflowLearningRecord]) -> DispatchPlan {
    let mut plan = DispatchPlan::default();
    for record in records {
        if !record.verification.is_completed() {
            plan.skipped_incomplete += 1;
            continue;
        }
        let mut routed = false;
        for hook in &record.hooks {
            let normalized = normalize_hook(hook);
            if normalized.is_empty() {
                continue;
            }
            if INTEGRATION_HOOKS.contains(&normalized.as_str()) {
                routed = true;
            } else {
                plan.unrouted_hooks.insert(normalized);
            }
        }
        if !routed {
            plan.skipped_unhooked += 1;
            continue;
        }
        // One call per record however many integration-backed hooks the spec
        // named: the subsystems share a single entry point, so dispatching
        // twice would double-count one stage outcome.
        plan.calls.push(dispatch_call(record));
    }
    plan
}

/// Replay every planned call against a learning stack. Returns how many landed.
pub(crate) fn apply_dispatch(plan: &DispatchPlan, learning: &mut LearningIntegration) -> usize {
    for call in &plan.calls {
        learning.on_agent_start(&call.agent, &call.phase, &call.task, &call.pipeline_id);
        learning.on_agent_complete(&call.agent, call.quality, &call.summary);
    }
    plan.calls.len()
}

fn dispatch_call(record: &WorkflowLearningRecord) -> DispatchCall {
    DispatchCall {
        agent: record.agent_key().to_string(),
        phase: if record.phase.trim().is_empty() {
            "workflow".to_string()
        } else {
            record.phase.clone()
        },
        task: format!("{} / {}", record.name, record.stage_id),
        pipeline_id: record.run_id.clone(),
        quality: record.quality(),
        summary: format!(
            "workflow '{}' stage '{}' finished {:?} ({:?}); attempt {}; {} artifact(s){}",
            record.name,
            record.stage_id,
            record.status,
            record.verification,
            record.telemetry.attempt,
            record.telemetry.artifact_count,
            if record.durable { "; durable" } else { "" }
        ),
    }
}

/// Lowercase and strip everything that is not alphanumeric, so hook spellings
/// that differ only in separators or case compare equal.
fn normalize_hook(hook: &str) -> String {
    hook.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

/// Build the learning stack the fold dispatches into.
///
/// SONA trajectories are left off: they need an embedding provider and a GNN
/// input dimension that only the interactive/pipeline construction sites carry,
/// and claiming a trajectory we cannot embed would be worse than not recording
/// one. DESC is the part that closes the loop from here — episodes written now
/// are what `InjectionFilter` feeds back into a later `on_agent_start`.
///
/// `None` when the store will not open or its schema will not initialise.
fn build_fold_learning(project_root: &Path) -> Option<LearningIntegration> {
    let path = project_root.join(".archon").join("learning-state.db");
    let db = super::open_store(&path, "learning")
        .map_err(|error| tracing::debug!(%error, "learning store unavailable for dispatch"))
        .ok()?;
    if let Err(error) = archon_pipeline::learning::schema::initialize_learning_schemas(&db) {
        tracing::debug!(%error, "learning schema init failed; workflow records not dispatched");
        return None;
    }
    let config = LearningIntegrationConfig {
        track_trajectories: false,
        route_prefix: "workflow/".to_string(),
        ..LearningIntegrationConfig::default()
    };
    Some(
        LearningIntegration::new(None, None, config, None).with_desc_store(
            archon_pipeline::learning::desc::DescEpisodeStore::from_arc(db),
        ),
    )
}
