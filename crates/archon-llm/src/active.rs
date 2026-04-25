//! TASK-AGS-709: `ActiveProvider` — an ArcSwap-backed live-swappable
//! `LlmProvider` handle. Every call delegates to whichever provider is
//! currently stored; `swap()` atomically replaces the backing provider
//! without disturbing in-flight requests (session state preserved per
//! US-PROV-03 AC-01).
//!
//! Spec refs:
//!   - 01-functional-spec.md US-PROV-03 (lines 1794-1807)
//!     "I can switch models without losing the current session"
//!   - 01-functional-spec.md EC-PROV-02 (line 1702)
//!     "fall back to previously selected provider"
//!   - 02-technical-spec.md TECH-AGS-TUI-MOD (line 978)
//!     "consumes provider registry with live switch"
//!
//! ## Atomicity guarantee
//!
//! `ActiveProvider::swap` builds the NEW provider first, then stores it
//! atomically via `ArcSwap::store`. If construction fails (missing env
//! var, unknown provider, network probe failure), the old provider is
//! left completely untouched — the `ArcSwap::store` is never reached.
//! This is the EC-PROV-02 fallback contract.
//!
//! ## Concurrency
//!
//! `ArcSwap` guarantees lock-free `load()` for readers. Ongoing
//! in-flight requests hold an `Arc` to the OLD provider (returned by
//! `current()` before the swap) and will finish normally. New requests
//! issued after the swap pick up the NEW provider via a fresh
//! `current()` call. There is no "torn swap" where a request sees half
//! of the old state and half of the new.

use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::anthropic::AnthropicClient;
use crate::config::LlmConfig;
use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::providers::{ProviderDescriptor, ProviderError, build_llm_provider_with_policy};
use crate::retry::RetryPolicy;
use crate::streaming::StreamEvent;

/// Snapshot of the "currently active" provider + its descriptor. Stored
/// inside an `ArcSwap<ProviderState>` so we can hot-swap both fields in
/// a single atomic step (no possibility of a request seeing descriptor
/// A and provider B, or vice versa).
struct ProviderState {
    provider: Arc<dyn LlmProvider>,
    descriptor: &'static ProviderDescriptor,
}

/// Live-swappable `LlmProvider` handle. Implements `LlmProvider` itself
/// so existing agent-loop call sites that hold `Arc<dyn LlmProvider>`
/// can be rewired to an `ActiveProvider` without any further changes.
///
/// # Examples
///
/// ```ignore
/// let http = Arc::new(reqwest::Client::new());
/// let cfg = LlmConfig { provider: "groq".into(), .. };
/// let active = Arc::new(ActiveProvider::new(&cfg, http)?);
/// // ... pass `active` wherever `Arc<dyn LlmProvider>` is needed ...
/// active.swap(&LlmConfig { provider: "deepseek".into(), .. })?;
/// ```
pub struct ActiveProvider {
    state: ArcSwap<ProviderState>,
    http: Arc<reqwest::Client>,
    retry_policy: RetryPolicy,
}

impl ActiveProvider {
    /// Build an `ActiveProvider` from an `LlmConfig`. Resolves the
    /// descriptor, constructs the initial provider via
    /// `build_llm_provider_with_policy`, and stores both atomically.
    ///
    /// The `RetryPolicy` is cloned from `cfg.retry` (or
    /// `RetryPolicy::default()` if absent) and PERSISTS across swaps —
    /// that is, `swap()` uses THIS policy even if the new `LlmConfig`
    /// has a different `retry` field. This keeps backoff behavior
    /// predictable when the user flips models at runtime.
    ///
    /// # Errors
    ///
    /// - `ProviderError::MissingCredential` if `cfg.provider` is
    ///   unknown or the required env var is absent.
    /// - `ProviderError::InvalidResponse` if a native provider's
    ///   hand-rolled constructor fails.
    pub fn new(cfg: &LlmConfig, http: Arc<reqwest::Client>) -> Result<Self, ProviderError> {
        let retry_policy = cfg.retry.clone().map(RetryPolicy::from).unwrap_or_default();
        let descriptor = cfg.resolve_descriptor()?;
        let provider = build_llm_provider_with_policy(cfg, http.clone(), retry_policy.clone())?;
        Ok(Self {
            state: ArcSwap::from_pointee(ProviderState {
                provider,
                descriptor,
            }),
            http,
            retry_policy,
        })
    }

