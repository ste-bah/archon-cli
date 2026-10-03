//! The critic request identity (Issue 259 review): every setting that can
//! change an answer moves it; a secret never does and never appears in it.

use archon_core::config::ArchonConfig;
use archon_llm::provider::LlmRequest;
use archon_workflow::llm_client_port::WorkflowLlmClient;
use std::sync::Arc;

fn client(config: &ArchonConfig) -> Arc<dyn WorkflowLlmClient> {
    let mut config = config.clone();
    // Exercise API-key identity selection without reading ambient credentials.
    let auth =
        archon_llm::auth::AuthProvider::ApiKey(archon_llm::types::Secret::new("test-key".into()));
    let mode =
        archon_llm::identity::resolve_identity_mode(&auth, false, &config.identity.as_view());
    let identity = archon_llm::identity::IdentityProvider::new(
        mode,
        "session".into(),
        "device".into(),
        "account".into(),
    );
    let policy = crate::command::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly;
    let route = crate::command::workflow_provider_route::resolve_anthropic_route(
        config.api.base_url.as_deref(),
        policy,
    );
    let anthropic = archon_llm::anthropic::AnthropicClient::new(auth, identity, route.endpoint);
    if config.llm.openai.api_key.is_none() {
        config.llm.openai.api_key = Some("test-openai-key".into());
    }
    let provider =
        crate::runtime::llm::build_llm_provider_selection(&config.llm, &config.models, anthropic)
            .provider;
    let raw = Arc::new(
        archon_pipeline::llm_adapter::ProviderLlmAdapter::new(provider).with_max_tokens(
            crate::command::pipeline_support::configured_output_ceiling(&config),
        ),
    );
    let inner = Arc::new(
        archon_pipeline::subagent_adapter::SubagentPipelineClient::new(
            raw,
            archon_tools::tool::ToolContext::default(),
        ),
    );
    crate::command::pipeline_workflow_llm::PipelineWorkflowLlmClient::configured_for_route(
        inner, &config, policy,
    )
}

fn identity(config: &ArchonConfig) -> String {
    client(config).request_identity().expect("known envelope")
}

#[test]
fn the_output_ceiling_and_every_endpoint_change_the_identity() {
    let base = ArchonConfig::default();
    let id = identity(&base);
    assert_eq!(id.len(), 64, "a digest, never the settings in clear");
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(id, identity(&base.clone()), "stable for the same settings");

    let mut changed = base.clone();
    changed.api.max_tokens = Some(base.api.thinking_budget + 4096);
    assert_ne!(identity(&changed), id, "max_tokens");

    let mut changed = base.clone();
    changed.api.thinking_budget += 1;
    assert_ne!(
        identity(&changed),
        id,
        "the ceiling an unset max_tokens falls back to"
    );

    let mut changed = base.clone();
    changed.api.base_url = Some("https://gateway.example/v1/messages".into());
    assert_ne!(identity(&changed), id, "the configured endpoint");

    for provider in ["openai", "local"] {
        let mut selected = base.clone();
        selected.llm.provider = provider.into();
        let before = identity(&selected);
        assert_ne!(before, id, "the provider {provider}");
        let mut moved = selected.clone();
        moved.llm.openai.base_url = Some("https://other.example/v1".into());
        moved.llm.local.base_url = "http://127.0.0.2:11434".into();
        assert_ne!(identity(&moved), before, "{provider} endpoint");
    }
}

#[test]
fn a_secret_never_changes_or_enters_the_identity() {
    let mut base = ArchonConfig::default();
    base.llm.provider = "openai".into();
    let id = identity(&base);
    let mut keyed = base.clone();
    keyed.llm.openai.api_key = Some("sk-not-a-real-key".into());
    assert_eq!(identity(&keyed), id);
}

#[test]
fn api_key_clean_and_spoof_prompts_have_distinct_request_identities() {
    let mut clean = ArchonConfig::default();
    clean.identity.mode = "clean".into();
    let mut spoof = clean.clone();
    spoof.identity.mode = "spoof".into();
    assert_ne!(
        identity(&clean),
        identity(&spoof),
        "different wire system prompts must not share verdicts"
    );
}

#[test]
fn rendered_prompt_model_temperature_and_tools_change_the_identity() {
    let client = client(&ArchonConfig::default());
    let base = LlmRequest {
        model: "opus".into(),
        messages: vec![serde_json::json!({"role": "user", "content": "template version one"})],
        extra: serde_json::json!({"temperature": 0.0}),
        ..Default::default()
    };
    let id = client.message_request_identity(&base).unwrap();
    let mut changed = base.clone();
    changed.messages[0]["content"] = serde_json::json!("template version two");
    assert_ne!(
        client.message_request_identity(&changed).unwrap(),
        id,
        "same-HEAD template edit"
    );
    changed = base.clone();
    changed.system = vec![serde_json::json!({"type": "text", "text": "new critic system"})];
    assert_ne!(
        client.message_request_identity(&changed).unwrap(),
        id,
        "system blocks"
    );
    changed = base.clone();
    changed.model = "sonnet".into();
    assert_ne!(
        client.message_request_identity(&changed).unwrap(),
        id,
        "resolved model"
    );
    changed = base.clone();
    changed.extra["temperature"] = serde_json::json!(0.5);
    assert_ne!(
        client.message_request_identity(&changed).unwrap(),
        id,
        "temperature"
    );
    changed = base;
    changed.tools = archon_llm::provider::shared_tools(vec![
        serde_json::json!({"name":"check", "description":"check", "input_schema":{"type":"object"}}),
    ]);
    assert_ne!(
        client.message_request_identity(&changed).unwrap(),
        id,
        "tools"
    );
}

#[test]
fn unknown_or_secret_bearing_routes_have_no_reusable_identity() {
    let mut config = ArchonConfig::default();
    for route in [
        "https://user:secret@example.com/v1/messages",
        "https://example.com/v1/messages?api_key=secret",
    ] {
        config.api.base_url = Some(route.into());
        assert!(client(&config).request_identity().is_none());
    }
}

#[path = "../../build_fingerprint.rs"]
mod build_fingerprint;

#[test]
fn source_fingerprint_changes_for_same_head_parser_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::create_dir(root.join("crates")).unwrap();
    for file in [
        "build.rs",
        "build_fingerprint.rs",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo/config.toml",
    ] {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "unchanged build input").unwrap();
    }
    let parser = root.join("src/parser.rs");
    std::fs::write(&parser, "parser one").unwrap();
    let before = build_fingerprint::source_fingerprint(root);
    assert_eq!(before, build_fingerprint::source_fingerprint(root));
    std::fs::write(&parser, "parser two").unwrap();
    assert_ne!(before, build_fingerprint::source_fingerprint(root));
    std::fs::write(&parser, "parser one").unwrap();
    assert_eq!(
        before,
        build_fingerprint::source_fingerprint(root),
        "restoring source restores identity"
    );
}
