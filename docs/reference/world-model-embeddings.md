# World Model Embeddings

World-model embeddings are configured separately from chat/provider routing.

```toml
[learning.world_model.embeddings]
source = "local" # local | third_party | auto
provider = "fastembed"
model = "bge-base-en-v1.5"
dimensions = 768
projection_dim = 384
cache_enabled = true
cache_max_mb = 1024
redact_before_embedding = true
allow_third_party = false
```

## Provider Matrix

| Source | Provider | Default | Policy posture |
|---|---|---|---|
| `local` | FastEmbed BGE | Yes | Allowed by default. |
| `third_party` | OpenAI-compatible or provider API | No | Requires config and policy approval. |
| `auto` | Local first, external only when allowed | No | Must respect `allow_third_party` and policy gates. |

The implementation reuses `archon-memory` for FastEmbed and OpenAI embedding
calls, redacts text before embedding by default, folds vectors into the
configured world-model state dimension, and stores persistent cache entries
under `~/.archon/world-model/embeddings/cache`.

External embedding calls are gated by both `[learning.world_model.embeddings]`
and `[policy.world_model]`. Approved third-party calls write audit events to
`~/.archon/world-model/ledgers/embedding-policy-events.jsonl` with provider,
model, dimensionality, redaction state, and policy reason metadata.

## Cache Key

Embedding cache entries include:

| Component | Why it matters |
|---|---|
| provider and model | avoids mixing embedding spaces |
| source dimensions | distinguishes local and external dimensions |
| projection dimensions | tracks the world-model latent projection |
| redaction policy | prevents pre-redaction and post-redaction rows colliding |
| source hash | deduplicates without storing raw text |
| redacted text hash | protects against accidental source-hash reuse |

Conversation and agent-output rows store redacted excerpts plus evidence references. Raw transcripts remain in their original source artifacts.
