# LLM Error Match-Site Audit

`archon_llm::provider::LlmError` is marked `#[non_exhaustive]` so adding
provider error variants does not break downstream exhaustive matches.

Audited after adding `ContextWindowExceeded`:

- `crates/archon-llm/src/retry.rs`: same-crate classifier; handles
  `ContextWindowExceeded` as fail-fast.
- `src/runtime/provider_observer.rs`: downstream match sites include
  `ContextWindowExceeded` and wildcard fallbacks.
- `src/runtime/provider_profile_updates.rs`: downstream error reason mapping
  includes `ContextWindowExceeded` and a wildcard fallback.
- `src/runtime/provider_limit_windows.rs`: downstream rate/quota extraction
  already uses wildcard fallbacks for non-rate-limit errors.

Integration-test match sites already use fallback arms where they inspect
specific variants.
