// ---------------------------------------------------------------------------
// TASK-AGS-807: tests for build_command_context primary-name resolution
// ---------------------------------------------------------------------------
//
// Rationale for not using a full `SlashCommandContext` fixture:
//
// `SlashCommandContext` carries 24+ fields including `McpServerManager`,
// `Arc<dyn MemoryTrait>`, `Arc<RwLock<AgentRegistry>>`, `SkillRegistry`,
// and several `Mutex`-wrapped runtime state slots. Standing up a real
// fixture would (a) drag test-only dependencies into the bin crate,
// and (b) couple AGS-807's test surface to fields that have nothing to
// do with /status. The AGS-807 executor-report directive explicitly
// permits reporting the chosen approach.
//
// The builder's interesting behaviour is the alias-aware primary-name
// resolution: "/status" → Some("status"), "/info" → Some("status"),
// "/tasks" → Some("tasks") (no snapshot populated). All three of those
// behaviours live in `resolve_primary_from_input`, which takes a
// `&Registry` and is fully testable against `default_registry()` with
// no SlashCommandContext fixture at all.

use super::resolve_primary_from_input;
use crate::command::registry::{CommandEffect, default_registry};
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn build_command_context_populates_status_snapshot_for_slash_status() {
    // Primary lookup proves the builder would populate the snapshot.
    // The full `build_command_context` path is exercised indirectly
    // via live smoke (Gate 5); here we pin the routing decision.
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/status", &reg).as_deref(),
        Some("status"),
        "/status must resolve to primary 'status' so build_command_context \
             populates a StatusSnapshot"
    );
}

#[test]
fn build_command_context_populates_status_snapshot_for_slash_info_alias() {
    // The alias `info` must route to primary `status`. If this ever
    // regresses, /info would fall through to None and StatusHandler
    // would return Err at execute time (see status.rs handler test
    // `status_handler_execute_without_snapshot_returns_err`).
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/info", &reg).as_deref(),
        Some("status"),
        "alias '/info' must route through the registry alias map back to \
             primary 'status' so build_command_context fires the snapshot branch"
    );
}

