use super::*;
use crate::subagent::runner::tests::{MockProvider, make_runner};

#[tokio::test]
async fn the_snapshot_freezes_every_runner_limit_and_shared_config_value() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut runner = make_runner(provider, 7);
    let mut config = (*runner.agent_config).clone();
    config.subagent_stream_idle_timeout_secs = 43;
    config.max_tool_concurrency = 2;
    config.max_tokens = 1024;
    runner.agent_config = Arc::new(config);
    runner.timeout_secs = 91;
    runner.model = "resolved-model".into();
    runner.set_request_system(vec![
        serde_json::json!({"type":"text","text":"original system"}),
    ]);
    runner.set_critical_system_reminder("original reminder".into());
    let parent_config = runner.agent_config.clone();
    let request = archon_tools::subagent_request::SubagentRequest {
        prompt: "task".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 7,
        timeout_secs: 91,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        read_roots: vec![],
        write_roots: vec![],
        provider_env: None,
    };
    let context =
        EffectiveRunContext::capture(&mut runner, request, IsolationTier::Shared, None, None).await;
    parent_config
        .fast_mode
        .store(true, std::sync::atomic::Ordering::Relaxed);
    *parent_config.permission_mode.lock().await = "bypassPermissions".into();
    parent_config
        .extra_dirs
        .lock()
        .await
        .push(std::env::temp_dir());
    *parent_config.model_override.lock().await = "replacement".into();
    let resumed = context
        .runner("child", &tokio_util::sync::CancellationToken::new(), None)
        .unwrap();
    assert!(
        !resumed
            .agent_config
            .fast_mode
            .load(std::sync::atomic::Ordering::Relaxed),
        "shared parent settings changed the stored effective context"
    );
    assert_ne!(
        *resumed.agent_config.permission_mode.lock().await,
        "bypassPermissions"
    );
    assert!(resumed.agent_config.extra_dirs.lock().await.is_empty());
    assert!(resumed.agent_config.model_override.lock().await.is_empty());
    assert_eq!(resumed.max_turns, 7);
    assert_eq!(resumed.timeout_secs, 91);
    assert_eq!(resumed.model, "resolved-model");
    assert_eq!(resumed.agent_config.subagent_stream_idle_timeout_secs, 43);
    assert_eq!(resumed.agent_config.max_tool_concurrency, 2);
    assert_eq!(resumed.agent_config.max_tokens, 1024);
    assert_eq!(
        resumed.agent_config.filesystem,
        runner.agent_config.filesystem
    );
    assert_eq!(resumed.effort, runner.effort);
    assert_eq!(
        resumed.critical_system_reminder,
        runner.critical_system_reminder
    );
    assert!(Arc::ptr_eq(&resumed.request_system, &runner.request_system));
    assert!(Arc::ptr_eq(
        &resumed.tool_definitions,
        &runner.tool_definitions
    ));
    assert!(Arc::ptr_eq(&resumed.registry, &runner.registry));
    assert!(Arc::ptr_eq(&resumed.provider, &runner.provider));
    assert!(Arc::ptr_eq(&resumed.identity, &runner.identity));
    assert!(resumed.subagent_manager.is_none());
    assert!(resumed.initial_messages.is_none());
    assert!(resumed.completed_history.is_none());
    assert!(resumed.progress.is_none());
}

#[tokio::test]
async fn stopping_one_execution_does_not_cancel_its_resume_scope() {
    let mut runner = make_runner(Arc::new(MockProvider::new(vec![])), 7);
    let parent = tokio_util::sync::CancellationToken::new();
    let stopped_execution = parent.child_token();
    runner.tool_context.cancel_parent = Some(stopped_execution.clone());
    let request = archon_tools::subagent_request::SubagentRequest {
        prompt: "task".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 7,
        timeout_secs: 91,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        read_roots: vec![],
        write_roots: vec![],
        provider_env: None,
    };
    let context = EffectiveRunContext::capture(
        &mut runner,
        request,
        IsolationTier::Shared,
        None,
        Some(parent.clone()),
    )
    .await;
    stopped_execution.cancel();
    assert!(!parent.is_cancelled());
    context
        .runner("child", &tokio_util::sync::CancellationToken::new(), None)
        .expect("stopping an execution must not revoke the original parent scope");
    parent.cancel();
    assert!(
        context
            .runner("child", &tokio_util::sync::CancellationToken::new(), None)
            .is_err(),
        "the original parent scope must still constrain resume"
    );
}
