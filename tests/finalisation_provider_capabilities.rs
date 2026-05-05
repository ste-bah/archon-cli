use archon_cli_workspace::cli_args::{Cli, Commands, ProvidersAction};
use archon_llm::providers::{
    ProviderCapability, capabilities_for, render_capability_markdown, supports,
};
use clap::Parser;

#[test]
fn generated_provider_capabilities_doc_matches_code() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/generated/provider-capabilities.md");
    let doc = std::fs::read_to_string(&path).expect("generated provider capabilities doc exists");
    assert_eq!(
        doc,
        render_capability_markdown(),
        "docs/generated/provider-capabilities.md must stay generated from archon_llm::providers::capabilities"
    );
}

#[test]
fn provider_capability_matrix_documents_codex_tui_but_not_pipelines() {
    let codex = capabilities_for("openai-codex").expect("openai-codex capability row");
    assert!(codex.supports(ProviderCapability::OneShotChat));
    assert!(codex.supports(ProviderCapability::InteractiveSession));
    assert!(codex.supports(ProviderCapability::Streaming));
    assert!(!codex.supports(ProviderCapability::Subagents));
    assert!(!codex.supports(ProviderCapability::PipelineCoding));
    assert!(!codex.supports(ProviderCapability::PipelineResearch));
    assert!(!codex.supports(ProviderCapability::PipelineGametheory));
    assert!(!codex.supports(ProviderCapability::BtwSideQuestion));
}

#[test]
fn provider_capability_matrix_documents_anthropic_agentic_surfaces() {
    for provider_id in ["anthropic-oauth", "anthropic-api-key"] {
        assert!(supports(provider_id, ProviderCapability::ToolUse));
        assert!(supports(provider_id, ProviderCapability::Subagents));
        assert!(supports(provider_id, ProviderCapability::PipelineCoding));
        assert!(supports(provider_id, ProviderCapability::PipelineResearch));
        assert!(supports(
            provider_id,
            ProviderCapability::PipelineGametheory
        ));
    }
}

#[test]
fn cli_parses_providers_capabilities_subcommand() {
    let cli = Cli::parse_from(["archon", "providers", "capabilities"]);
    match cli.command {
        Some(Commands::Providers {
            action: Some(ProvidersAction::Capabilities),
        }) => {}
        other => panic!("expected providers capabilities command, got {other:?}"),
    }
}

#[test]
fn cli_parses_providers_doctor_subcommand() {
    let cli = Cli::parse_from(["archon", "providers", "doctor"]);
    match cli.command {
        Some(Commands::Providers {
            action: Some(ProvidersAction::Doctor),
        }) => {}
        other => panic!("expected providers doctor command, got {other:?}"),
    }
}
