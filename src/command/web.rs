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
    allow_unauthenticated_nonlocal_bind: bool,
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
    let token = if is_local || allow_unauthenticated_nonlocal_bind {
        None
    } else {
        Some(
            archon_core::remote::auth::load_or_create_token()
                .map_err(|e| anyhow::anyhow!("failed to load web auth token: {e}"))?,
        )
    };

    let web_cfg = WebConfig {
        port: effective_port,
        bind_address: effective_bind,
        open_browser: effective_open,
        max_body_bytes: config.web.max_body_bytes,
    };

    let policy = web_policy_summary();
    let shutdown_signal = register_web_shutdown_signal()?;
    let paths = WebRuntimePaths::from_overrides(
        config.memory.db_path.as_deref(),
        config.session.db_path.as_deref(),
    );
    // The chat tab is the ONLY surface here that needs a provider. Memory,
    // corpus, ingest, learning, cognitive, world model, JEPA, pipelines,
    // workflows, metrics, settings, evidence and the task board are all read
    // from local files and databases.
    //
    // So a credential that will not refresh must not take the workbench down
    // with it (#147): thirteen working surfaces are not worth forfeiting to
    // protect one, and a stale token is the common case, not an exotic one.
    // `WebServer` already serves a chat-less dashboard — attached `/web` runs
    // exactly that way and reports `features.chat: false`, which the frontend
    // already handles by hiding the tab.
    let chat_backend =
        match crate::command::web_chat::WebChatBridge::new(config, cli, env_vars, resolved_flags)
            .await
        {
            Ok(bridge) => Some(Arc::new(bridge)),
            Err(error) => {
                tracing::warn!(
                    error = format!("{error:#}"),
                    "chat backend unavailable; serving the workbench without the chat tab"
                );
                eprintln!(
                    "Chat is unavailable ({error:#}).\n\
                 Every other tab still works; run `archon auth login` to restore chat."
                );
                None
            }
        };
    let mut server = WebServer::with_policy_and_paths(web_cfg, token, policy, paths);
    if let Some(backend) = chat_backend.clone() {
        server = server.with_chat_backend(backend);
    }
    if allow_unauthenticated_nonlocal_bind {
        server = server.unsafe_allow_unauthenticated_nonlocal_bind_for_cli();
    }
    let (shutdown_error_tx, shutdown_error_rx) = tokio::sync::oneshot::channel();
    let chat_backend_for_shutdown = chat_backend.clone();
    let server_result = server
        .run_until(async move {
            let result = shutdown_signal.await;
            if let Some(backend) = chat_backend_for_shutdown {
                backend.begin_shutdown().await;
            }
            let _ = shutdown_error_tx.send(result);
        })
        .await;
    let audit_result = match chat_backend {
        Some(backend) => {
            backend.begin_shutdown().await;
            backend.finish_shutdown().await
        }
        // Nothing was started, so there is no audit to drain and nothing to
        // report as a shutdown failure.
        None => Ok(()),
    };
    let signal_result = shutdown_error_rx
        .await
        .map_err(|_| anyhow::anyhow!("web shutdown signal task ended without a result"))
        .and_then(|result| result);

    finish_web_shutdown(server_result, signal_result, audit_result)
}

fn finish_web_shutdown(
    server_result: anyhow::Result<()>,
    signal_result: anyhow::Result<()>,
    audit_result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    let errors: Vec<String> = [
        server_result
            .err()
            .map(|error| format!("web server failed: {error:#}")),
        signal_result
            .err()
            .map(|error| format!("web shutdown signal failed: {error:#}")),
        audit_result
            .err()
            .map(|error| format!("web audit shutdown failed: {error:#}")),
    ]
    .into_iter()
    .flatten()
    .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(errors.join("; ")))
    }
}

fn register_web_shutdown_signal()
-> anyhow::Result<std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>> {
    #[cfg(unix)]
    {
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .map_err(|error| anyhow::anyhow!("web: SIGINT handler failed: {error}"))?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|error| anyhow::anyhow!("web: SIGTERM handler failed: {error}"))?;
        Ok(Box::pin(async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
            Ok(())
        }))
    }
    #[cfg(not(unix))]
    {
        let mut interrupt = tokio::signal::windows::ctrl_c()
            .map_err(|error| anyhow::anyhow!("web: Ctrl-C handler failed: {error}"))?;
        Ok(Box::pin(async move {
            interrupt.recv().await;
            Ok(())
        }))
    }
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
            allow_web_terminal: policy.web.allow_web_terminal,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_signal_is_registered_before_web_session_starts() {
        let source = include_str!("web.rs");
        let registration = source
            .find("let shutdown_signal = register_web_shutdown_signal()")
            .expect("web shutdown signal registration");
        let session_start = source
            .find("WebChatBridge::new")
            .expect("web session startup");

        assert!(
            registration < session_start,
            "web session can consume SIGTERM before server listener is registered"
        );
    }

    #[test]
    fn web_shutdown_succeeds_when_every_stage_succeeds() {
        finish_web_shutdown(Ok(()), Ok(()), Ok(())).expect("clean web shutdown");
    }

    #[test]
    fn web_shutdown_preserves_each_individual_failure() {
        for (server_result, signal_result, audit_result, expected) in [
            (
                Err(anyhow::anyhow!("server failed")),
                Ok(()),
                Ok(()),
                "server failed",
            ),
            (
                Ok(()),
                Err(anyhow::anyhow!("signal failed")),
                Ok(()),
                "signal failed",
            ),
            (
                Ok(()),
                Ok(()),
                Err(anyhow::anyhow!("audit failed")),
                "audit failed",
            ),
        ] {
            let error = finish_web_shutdown(server_result, signal_result, audit_result)
                .expect_err("web shutdown failure must remain visible");

            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[test]
    fn web_shutdown_preserves_server_signal_and_audit_failures() {
        let error = finish_web_shutdown(
            Err(anyhow::anyhow!("server failed")),
            Err(anyhow::anyhow!("signal failed")),
            Err(anyhow::anyhow!("audit failed")),
        )
        .expect_err("all web shutdown failures must remain visible");
        let message = error.to_string();

        assert!(message.contains("server failed"), "{error:#}");
        assert!(message.contains("signal failed"), "{error:#}");
        assert!(message.contains("audit failed"), "{error:#}");
    }
}
