//! GHOST-003: NATIVE_REGISTRY — descriptor metadata for the 5 native
//! providers (openai, anthropic, gemini, xai, bedrock). Mirror of
//! TASK-AGS-702's OPENAI_COMPAT_REGISTRY but for Native compat_kind.
//! The 4 stub providers (azure, cohere, copilot, minimax) were removed
//! per GHOST-003 Option B — they returned LlmError::Unsupported and
//! had no real wire implementations.

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
            is_gap: false,
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
            is_gap: false,
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
            is_gap: false,
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
            is_gap: false,
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
            is_gap: false,
        },
    );

    debug_assert_eq!(
        m.len(),
        5,
        "GHOST-003: NATIVE_REGISTRY must have 5 entries (4 stubs removed)"
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
