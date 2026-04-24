//! TASK-AGS-706: `build_llm_provider()` runtime dispatcher.
//!
//! Spec: 02-technical-spec.md TECH-AGS-PROVIDERS api_contracts line 1116:
//! `pub fn build_llm_provider(cfg: &LlmConfig, http: Arc<reqwest::Client>)
//! -> Arc<dyn LlmProvider>;`
//!
//! Spec deviation (greenlit 2026-04-13):
//!   Spec signature returns `Arc<dyn LlmProvider>`; we return
//!   `Result<Arc<dyn LlmProvider>, ProviderError>` so credential-miss paths
//!   surface as `ProviderError::MissingCredential` per Validation Criteria
//!   5 and 7. The caller (TASK-AGS-710 / archon-cli main.rs) already
//!   handles error propagation at the boundary.
//!
//! Dispatcher responsibilities:
//!   1. Resolve descriptor via `cfg.resolve_descriptor()` (exact-native ->
//!      compat-prefix -> shorthand fallback).
//!   2. Load `ApiKey` from env, honoring `cfg.api_key_env` override or
//!      `descriptor.env_key_var`; skip for `AuthFlavor::None`.
//!   3. Route on `descriptor.compat_kind`:
//!      - `OpenAiCompat` -> construct `OpenAiCompatProvider` directly.
//!      - `Native` -> delegate to `dispatch_native()`, which is the **one**
//!        allowed `match descriptor.id` site. Native impls are legitimately
//!        different concrete types with hand-rolled constructors.
//!
//! NFR-ARCH-002: the 5 existing native impls (openai, anthropic, bedrock,
//! local, vertex) are wired without any source-code modification to their
//! modules. Their constructors pre-date the descriptor pattern and take
//! non-uniform arguments; where the mapping is trivial we wire them here,
//! and where full wiring requires auth-flow work (GCP ADC for vertex,
//! AnthropicClient for anthropic, Ollama health-probe for local) we return
//! `ProviderError::InvalidResponse` pointing at TASK-AGS-710 which is the
//! task that actually threads `archon-cli/src/main.rs:1885` onto this
//! builder.
//!
//! NFR-SECURITY-001: `ApiKey` never appears in error messages; only the
//! env var *name* is propagated.

use std::sync::Arc;

use crate::config::LlmConfig;
use crate::provider::LlmProvider;
use crate::retry::{RetryPolicy, RetryProvider};
use crate::secrets::ApiKey;

use super::descriptor::{AuthFlavor, CompatKind, ProviderDescriptor};
use super::error::ProviderError;
use super::local::LocalProvider;
use super::native_gap::{AzureProvider, CohereProvider, CopilotProvider, MinimaxProvider};
use super::openai::OpenAiProvider;
use super::openai_compat::OpenAiCompatProvider;

/// Single public entry point for obtaining an `Arc<dyn LlmProvider>` from
/// a runtime `LlmConfig`. Every code path that previously instantiated a
/// provider directly should route through this function.
///
/// TASK-AGS-708: the returned provider is wrapped in `RetryProvider` so
/// every caller automatically gets ERR-PROV-02 retry semantics. The
/// policy comes from `cfg.retry` if set, otherwise `RetryPolicy::default()`.
pub fn build_llm_provider(
    cfg: &LlmConfig,
    http: Arc<reqwest::Client>,
) -> Result<Arc<dyn LlmProvider>, ProviderError> {
    let policy = cfg.retry.clone().map(RetryPolicy::from).unwrap_or_default();
    build_llm_provider_with_policy(cfg, http, policy)
}

/// Variant of `build_llm_provider` that accepts an explicit `RetryPolicy`,
/// for call sites that want to override the default (tests, retries
/// disabled, custom backoff). The returned provider is still wrapped in
/// `RetryProvider` — pass `RetryPolicy { max_attempts: 1, .. }` to
/// effectively disable retries.
pub fn build_llm_provider_with_policy(
    cfg: &LlmConfig,
    http: Arc<reqwest::Client>,
    policy: RetryPolicy,
) -> Result<Arc<dyn LlmProvider>, ProviderError> {
    let descriptor = cfg.resolve_descriptor()?;

    let api_key = match descriptor.auth_flavor {
        AuthFlavor::None => ApiKey::new(String::new()),
        _ => {
            let var = cfg
                .api_key_env
                .as_deref()
                .unwrap_or(descriptor.env_key_var.as_str());
            ApiKey::from_env(var)?
        }
    };

    let inner: Arc<dyn LlmProvider> = match descriptor.compat_kind {
        CompatKind::OpenAiCompat => Arc::new(OpenAiCompatProvider::new(descriptor, http, api_key)),
        CompatKind::Native => dispatch_native(cfg, descriptor, http, api_key)?,
    };

    Ok(Arc::new(RetryProvider::<dyn LlmProvider>::new(
        inner, policy,
    )))
}