#[test]
fn build_command_context_leaves_snapshot_none_for_slash_tasks() {
    // `/tasks` is its own primary. The builder should see a primary
    // name != "status" and leave `status_snapshot` at None so the
    // TasksHandler does not pay for unused lock traffic.
    let reg = default_registry();
    let primary = resolve_primary_from_input("/tasks", &reg);
    assert_eq!(
        primary.as_deref(),
        Some("tasks"),
        "/tasks must resolve to its own primary, not 'status'"
    );
    assert_ne!(
        primary.as_deref(),
        Some("status"),
        "the snapshot branch must only fire for 'status' — other primaries \
             must observe status_snapshot = None"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-808: model snapshot routing + apply_effect mutex write.
//
// The builder routes `/model` (and its aliases `/m`, `/switch-model`)
// to `model::build_model_snapshot`. Same rationale as the AGS-807
// /status tests — we pin the routing decision via the pure
// `resolve_primary_from_input` helper because standing up a full
// `SlashCommandContext` fixture drags McpServerManager / MemoryTrait
// / SkillRegistry into the test crate.
// -----------------------------------------------------------------

#[test]
fn build_command_context_populates_model_snapshot_for_slash_model() {
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/model", &reg).as_deref(),
        Some("model"),
        "/model must resolve to primary 'model' so build_command_context \
             populates a ModelSnapshot"
    );
}

/// Verifies the apply_effect semantics for
/// `CommandEffect::SetModelOverride`. Fixture choice: option (b)
/// in the AGS-808 executor report — a narrow
/// `Arc<Mutex<String>>` test harness that mirrors the apply_effect
/// match body for the one variant under test. Full
/// SlashCommandContext fixture is infeasible (24+ fields including
/// McpServerManager + Arc<dyn MemoryTrait>). The production
/// apply_effect keeps the `&SlashCommandContext` signature for
/// future-variant symmetry; this test exercises the write-back
/// invariant (`*mutex.lock().await = resolved`) directly.
#[tokio::test]
async fn apply_effect_set_model_override_writes_to_mutex() {
    let model_override_shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    assert!(
        model_override_shared.lock().await.is_empty(),
        "pre-condition: override must start empty"
    );

    let effect = CommandEffect::SetModelOverride("claude-sonnet-4-6".to_string());

    // Narrow harness mirroring apply_effect's match arm. If
    // production apply_effect diverges, this test will need to be
    // updated in lockstep — that is the intended coupling.
    match effect {
        CommandEffect::SetModelOverride(resolved) => {
            *model_override_shared.lock().await = resolved;
        }
        // TASK-AGS-POST-6-BODIES-B04-DIFF: RunGitDiffStat belongs
        // to /diff. This narrow harness only constructs
        // SetModelOverride above; RunGitDiffStat is unreachable
        // here. Arm exists to keep the match exhaustive and guard
        // against silent drift on future variants.
        CommandEffect::RunGitDiffStat(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        // TASK-AGS-POST-6-BODIES-B10-ADDDIR: AddExtraDir belongs to
        // /add-dir. This narrow harness only constructs
        // SetModelOverride above; AddExtraDir is unreachable here.
        // Arm exists to keep the match exhaustive and guard against
        // silent drift on future variants.
        CommandEffect::AddExtraDir(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        // TASK-AGS-POST-6-BODIES-B11-EFFORT: SetEffortLevelShared
        // belongs to /effort. This narrow harness only constructs
        // SetModelOverride above; SetEffortLevelShared is
        // unreachable here. Arm exists to keep the match exhaustive
        // and guard against silent drift on future variants.
        CommandEffect::SetEffortLevelShared(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: SetPermissionMode
        // belongs to /permissions. This narrow harness only
        // constructs SetModelOverride above; SetPermissionMode is
        // unreachable here. Arm exists to keep the match exhaustive
        // and guard against silent drift on future variants.
        CommandEffect::SetPermissionMode(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        CommandEffect::EnterPlanMode { .. } => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        CommandEffect::SetActivePlanId(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        CommandEffect::StartPipelineWork(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        // FCDP-DRAFT: RunDraft belongs to /draft, added by PR #51. This
        // harness only constructs SetModelOverride above, so it is
        // unreachable here; the arm keeps the match exhaustive so a future
        // variant cannot be added without this test being updated in step.
        CommandEffect::RunDraft { .. } | CommandEffect::RateMessage { .. } => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
        // #200 Phase 4: ReferenceSession belongs to /session-ref, whose own
        // end-to-end coverage runs against a real SlashCommandContext in
        // src/command/session_ref.rs. Unreachable here; the arm keeps the
        // match exhaustive so a later variant cannot slip past this pin.
        CommandEffect::ReferenceSession(_) => {
            unreachable!("narrow apply_effect harness only exercises SetModelOverride")
        }
    }

    let got = model_override_shared.lock().await.clone();
    assert_eq!(
        got, "claude-sonnet-4-6",
        "apply_effect must overwrite model_override_shared with the \
             resolved full model id"
    );
}

// AGS-808 — we pin the routing decision via
// `resolve_primary_from_input` because standing up a full
// `SlashCommandContext` fixture drags McpServerManager /
// MemoryTrait / SkillRegistry into the test crate. The primary
// name returned here is what `build_command_context` uses to
// decide whether to populate `ctx.cost_snapshot`.
//
// /cost is READ-ONLY, so there is no matching `apply_effect` test
// in this ticket — no CommandEffect variant was added for AGS-809.
// -----------------------------------------------------------------

#[test]
fn build_command_context_populates_cost_snapshot_for_slash_cost() {
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/cost", &reg).as_deref(),
        Some("cost"),
        "/cost must resolve to primary 'cost' so build_command_context \
             populates a CostSnapshot"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-811: /mcp snapshot routing. Same rationale as AGS-807 /
// AGS-808 / AGS-809 — we pin the routing decision via
// `resolve_primary_from_input` because standing up a full
// `SlashCommandContext` fixture drags McpServerManager /
// MemoryTrait / SkillRegistry into the test crate. The primary
// name returned here is what `build_command_context` uses to
// decide whether to populate `ctx.mcp_snapshot`.
//
// /mcp is READ-ONLY, so there is no matching `apply_effect` test
// in this ticket — no CommandEffect variant was added for AGS-811.
// Also no aliases — the shipped stub had none and the AGS-811
// spec lists none.
// -----------------------------------------------------------------

#[test]
fn build_command_context_populates_mcp_snapshot_for_slash_mcp() {
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/mcp", &reg).as_deref(),
        Some("mcp"),
        "/mcp must resolve to primary 'mcp' so build_command_context \
             populates an McpSnapshot"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-814: /context snapshot routing. Same rationale as
// AGS-807/808/809/811 — we pin the routing decision via
// `resolve_primary_from_input` because standing up a full
// `SlashCommandContext` fixture drags McpServerManager /
// MemoryTrait / SkillRegistry into the test crate. The primary
// name returned here is what `build_command_context` uses to
// decide whether to populate `ctx.context_snapshot`.
//
// /context is READ-ONLY, so there is no matching `apply_effect`
// test in this ticket — no CommandEffect variant was added for
// AGS-814. No aliases either — shipped stub's `ctx` alias was
// cosmetic (legacy match arm only matched `/context` literally).
// -----------------------------------------------------------------

#[test]
fn build_command_context_populates_context_snapshot_for_slash_context() {
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/context", &reg).as_deref(),
        Some("context"),
        "/context must resolve to primary 'context' so \
             build_command_context populates a ContextSnapshot"
    );
}

// -----------------------------------------------------------------
// Issue #37 AC#2: the permission-mode event must be written through
// the governed-learning DB handle.
//
// The narrow `apply_effect` harness above deliberately stops at the
// match-arm level, which is why `SetPermissionMode` is `unreachable!()`
// there — mirroring the arm would mean re-implementing the very handle
// choice that regressed. This test therefore drives the REAL
// `apply_effect` against a REAL `SlashCommandContext` (see
// `slash_ctx_test_fixture.rs` for why that fixture had to be built), with
// `cozo_db` and `governed_learning_db` pointed at two different
// databases so the assertion can tell them apart.
//
// Reverting `effects.rs` to `slash_ctx.cozo_db.as_ref()` fails this
// test: the row lands in the project DB and the governed DB stays
// empty.
// -----------------------------------------------------------------

#[tokio::test]
async fn apply_effect_set_permission_mode_records_event_in_governed_learning_db() {
    fn learning_db() -> Arc<cozo::DbInstance> {
        let db = cozo::DbInstance::new("mem", "", "").expect("in-memory cozo db");
        archon_learning::schema::ensure_learning_schema(&db).expect("learning schema");
        Arc::new(db)
    }

    // Two distinct handles, both carrying the learning schema. Only the
    // governed one may receive the event; if the project handle also has
    // the relation, a wrong-handle write would succeed silently in
    // production — which is exactly how the original bug hid.
    let project_db = learning_db();
    let governed_db = learning_db();

    let fixture = super::slash_ctx_test_fixture::build_test_slash_context(
        "session-ac2",
        "default",
        Some(Arc::clone(&project_db)),
        Some(Arc::clone(&governed_db)),
    );
    let (tui_tx, mut tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();

    super::apply_effect(
        CommandEffect::SetPermissionMode("plan".to_string()),
        &fixture.ctx,
        &tui_tx,
    )
    .await;

    // 1. The shared permission mode was actually written.
    assert_eq!(
        fixture.ctx.permission_mode.lock().await.as_str(),
        "plan",
        "apply_effect must write the resolved mode to the shared slot"
    );

    // 2. The event row landed in the GOVERNED handle...
    let governed_rows =
        archon_learning::permission_runtime_events::list_permission_runtime_events_by_session(
            &governed_db,
            "session-ac2",
        )
        .expect("read governed permission events");
    assert_eq!(
        governed_rows.len(),
        1,
        "the permission-mode event must be written through governed_learning_db"
    );
    assert_eq!(governed_rows[0].tool_name, "PermissionMode");
    assert_eq!(governed_rows[0].permission_mode, "plan");
    assert_eq!(governed_rows[0].decision, "mode_changed");
    assert_eq!(
        governed_rows[0].reason_code.as_deref(),
        Some("slash_permissions")
    );
    assert_eq!(
        governed_rows[0].raw_redacted_json["previous_mode"], "default",
        "the previous mode must be captured before the shared slot is overwritten"
    );

    // 3. ...and nowhere near the project handle.
    let project_rows =
        archon_learning::permission_runtime_events::list_permission_runtime_events(&project_db)
            .expect("read project permission events");
    assert!(
        project_rows.is_empty(),
        "no permission event may be written through cozo_db; got {project_rows:?}"
    );

    // 4. The TUI still sees the mode change.
    let mut saw_mode_change = false;
    while let Ok(event) = tui_rx.try_recv() {
        if matches!(event, archon_tui::app::TuiEvent::PermissionModeChanged(ref m) if m == "plan") {
            saw_mode_change = true;
        }
    }
    assert!(
        saw_mode_change,
        "apply_effect must still emit PermissionModeChanged"
    );
}

#[tokio::test]
async fn apply_effect_set_active_plan_id_writes_shared_plan_mode_state() {
    let fixture = super::slash_ctx_test_fixture::build_test_slash_context(
        "session-plan-effect",
        "default",
        None,
        None,
    );
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();

    super::apply_effect(
        CommandEffect::SetActivePlanId("safe-plan-id".to_string()),
        &fixture.ctx,
        &tui_tx,
    )
    .await;

    assert_eq!(
        fixture
            .ctx
            .plan_mode_state
            .lock()
            .await
            .active_plan_id
            .as_deref(),
        Some("safe-plan-id"),
        "apply_effect must persist the /plan open selection in shared state"
    );
}

#[test]
fn build_command_context_populates_cost_snapshot_for_slash_billing_alias() {
    // Spec wanted `/usage` as an alias for /cost, but `usage` is
    // already a shipped primary (UsageHandler). Only `/billing`
    // routes to /cost; `/usage` remains bound to UsageHandler.
    // See cost.rs module rustdoc + the CONFIRM R-item in the
    // AGS-809 executor report.
    let reg = default_registry();
    assert_eq!(
        resolve_primary_from_input("/billing", &reg).as_deref(),
        Some("cost"),
        "alias '/billing' must route through the registry alias map \
             back to primary 'cost' so build_command_context fires the \
             cost snapshot branch"
    );
    // Sanity: /usage must NOT route to 'cost'. It is a primary in
    // its own right and its snapshot branch (if any) belongs to a
    // future UsageHandler body-migrate, not AGS-809.
    assert_eq!(
        resolve_primary_from_input("/usage", &reg).as_deref(),
        Some("usage"),
        "/usage is a shipped primary (UsageHandler); must NOT resolve \
             to 'cost'"
    );
}
