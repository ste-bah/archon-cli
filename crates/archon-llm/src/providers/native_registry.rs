//! TASK-AGS-704: NATIVE_REGISTRY — descriptor metadata for the 9 native
//! providers (openai, anthropic, gemini, xai, bedrock, azure, cohere,
//! copilot, minimax). Mirror of TASK-AGS-702's OPENAI_COMPAT_REGISTRY but
//! for Native compat_kind. Pairs with native_gap.rs for the 4 stub impls.
//!
//! Spec deviation (inherited from TASK-AGS-703 greenlit 2026-04-13): the
//! stubs in native_gap.rs surface "Open Question #3" via LlmError::Unsupported
//! rather than ProviderError::InvalidResponse. The sentinel string is
//! preserved — grep 'Open Question #3' still finds all gap-fillers.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use url::Url;

use super::descriptor::{AuthFlavor, CompatKind, ProviderDescriptor};
use super::features::ProviderFeatures;
use super::quirks::ProviderQuirks;

fn parse_url(s: &str) -> Url {
    Url::parse(s).expect("TASK-AGS-704: hardcoded native base_url must parse")
}

pub static NATIVE_REGISTRY: Lazy<HashMap<&'static str, ProviderDescriptor>> = Lazy::new(|| {
    let mut m: HashMap<&'static str, ProviderDescriptor> = HashMap::new();

    // 1. openai (existing native impl in providers/openai.rs)
    m.insert(
        "openai",
        ProviderDescriptor {
            id: "openai".into(),
            display_name: "OpenAI".into(),
            base_url: parse_url("https://api.openai.com/v1"),
            auth_flavor: AuthFlavor::BearerApiKey,
            env_key_var: "OPENAI_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "gpt-4o".into(),
            supports: ProviderFeatures {
                streaming: true,
                tool_calling: true,
                vision: true,
                embeddings: true,
                json_mode: true,
            },
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 2. anthropic (existing native impl in providers/anthropic.rs)
    m.insert(
        "anthropic",
        ProviderDescriptor {
            id: "anthropic".into(),
            display_name: "Anthropic".into(),
            base_url: parse_url("https://api.anthropic.com/v1"),
            auth_flavor: AuthFlavor::Custom("x-api-key".into()),
            env_key_var: "ANTHROPIC_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "claude-sonnet-4-6".into(),
            supports: ProviderFeatures {
                streaming: true,
                tool_calling: true,
                vision: true,
                embeddings: false,
                json_mode: false,
            },
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 3. gemini (Google Generative Language API — no existing native impl;
    //    historically handled via vertex.rs but the spec lists gemini as
    //    its own native entry)
    m.insert(
        "gemini",
        ProviderDescriptor {
            id: "gemini".into(),
            display_name: "Google Gemini".into(),
            base_url: parse_url("https://generativelanguage.googleapis.com/v1beta"),
            auth_flavor: AuthFlavor::Custom("x-goog-api-key".into()),
            env_key_var: "GEMINI_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "gemini-2.5-flash".into(),
            supports: ProviderFeatures {
                streaming: true,
                tool_calling: true,
                vision: true,
                embeddings: true,
                json_mode: true,
            },
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 4. xai (existing support elsewhere; native descriptor here per spec)
    m.insert(
        "xai",
        ProviderDescriptor {
            id: "xai".into(),
            display_name: "xAI".into(),
            base_url: parse_url("https://api.x.ai/v1"),
            auth_flavor: AuthFlavor::BearerApiKey,
            env_key_var: "XAI_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "grok-4".into(),
            supports: ProviderFeatures {
                streaming: true,
                tool_calling: true,
                vision: true,
                embeddings: false,
                json_mode: true,
            },
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 5. bedrock (existing native impl in providers/bedrock.rs)
    m.insert(
        "bedrock",
        ProviderDescriptor {
            id: "bedrock".into(),
            display_name: "AWS Bedrock".into(),
            base_url: parse_url("https://bedrock-runtime.us-east-1.amazonaws.com"),
            auth_flavor: AuthFlavor::Custom("aws-sigv4".into()),
            env_key_var: "AWS_ACCESS_KEY_ID".into(),
            compat_kind: CompatKind::Native,
            default_model: "anthropic.claude-sonnet-4-20250514-v1:0".into(),
            supports: ProviderFeatures {
                streaming: true,
                tool_calling: true,
                vision: true,
                embeddings: false,
                json_mode: false,
            },
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 6-9: STUB natives (gap providers — pending Open Question #3)
    //     Impls live in providers/native_gap.rs

    // 6. azure
    m.insert(
        "azure",
        ProviderDescriptor {
            id: "azure".into(),
            display_name: "Azure OpenAI".into(),
            base_url: parse_url("https://example.openai.azure.com/openai"),
            auth_flavor: AuthFlavor::Custom("api-key".into()),
            env_key_var: "AZURE_OPENAI_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "gpt-4o".into(),
            supports: ProviderFeatures::chat_only(),
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 7. cohere
    m.insert(
        "cohere",
        ProviderDescriptor {
            id: "cohere".into(),
            display_name: "Cohere".into(),
            base_url: parse_url("https://api.cohere.com/v1"),
            auth_flavor: AuthFlavor::BearerApiKey,
            env_key_var: "COHERE_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "command-r-plus".into(),
            supports: ProviderFeatures::chat_only(),
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 8. copilot (GitHub Copilot)
    m.insert(
        "copilot",
        ProviderDescriptor {
            id: "copilot".into(),
            display_name: "GitHub Copilot".into(),
            base_url: parse_url("https://api.githubcopilot.com"),
            auth_flavor: AuthFlavor::BearerApiKey,
            env_key_var: "GITHUB_TOKEN".into(),
            compat_kind: CompatKind::Native,
            default_model: "gpt-4o".into(),
            supports: ProviderFeatures::chat_only(),
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    // 9. minimax
    m.insert(
        "minimax",
        ProviderDescriptor {
            id: "minimax".into(),
            display_name: "MiniMax".into(),
            base_url: parse_url("https://api.minimax.chat/v1"),
            auth_flavor: AuthFlavor::BearerApiKey,
            env_key_var: "MINIMAX_API_KEY".into(),
            compat_kind: CompatKind::Native,
            default_model: "abab6.5s-chat".into(),
            supports: ProviderFeatures::chat_only(),
            headers: HashMap::new(),
            quirks: ProviderQuirks::DEFAULT,
        },
    );

    debug_assert_eq!(
        m.len(),
        9,
        "TASK-AGS-704: NATIVE_REGISTRY must have 9 entries"
    );
    m
});

pub fn list_native() -> Vec<&'static ProviderDescriptor> {
    NATIVE_REGISTRY.values().collect()
}

pub fn get_native(id: &str) -> Option<&'static ProviderDescriptor> {
    // Lazy guarantees &'static through deref
    NATIVE_REGISTRY.get(id)
}

pub fn count_native() -> usize {
    NATIVE_REGISTRY.len()
}
