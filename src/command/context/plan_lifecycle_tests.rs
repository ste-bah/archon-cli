use std::sync::Arc;

use crate::command::registry::CommandEffect;

#[tokio::test]
async fn slash_entry_records_default_before_entering_plan() {
    fn learning_db() -> Arc<cozo::DbInstance> {
        let db = cozo::DbInstance::new("mem", "", "").expect("in-memory cozo db");
        archon_learning::schema::ensure_learning_schema(&db).expect("learning schema");
        Arc::new(db)
    }

    let governed_db = learning_db();
    let fixture = super::slash_ctx_test_fixture::build_test_slash_context(
        "plan-entry",
        "default",
        None,
        Some(Arc::clone(&governed_db)),
    );
    let (tui_tx, mut tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();

    super::apply_effect(
        CommandEffect::EnterPlanMode {
            previous_mode: archon_permissions::mode::PermissionMode::Default,
        },
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
            .previous_permission_mode,
        Some(archon_permissions::mode::PermissionMode::Default)
    );
    assert_eq!(fixture.ctx.permission_mode.lock().await.as_str(), "plan");

    let governed_rows =
        archon_learning::permission_runtime_events::list_permission_runtime_events_by_session(
            &governed_db,
            "plan-entry",
        )
        .expect("read governed plan-entry event");
    assert_eq!(governed_rows.len(), 1);
    assert_eq!(governed_rows[0].permission_mode, "plan");
    assert_eq!(governed_rows[0].reason_code.as_deref(), Some("slash_plan"));

    assert!(
        matches!(
            tui_rx.try_recv(),
            Ok(archon_tui::app::TuiEvent::PermissionModeChanged(mode)) if mode == "plan"
        ),
        "entering plan mode must emit PermissionModeChanged(\"plan\")"
    );
}

#[tokio::test]
async fn slash_plan_exit_clears_model_entry_before_reentry_and_shared_restore() {
    use archon_core::agent::plan_mode_state::{PlanEntryPath, safe_restore_mode};
    use archon_permissions::mode::PermissionMode;

    let fixture = super::slash_ctx_test_fixture::build_test_slash_context(
        "model-plan-reentry",
        "plan",
        None,
        None,
    );
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    {
        let mut state = fixture.ctx.plan_mode_state.lock().await;
        state.record_entry(PermissionMode::Auto, PlanEntryPath::EnterPlanModeTool);
        state.active_plan_id = Some("stale-model-plan".into());
    }

    super::apply_effect(
        CommandEffect::SetPermissionMode("default".to_string()),
        &fixture.ctx,
        &tui_tx,
    )
    .await;
    {
        let state = fixture.ctx.plan_mode_state.lock().await;
        assert_eq!(state.previous_permission_mode, None);
        assert_eq!(state.active_plan_id, None);
        assert_eq!(state.entered_via, None);
    }

    super::apply_effect(
        CommandEffect::EnterPlanMode {
            previous_mode: PermissionMode::Default,
        },
        &fixture.ctx,
        &tui_tx,
    )
    .await;
    let mut state = fixture.ctx.plan_mode_state.lock().await;
    assert_eq!(
        state.previous_permission_mode,
        Some(PermissionMode::Default)
    );
    assert_eq!(state.entered_via, Some(PlanEntryPath::SlashCommand));
    assert_eq!(
        safe_restore_mode(state.previous_permission_mode.take(), false),
        PermissionMode::Default,
        "structured exit after re-entry must not restore stale Auto authority"
    );
}

#[tokio::test]
async fn slash_plan_exit_clears_entry_before_reentry_and_shared_restore() {
    use archon_core::agent::plan_mode_state::{PlanEntryPath, safe_restore_mode};
    use archon_permissions::mode::PermissionMode;

    let fixture =
        super::slash_ctx_test_fixture::build_test_slash_context("plan-reentry", "auto", None, None);
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();

    super::apply_effect(
        CommandEffect::EnterPlanMode {
            previous_mode: PermissionMode::Auto,
        },
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
            .previous_permission_mode,
        Some(PermissionMode::Auto)
    );

    super::apply_effect(
        CommandEffect::SetPermissionMode("default".to_string()),
        &fixture.ctx,
        &tui_tx,
    )
    .await;
    assert_eq!(fixture.ctx.permission_mode.lock().await.as_str(), "default");
    assert_eq!(
        fixture
            .ctx
            .plan_mode_state
            .lock()
            .await
            .previous_permission_mode,
        None,
        "slash /plan off must consume its lifecycle entry"
    );

    super::apply_effect(
        CommandEffect::EnterPlanMode {
            previous_mode: PermissionMode::Default,
        },
        &fixture.ctx,
        &tui_tx,
    )
    .await;
    let mut state = fixture.ctx.plan_mode_state.lock().await;
    assert_eq!(
        state.previous_permission_mode,
        Some(PermissionMode::Default)
    );
    assert_eq!(state.entered_via, Some(PlanEntryPath::SlashCommand));

    let restore = safe_restore_mode(state.previous_permission_mode.take(), false);
    state.active_plan_id = None;
    state.entered_via = None;
    assert_eq!(
        restore,
        PermissionMode::Default,
        "a later shared ExitPlanMode consume must restore the re-entry mode, never stale Auto"
    );
}
