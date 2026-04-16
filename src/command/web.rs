use archon_core::config::ArchonConfig;
use archon_sdk::web::{WebConfig, WebServer};

pub(crate) async fn handle_web_command(
    port: Option<u16>,
    bind_address: Option<String>,
    no_open: bool,
    config: &ArchonConfig,
) -> anyhow::Result<()> {
    // CLI args override config-file values; config.web provides defaults.
    let effective_port = port.unwrap_or(config.web.port);
    let effective_bind = bind_address.unwrap_or_else(|| config.web.bind_address.clone());
    let effective_open = if no_open { false } else { config.web.open_browser };

    // Bearer token: required for non-localhost to prevent unauthenticated access.
    let is_local = matches!(effective_bind.as_str(), "127.0.0.1" | "::1" | "localhost");
    let token = if is_local {
        None
    } else {
        Some(
            archon_core::remote::auth::load_or_create_token()
                .unwrap_or_else(|_| String::new()),
        )
    };

    let web_cfg = WebConfig {
        port: effective_port,
        bind_address: effective_bind,
        open_browser: effective_open,
    };

    let server = WebServer::new(web_cfg, token);
    if let Err(e) = server.run().await {
        eprintln!("web server error: {e}");
        std::process::exit(1);
    }
    Ok(())
}