    /// Test/integration helper: build an `ActiveProvider` from a
    /// pre-constructed provider + descriptor without going through
    /// env vars. TASK-AGS-710 will also use this to wire a
    /// pre-initialized Anthropic/Bedrock/Vertex provider that takes
    /// non-trivial setup outside `build_llm_provider`'s knowledge.
    pub fn from_parts(
        provider: Arc<dyn LlmProvider>,
        descriptor: &'static ProviderDescriptor,
        http: Arc<reqwest::Client>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            state: ArcSwap::from_pointee(ProviderState {
                provider,
                descriptor,
            }),
            http,
            retry_policy,
        }
    }

    /// Cheap, lock-free access to the currently active provider.
    /// Returns an `Arc` clone so callers may hold onto it across await
    /// points without blocking subsequent swaps.
    pub fn current(&self) -> Arc<dyn LlmProvider> {
        self.state.load().provider.clone()
    }

    /// Returns the `&'static ProviderDescriptor` of the currently
    /// active provider. The reference is `'static` because descriptors
    /// live in the NATIVE_REGISTRY / OPENAI_COMPAT_REGISTRY once_cells
    /// (or, for tests, in `Box::leak`ed storage).
    pub fn current_descriptor(&self) -> &'static ProviderDescriptor {
        self.state.load().descriptor
    }

    /// Atomically replace the active provider with one constructed
    /// from `new_cfg`.
    ///
    /// # Atomicity
    ///
    /// The new provider is fully built BEFORE `store()` is called. If
    /// construction fails, the old provider is unchanged — this is the
    /// EC-PROV-02 fallback contract that US-PROV-03 depends on.
    ///
    /// # Errors
    ///
    /// Same as `ActiveProvider::new`: `ProviderError::MissingCredential`
    /// for unknown providers / missing env vars, `InvalidResponse` for
    /// hand-rolled native constructor failures. On `Err`, the old
    /// provider remains active and callers may retry.
    pub fn swap(&self, new_cfg: &LlmConfig) -> Result<&'static ProviderDescriptor, ProviderError> {
        let descriptor = new_cfg.resolve_descriptor()?;
        let provider =
            build_llm_provider_with_policy(new_cfg, self.http.clone(), self.retry_policy.clone())?;
        self.state.store(Arc::new(ProviderState {
            provider,
            descriptor,
        }));
        Ok(descriptor)
    }

    /// Test/integration helper: atomically swap to a pre-constructed
    /// provider + descriptor pair. Same atomicity guarantees as
    /// `swap()` but bypasses `build_llm_provider_with_policy` so tests
    /// can use wiremock-backed descriptors built via `Box::leak`.
    pub fn swap_parts(
        &self,
        provider: Arc<dyn LlmProvider>,
        descriptor: &'static ProviderDescriptor,
    ) {
        self.state.store(Arc::new(ProviderState {
            provider,
            descriptor,
        }));
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl — every call delegates to `self.current()`
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for ActiveProvider {
    /// Returns the display_name of the currently active descriptor.
    /// Because the descriptor is `&'static ProviderDescriptor`, its
    /// `display_name: String` field lives in static memory and we can
    /// return `&'static str` here — satisfying the trait's elided
    /// `&self` lifetime without borrowing through an ArcSwap guard.
    fn name(&self) -> &str {
        // Copy the &'static descriptor out of the guard so the returned
        // &str is not tied to the guard's short lifetime.
        let descriptor: &'static ProviderDescriptor = self.state.load().descriptor;
        descriptor.display_name.as_str()
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.current().models()
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        self.current().supports_feature(feature)
    }

    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        // `as_anthropic` hands back a borrowed reference tied to
        // `&self`. We cannot safely do that through an ArcSwap load
        // (the Arc would drop at end of expression). For now, return
        // `None` — TASK-AGS-710 will rework the Anthropic path so the
        // caller holds the client directly rather than fishing it
        // through this trait method.
        None
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.current().complete(request).await
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.current().stream(request).await
    }
}
