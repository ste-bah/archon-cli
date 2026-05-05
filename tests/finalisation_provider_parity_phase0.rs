use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use archon_llm::providers::{ProviderCapability, capabilities_for};

const PHASE0_AUDIT_DOC: &str = "docs/development/provider-parity-phase0-audit.md";

#[test]
fn codex_agentic_capabilities_remain_disabled_except_verified_btw() {
    let codex = capabilities_for("openai-codex").expect("openai-codex capability row");

    assert!(codex.supports(ProviderCapability::OneShotChat));
    assert!(codex.supports(ProviderCapability::InteractiveSession));
    assert!(codex.supports(ProviderCapability::Streaming));
    assert!(codex.supports(ProviderCapability::BtwSideQuestion));
    assert!(codex.supports(ProviderCapability::Vision));

    for capability in [
        ProviderCapability::ToolUse,
        ProviderCapability::Subagents,
        ProviderCapability::PipelineCoding,
        ProviderCapability::PipelineResearch,
        ProviderCapability::PipelineGametheory,
        ProviderCapability::CostMetadata,
    ] {
        assert!(
            !codex.supports(capability),
            "Codex must not claim {} until PRD-FINALISATION-002 adapter tests prove parity",
            capability.label()
        );
    }
}

#[test]
fn phase0_direct_anthropic_construction_baseline_is_explicit() {
    let expected = BTreeMap::from([
        ("crates/archon-sdk/src/query.rs", 1usize),
        ("src/command/chat.rs", 1),
        ("src/command/gametheory.rs", 1),
        ("src/command/pipeline.rs", 3),
        ("src/command/team.rs", 1),
        ("src/runtime/llm.rs", 1),
        ("src/session.rs", 3),
        ("src/session_loop/slash_handlers.rs", 1),
    ]);

    let actual = direct_anthropic_construction_counts();
    assert_eq!(
        actual, expected,
        "Phase 0 direct Anthropic construction baseline changed. If this is intentional, update \
         {PHASE0_AUDIT_DOC} and explain whether the site was removed, moved into an approved \
         provider factory, or newly introduced."
    );
}

#[test]
fn phase0_audit_doc_mentions_every_direct_anthropic_site() {
    let root = repo_root();
    let doc = fs::read_to_string(root.join(PHASE0_AUDIT_DOC)).expect("phase 0 audit doc exists");

    for path in direct_anthropic_construction_counts().keys() {
        assert!(
            doc.contains(path),
            "{PHASE0_AUDIT_DOC} does not mention direct Anthropic construction site {path}"
        );
    }
}

#[test]
fn phase0_audit_doc_records_codex_continuation_primitives() {
    let root = repo_root();
    let doc = fs::read_to_string(root.join(PHASE0_AUDIT_DOC)).expect("phase 0 audit doc exists");

    for required in [
        "tools_to_responses_tools()",
        "StreamAccumulator",
        "ResponseInputItem::FunctionCallOutput",
        "tool result continuation",
    ] {
        assert!(
            doc.contains(required),
            "{PHASE0_AUDIT_DOC} should document Codex primitive `{required}`"
        );
    }
}

fn direct_anthropic_construction_counts() -> BTreeMap<&'static str, usize> {
    let root = repo_root();
    let paths = [
        "crates/archon-sdk/src/query.rs",
        "src/command/chat.rs",
        "src/command/gametheory.rs",
        "src/command/pipeline.rs",
        "src/command/team.rs",
        "src/runtime/llm.rs",
        "src/session.rs",
        "src/session_loop/slash_handlers.rs",
    ];

    paths
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(root.join(path))
                .unwrap_or_else(|err| panic!("failed to read {path}: {err}"));
            (path, text.matches("AnthropicClient::new").count())
        })
        .filter(|(_, count)| *count > 0)
        .collect()
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
