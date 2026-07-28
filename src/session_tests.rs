use super::*;

#[test]
fn web_shutdown_preempts_buffered_session_input() {
    let source = include_str!("session_loop/loop_input.rs");
    let shutdown_branch = source.find("ctx.shutdown.cancelled()").unwrap();
    let input_branch = source.find("ctx.user_input_rx.recv()").unwrap();

    assert!(
        shutdown_branch < input_branch,
        "biased loop input must observe web shutdown before buffered prompts"
    );
}

#[test]
fn web_session_delegates_process_signals_to_outer_server() {
    let web_source = include_str!("session/web_runtime.rs");
    let interactive_source = include_str!("session/interactive_ui.rs");

    assert!(
        web_source.contains("sandbox_audit_drain.clone(),\n            false,"),
        "web session must not compete with the outer server for process signals"
    );
    assert!(
        interactive_source.contains("sandbox_audit_drain.clone(),\n            true,"),
        "interactive session must retain direct process signal handling"
    );
}

#[test]
fn startup_reports_primary_and_audit_failures_together() {
    let result = finish_startup_failure(
        anyhow::anyhow!("startup failed"),
        Err(anyhow::anyhow!("startup audit drain failed")),
    );
    let message = result.to_string();

    assert!(message.contains("startup failed"), "{result:#}");
    assert!(message.contains("startup audit drain failed"), "{result:#}");
}

#[test]
fn lifecycle_reports_loop_and_audit_failures_together() {
    let result = finish_loop_and_audit(
        Ok(Err(anyhow::anyhow!("loop failed"))),
        Err(anyhow::anyhow!("audit failed")),
    )
    .expect_err("both lifecycle failures must remain visible");
    let message = result.to_string();

    assert!(message.contains("loop failed"), "{result:#}");
    assert!(message.contains("audit failed"), "{result:#}");
}

#[test]
fn sandbox_audit_starts_after_fallible_agent_initialization() {
    let print_source = include_str!("session/build_agent.rs");
    let interactive_source = include_str!("session/interactive_agent.rs");

    assert!(
        print_source.find("resolve_session_provider(").unwrap()
            < print_source.find("audit_sandbox_backend(").unwrap(),
        "print/headless audit writer starts before provider initialization can fail"
    );
    assert!(
        print_source.find("audit_sandbox_backend(").unwrap()
            < print_source
                .find("open_cognitive_store(&working_dir)")
                .unwrap(),
        "print/headless cognitive startup can fail before sandbox configuration is audited"
    );
    assert!(
        print_source.find("audit_sandbox_backend(").unwrap()
            < print_source.find("spawn_metrics_exporter(").unwrap(),
        "print/headless metrics startup can fail before sandbox configuration is audited"
    );
    assert!(
        interactive_source.find("resolve_provider(").unwrap()
            < interactive_source.find("audit_sandbox_backend(").unwrap(),
        "interactive/web audit writer starts before provider initialization can fail"
    );
    assert!(
        interactive_source.find("audit_sandbox_backend(").unwrap()
            < interactive_source
                .find("open_cognitive_store(&working_dir)")
                .unwrap(),
        "interactive/web cognitive startup can fail before sandbox configuration is audited"
    );
    assert!(
        interactive_source.find("audit_sandbox_backend(").unwrap()
            < interactive_source.find("spawn_metrics_exporter(").unwrap(),
        "interactive/web metrics startup can fail before sandbox configuration is audited"
    );
}

#[test]
fn active_session_model_uses_configured_codex_default_when_claude_default_would_leak() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.llm.provider = "openai-codex".into();
    config.api.default_model = "claude-sonnet-4-6".into();
    config.models.openai_codex.default = "gpt-codex-default".into();

    assert_eq!(active_session_model(&config), "gpt-codex-default");
}

#[test]
fn active_session_model_uses_configured_codex_mini_for_haiku_default() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.llm.provider = "openai-codex".into();
    config.api.default_model = "claude-haiku-4-5-20251001".into();
    config.models.openai_codex.mini = "gpt-codex-mini".into();

    assert_eq!(active_session_model(&config), "gpt-codex-mini");
}

#[test]
fn active_session_model_preserves_explicit_codex_model_override() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.llm.provider = "openai-codex".into();
    config.api.default_model = "gpt-5.4-codex-test".into();

    assert_eq!(active_session_model(&config), "gpt-5.4-codex-test");
}

#[test]
fn active_session_model_preserves_anthropic_default() {
    let _env_lock = super::anthropic_model_env_lock()
        .lock()
        .expect("Anthropic model environment lock");
    let previous = std::env::var_os("ANTHROPIC_MODEL");
    unsafe {
        std::env::remove_var("ANTHROPIC_MODEL");
    }
    let config = archon_core::config::ArchonConfig::default();
    let model = active_session_model(&config);
    unsafe {
        match previous {
            Some(value) => std::env::set_var("ANTHROPIC_MODEL", value),
            None => std::env::remove_var("ANTHROPIC_MODEL"),
        }
    }

    assert_eq!(model, config.api.default_model);
}