/// The **one** allowed `match descriptor.id` site in the whole providers
/// module. Native impls are legitimately different concrete types (5
/// hand-rolled + 4 TASK-AGS-704 stubs) and cannot be unified under a
/// single parametric constructor.
///
/// Validation Criterion 8: `grep -c 'match.*descriptor.id'` on this file
/// MUST return exactly 1 — that is this match, right below.
///
/// TASK-AGS-710: the `cfg` parameter threads flat-config overrides
/// (`base_url`, `model`) into native arms that can accept them (currently
/// just `local`). Other natives ignore `cfg` because their construction
/// data comes from the descriptor + `api_key` only.
fn dispatch_native(
    cfg: &LlmConfig,
    descriptor: &'static ProviderDescriptor,
    http: Arc<reqwest::Client>,
    api_key: ApiKey,
) -> Result<Arc<dyn LlmProvider>, ProviderError> {
    match descriptor.id.as_str() {
        // --- 4 TASK-AGS-704 stubs: uniform (descriptor, http, api_key) ---
        "azure" => Ok(Arc::new(AzureProvider::new(descriptor, http, api_key))),
        "cohere" => Ok(Arc::new(CohereProvider::new(descriptor, http, api_key))),
        "copilot" => Ok(Arc::new(CopilotProvider::new(descriptor, http, api_key))),
        "minimax" => Ok(Arc::new(MinimaxProvider::new(descriptor, http, api_key))),

        // --- openai: hand-rolled native, simplest constructor ---
        "openai" => {
            let default_model = descriptor.default_model.clone();
            let base_url = descriptor.base_url.to_string();
            let key = api_key.expose().to_string();
            Ok(Arc::new(OpenAiProvider::new(
                key,
                Some(base_url),
                default_model,
            )))
        }

        // --- TASK-AGS-710: newly wired native arms ------------------------

        // xai's native-registry descriptor declares CompatKind::Native, but
        // its wire protocol is OpenAI-compatible (https://api.x.ai/v1).
        // Route through OpenAiCompatProvider so the native-registry entry
        // drives behavior via the compat adapter.
        "xai" => Ok(Arc::new(OpenAiCompatProvider::new(
            descriptor, http, api_key,
        ))),

        // `local` is not currently in NATIVE_REGISTRY (the archon-cli
        // adapter at src/runtime/llm.rs handles the `local` case directly
        // against the nested archon_core::config::LlmLocalConfig shape),
        // so this arm is defensive dead code protecting against future
        // registry additions. Honors cfg.base_url / cfg.model overrides
        // when the entry does get registered.
        "local" => {
            let base_url = cfg
                .base_url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| descriptor.base_url.to_string());
            let model = cfg
                .model
                .clone()
                .unwrap_or_else(|| descriptor.default_model.clone());
            Ok(Arc::new(LocalProvider::new(base_url, model, 300, true)))
        }

        // --- TASK-AGS-710: explicit architectural errors
        //
        // These natives cannot be constructed from the flat
        // archon_llm::LlmConfig because their constructors require shapes
        // the flat config cannot express. Callers that need them must go
        // through the archon-cli adapter (src/runtime/llm.rs) which reads
        // the nested archon_core::config::LlmConfig.
        "anthropic" => Err(ProviderError::InvalidResponse {
            name: descriptor.display_name.clone(),
            detail: "native anthropic requires AnthropicClient (auth + \
                     identity) not expressible in the flat \
                     archon_llm::LlmConfig; construct directly via \
                     archon_llm::providers::AnthropicProvider::new or \
                     through the archon-cli runtime/llm.rs wrapper"
                .to_string(),
        }),

        "bedrock" => Err(ProviderError::InvalidResponse {
            name: descriptor.display_name.clone(),
            detail: "native bedrock requires region + model_id from nested \
                     archon_core::config::LlmBedrockConfig; not expressible \
                     in the flat archon_llm::LlmConfig — use the archon-cli \
                     runtime/llm.rs wrapper"
                .to_string(),
        }),

        "vertex" => Err(ProviderError::InvalidResponse {
            name: descriptor.display_name.clone(),
            detail: "native vertex requires project_id, region, \
                     credentials_file from nested \
                     archon_core::config::LlmVertexConfig; not expressible \
                     in the flat archon_llm::LlmConfig — use the archon-cli \
                     runtime/llm.rs wrapper"
                .to_string(),
        }),

        "gemini" => Err(ProviderError::InvalidResponse {
            name: descriptor.display_name.clone(),
            detail: "native gemini (Google Generative Language API with \
                     x-goog-api-key auth) has no concrete provider impl in \
                     this crate; use Vertex AI via archon-cli runtime/llm.rs \
                     wrapper or add a native gemini provider"
                .to_string(),
        }),

        // Any id the registry claims is Native but dispatch doesn't know
        // about is a registry/dispatch skew bug. Surface as
        // InvalidResponse so caller gets a clear message.
        other => Err(ProviderError::InvalidResponse {
            name: descriptor.display_name.clone(),
            detail: format!(
                "unknown native provider id `{other}` — native_registry and dispatch_native are out of sync"
            ),
        }),
    }
}
