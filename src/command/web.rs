use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_sdk::web::{
    WebConfig, WebPolicySummary, WebRuntimePaths, WebServer, WebSubsystemPolicySummary,
    api::EffectivePolicySummary,
};
use std::sync::Arc;

use crate::cli_args::Cli;

pub(crate) async fn handle_web_command(
    port: Option<u16>,
    bind_address: Option<String>,
    no_open: bool,
    config: &ArchonConfig,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> anyhow::Result<()> {
    // CLI args override config-file values; config.web provides defaults.
    let effective_port = port.unwrap_or(config.web.port);
    let effective_bind = bind_address.unwrap_or_else(|| config.web.bind_address.clone());
    let effective_open = if no_open {
        false
    } else {
        config.web.open_browser
    };

    // Bearer token: required for non-localhost to prevent unauthenticated access.
    let is_local = matches!(effective_bind.as_str(), "127.0.0.1" | "::1" | "localhost");
    let token = if is_local {
        None
    } else {
        Some(archon_core::remote::auth::load_or_create_token().unwrap_or_else(|_| String::new()))
    };

    let web_cfg = WebConfig {
        port: effective_port,
        bind_address: effective_bind,
        open_browser: effective_open,
    };

    let policy = web_policy_summary();
    let paths = WebRuntimePaths::from_overrides(
        config.memory.db_path.as_deref(),
        config.session.db_path.as_deref(),
    );
    let chat_backend =
        crate::command::web_chat::WebChatBridge::new(config, cli, env_vars, resolved_flags).await?;
    let server = WebServer::with_policy_and_paths(web_cfg, token, policy, paths)
        .with_chat_backend(Arc::new(chat_backend));
    if let Err(e) = server.run().await {
        eprintln!("web server error: {e}");
        std::process::exit(1);
    }
    Ok(())
}

fn web_policy_summary() -> EffectivePolicySummary {
    let policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();
    EffectivePolicySummary {
        web: WebPolicySummary {
            allow_mutating_actions: policy.web.allow_mutating_actions,
            allow_file_uploads: policy.web.allow_file_uploads,
            allow_pipeline_controls: policy.web.allow_pipeline_controls,
            allow_model_training_actions: policy.web.allow_model_training_actions,
            allow_corpus_open_paths: policy.web.allow_corpus_open_paths,
        },
        subsystem: WebSubsystemPolicySummary {
            allow_behavior_proposal_actions: true,
            allow_model_behavior_changes: policy.world_model.allow_behavior_changes,
            allow_pipeline_controls: policy.web.allow_pipeline_controls,
            allow_corpus_open_paths: policy.web.allow_corpus_open_paths,
            allow_file_uploads: policy.web.allow_file_uploads,
        },
        action_gate: "global web mutation gate AND action-family gate AND subsystem gate"
            .to_string(),
        requires_confirmation: vec![
            "pipeline control".to_string(),
            "model promotion".to_string(),
            "training action".to_string(),
            "corpus filesystem open".to_string(),
            "behaviour proposal approval".to_string(),
        ],
    }
}
