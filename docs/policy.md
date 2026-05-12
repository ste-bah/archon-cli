# Archon Policy

Archon policy is a TOML gate for features that can change behaviour, use
networked providers, expose services, store raw text, or auto-apply learned
updates.

There is currently no `archon policy` CLI namespace. Policy is loaded by the
feature gates that need it.

## Locations

Policy is loaded in this order, with later files overriding earlier files:

1. `/etc/archon/policy.toml`
2. `~/.archon/policy.toml`
3. `<workspace>/.archon/policy.toml`

If no policy file exists, Archon uses default-deny for network, VLM,
game-theory Tier 11, MCP exposure, cloud learning workers, and governed-learning
auto-apply.

## Example

```toml
[policy.network]
default = "deny"
allow_cloud_vlm = false
allow_web_strategy_agents = false
allow_mcp_server_exposure = false

[policy.workers]
ocr = "allow-local"
embedding = "allow-local"
vlm = "deny"
web_fetch = "deny"

[policy.gametheory]
max_agents_per_council = 12
max_cost_usd = 20.00
enable_tier11 = false
allow_web_tools = false

[policy.learning]
auto_apply_low_risk = false
require_approval_for_prompt_changes = true
require_approval_for_blocking_gates = true
require_approval_for_network_changes = true

[policy.world_model]
allow_third_party_embeddings = false
allow_llm_labeler = false
allow_behavior_changes = false

[policy.web]
allow_mutating_actions = false
allow_file_uploads = false
allow_pipeline_controls = false
allow_model_training_actions = false
allow_corpus_open_paths = false

[policy.reasoning_quality]
allow_llm_critic = false
allow_critic_cloud_data_flow = false
allow_third_party_critic = false
allow_raw_text_storage = false
allow_behavior_proposal_generation = true
allow_session_start_injection = true
allow_trust_updates_during_shadow = false
auto_migrate_reasoning_quality = false

[policy.docs.vlm]
enabled = false
mode = "disabled" # disabled | local | cloud | hybrid
provider = "disabled" # disabled | ollama | gemini | anthropic | openai-compat
allow_cloud = false
require_user_confirmation_for_cloud = true

[policy.docs.vlm.ollama]
endpoint = "http://localhost:11434"
model = "gemma4:e4b"
timeout_secs = 120

[policy.docs.vlm.gemini]
api_key_env = "GOOGLE_API_KEY"
model = "gemini-3-flash-preview"
endpoint_base = "https://generativelanguage.googleapis.com/v1beta"
rpm_limit = 12

[policy.docs.vlm.anthropic]
model = "claude-sonnet-4-6"

[policy.docs.vlm.openai_compat]
endpoint = "http://localhost:1234/v1"
model = "google/gemma-3-12b-it"
api_key_env = "OPENAI_API_KEY"
timeout_secs = 120
max_tokens = 8192
temperature = 0.2

[policy.docs.pdf]
extract_embedded_images = true
min_image_dimension = 200
min_image_bytes = 4096
vlm_per_page_image = true
render_text_pdf_pages = false

[policy.docs.retrieval]
exact_weight = 0.45
semantic_weight = 0.55
```

## Current Gates

`archon gametheory run --enable-tier11` only enables Tier 11 specialists when
`policy.gametheory.enable_tier11 = true`.

Document VLM descriptions are denied unless `[policy.docs.vlm]` enables a
provider and the matching worker/network policy allows it. Local Ollama requires
`policy.workers.vlm = "allow-local"`. Gemini and Anthropic require
`policy.workers.vlm = "allow-cloud"`, `policy.docs.vlm.allow_cloud = true`, and
`policy.network.allow_cloud_vlm = true`.

PDF image extraction is enabled by default through `[policy.docs.pdf]`, but VLM
calls for those extracted images still require the normal VLM gates.
`render_text_pdf_pages = false` means full-page rendering is only used for
scanned/image-only fallback unless explicitly enabled.

Document search defaults to hybrid retrieval. `[policy.docs.retrieval]` controls
the exact/semantic weighting used by `archon docs search --mode hybrid`.

Governed-learning auto-apply is denied by default. Low-risk updates can only
auto-apply when `policy.learning.auto_apply_low_risk = true`; prompt, blocking
gate, policy, and network changes remain approval-gated.

The local world model is advisory by default. Third-party embeddings require
both `[learning.world_model.embeddings].allow_third_party = true` and
`policy.world_model.allow_third_party_embeddings = true`; cloud embedding calls
also require `policy.workers.embedding = "allow-cloud"` and
`policy.network.default = "allow"`. LLM-assisted semantic labeling requires
`policy.world_model.allow_llm_labeler = true`. Any world-model path that can
change runtime behaviour requires `policy.world_model.allow_behavior_changes =
true`; current runtime integrations remain advisory and fail open.

The browser workbench is inspect-only by default. Browser-originated actions
require `policy.web.allow_mutating_actions = true` plus the matching
action-family gate (`allow_file_uploads`, `allow_pipeline_controls`,
`allow_model_training_actions`, or `allow_corpus_open_paths`). World-model
training and checkpoint-promotion actions also require
`policy.world_model.allow_behavior_changes = true`.

Reasoning-quality extraction is local and deterministic by default. Optional
LLM critique requires both `learning.reasoning_quality.critic.allow_llm = true`
and `policy.reasoning_quality.allow_llm_critic = true`. If the active
`LlmProvider` is cloud-hosted, `allow_critic_cloud_data_flow = true` must also
be set. Raw visible-turn text persistence requires
`allow_raw_text_storage = true`; otherwise Archon stores redacted excerpts,
hashes, and redacted entity keys.

## Full State Verification

Policy verification is feature-specific:

| Gate | Trigger | Expected source-of-truth read |
|---|---|---|
| Tier 11 | `archon gametheory run ... --enable-tier11` | routing output shows Tier 11 only when policy allows it |
| VLM | `archon docs ingest <image-or-pdf>` | OCR/VLM rows show local/denied/provider state |
| Hybrid retrieval | `archon docs search <query> --mode hybrid --debug` | debug output shows exact and semantic score components |
| Governed learning | `archon behaviour apply <proposal-id>` | proposal decision and manifest history reflect policy outcome |
| World model | `archon world predict-next ...` | unavailable advisors fail open; behavior-changing use remains disabled unless policy allows it |
| Reasoning quality | `archon reasoning status` | critic/cloud/raw-text gates and dead-letter state match policy |

Keep policy files in source control for project-level gates when possible:

```text
<workspace>/.archon/policy.toml
```
