use std::collections::HashSet;

use archon_llm::compaction_policy::{
    COMPACTION_POLICIES, CompactionBackend, ProviderFamily, WireShape, compaction_policy_for_family,
};

#[test]
fn every_supported_provider_family_has_explicit_compaction_policy() {
    let families: HashSet<_> = ProviderFamily::ALL.iter().copied().collect();
    let policy_families: HashSet<_> = COMPACTION_POLICIES
        .iter()
        .map(|policy| policy.provider_family)
        .collect();

    assert_eq!(
        families, policy_families,
        "new provider families must declare an explicit compaction policy"
    );
}

#[test]
fn provider_family_matrix_pins_wire_shape_and_backend() {
    let expected = [
        (
            ProviderFamily::AnthropicApi,
            WireShape::AnthropicMessages,
            CompactionBackend::Anthropic,
            false,
        ),
        (
            ProviderFamily::AnthropicOAuth,
            WireShape::AnthropicMessages,
            CompactionBackend::Anthropic,
            false,
        ),
        (
            ProviderFamily::Bedrock,
            WireShape::BedrockConverse,
            CompactionBackend::Anthropic,
            false,
        ),
        (
            ProviderFamily::Vertex,
            WireShape::VertexAnthropic,
            CompactionBackend::Anthropic,
            false,
        ),
        (
            ProviderFamily::OpenAiNative,
            WireShape::OpenAiChatCompletions,
            CompactionBackend::Generic,
            false,
        ),
        (
            ProviderFamily::OpenAiCompatible,
            WireShape::OpenAiChatCompletions,
            CompactionBackend::Generic,
            false,
        ),
        (
            ProviderFamily::CodexOAuth,
            WireShape::OpenAiResponses,
            CompactionBackend::Generic,
            false,
        ),
        (
            ProviderFamily::CodexAppServer,
            WireShape::CodexAppServerRpc,
            CompactionBackend::Unsupported,
            true,
        ),
        (
            ProviderFamily::Local,
            WireShape::OpenAiChatCompletions,
            CompactionBackend::Generic,
            false,
        ),
    ];

    for (family, wire_shape, backend, generic_fallback) in expected {
        let policy = compaction_policy_for_family(family);
        assert_eq!(policy.wire_shape, wire_shape, "{family:?} wire shape");
        assert_eq!(policy.backend, backend, "{family:?} backend");
        assert_eq!(
            policy.generic_fallback, generic_fallback,
            "{family:?} fallback flag"
        );
    }
}

#[test]
fn provider_id_mapping_is_explicit_for_runtime_provider_names() {
    assert_eq!(
        ProviderFamily::from_provider_id("anthropic"),
        ProviderFamily::AnthropicApi
    );
    assert_eq!(
        ProviderFamily::from_provider_id("bedrock"),
        ProviderFamily::Bedrock
    );
    assert_eq!(
        ProviderFamily::from_provider_id("vertex"),
        ProviderFamily::Vertex
    );
    assert_eq!(
        ProviderFamily::from_provider_id("openai"),
        ProviderFamily::OpenAiNative
    );
    assert_eq!(
        ProviderFamily::from_provider_id("openai-codex"),
        ProviderFamily::CodexOAuth
    );
    assert_eq!(
        ProviderFamily::from_provider_id("local"),
        ProviderFamily::Local
    );
    assert_eq!(
        ProviderFamily::from_provider_id("groq"),
        ProviderFamily::OpenAiCompatible
    );
}
