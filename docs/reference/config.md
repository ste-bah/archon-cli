# Configuration

archon-cli reads layered TOML config from (lowest to highest precedence — later layers override earlier):

1. `~/.config/archon/config.toml` — **user** layer (global, all projects)
2. `<workdir>/.archon/config.toml` — **project** layer (committed to repo)
3. `<workdir>/.archon/config.local.toml` — **local** layer (gitignored, secrets)
4. `<workdir>/.archon/worktrees/<name>.toml` — **worktree** layer (per-git-worktree)
5. `--settings <PATH>` — **CLI overlay**
6. Environment variables (see [env-vars](env-vars.md))
7. CLI flags (see [cli-flags](cli-flags.md))

On first run archon generates a commented config file at `~/.config/archon/config.toml` with all defaults. The repo also ships:
- `config.toml` at archon-cli repo root — example/template (copy to one of the layers above to activate)
- `.archon/config.toml` — project config that ships with the repo

This page explains every section. Each table tells you **what** the field does, **how** it affects behavior, and **why** you might change it.

## Table of contents

- [`[api]`](#api) — Anthropic API + model defaults
- [`[llm]`](#llm) — provider routing
- [`[identity]`](#identity) — Claude Code spoofing
- [`[personality]`](#personality) — agent personality profile
- [`[consciousness]`](#consciousness) — inner voice and rule engine
- [`[tools]`](#tools) — tool execution defaults
- [`[permissions]`](#permissions) — tool gating
- [`[context]`](#context) — compaction and prompt cache
- [`[memory]`](#memory) — memory graph backbone
- [`[memory.garden]`](#memorygarden) — background consolidation
- [`[memory.auto_capture]`](#memoryauto_capture) — regex memory detection
- [`[memory.auto_extraction]`](#memoryauto_extraction) — LLM fact extraction
- [`[learning.*]`](#learning) — 8 learning subsystems
- [`[learning.gnn]`](#learninggnn) — GNN model
- [`[learning.gnn.training]`](#learninggnntraining) — GNN training hyperparams
- [`[learning.gnn.auto_trainer]`](#learninggnnauto_trainer) — background retraining
- [`[learning.reflexion]`](#learningreflexion) — N-attempt retry
- [`[cost]`](#cost) — spending guardrails
- [`[logging]`](#logging) — log rotation
- [`[session]`](#session) — session persistence
- [`[checkpoint]`](#checkpoint) — file snapshots
- [`[tui]`](#tui) — terminal UI
- [`[update]`](#update) — self-update channel
- [`[remote]` / `[remote.ssh]`](#remote) — SSH remote agent
- [`[ws_remote]`](#ws_remote) — WebSocket server
- [`[orchestrator]`](#orchestrator) — multi-agent teams
- [`[voice]`](#voice) — speech-to-text
- [`[web]`](#web) — browser UI
- [Authentication & secrets](#authentication--secrets)
- [Local LLMs and proxies](#local-llms-and-proxies)
- [Recipes](#recipes)

---

## `[api]`

Anthropic API model and HTTP behavior.

```toml
[api]
default_model = "claude-sonnet-4-6"
thinking_budget = 16384
default_effort = "high"
max_retries = 3
# base_url = "http://localhost:4000/v1/messages"
```

| Field | Default | What / Why |
|---|---|---|
| `default_model` | `"claude-sonnet-4-6"` | Model used for the main agent. Options: `claude-haiku-4-5-20251001` (fast/cheap), `claude-sonnet-4-6` (balanced), `claude-opus-4-7` (highest quality). Override per-session with `--model <NAME>` or `/model` slash command. |
| `thinking_budget` | `16384` | Max tokens of "extended thinking" the model can use per turn. `0` disables extended thinking. Higher = more thorough reasoning but slower + costs more. Only Sonnet 4.6 and Opus 4.7 support extended thinking; Haiku ignores this. |
| `default_effort` | `"high"` | Reasoning effort: `"low"`, `"medium"`, `"high"`. Maps to thinking budget tiers. `low` is best for quick lookups, `high` for code generation. Override with `--effort` or `/effort`. |
| `max_retries` | `3` | HTTP retry attempts on transient failures (5xx, network errors). Each retry uses exponential backoff. Set higher for unreliable networks; never set 0 (rate-limit hiccups become hard failures). |
| `base_url` | unset | Override API endpoint. Use to route through LiteLLM, Ollama, or any Anthropic-compatible proxy. See [Local LLMs and proxies](#local-llms-and-proxies). |

---

## `[llm]`

Provider routing and per-provider settings.

```toml
[llm]
provider = "anthropic"
# [llm.openai]   - alternative LLM provider for chat
# [llm.bedrock]  - AWS Bedrock
# [llm.vertex]   - GCP Vertex
# [llm.local]    - local model server
```

| Field | Default | What / Why |
|---|---|---|
| `provider` | `"anthropic"` | Active LLM provider. Use `"openai"` to route chat through OpenAI models instead of Claude. Anthropic is the default because archon-cli uses Claude features (extended thinking, prompt caching) heavily. Set this only if you must use a non-Anthropic LLM. |

Sub-tables `[llm.openai]`, `[llm.bedrock]`, `[llm.vertex]`, `[llm.local]` configure each provider. Most users only need `[llm.openai]` if using OpenAI as primary or for embeddings — see [env-vars](env-vars.md#resolution-order-for-openai-key).

---

## `[identity]`

Claude Code spoofing — what HTTP headers and identity fields archon-cli sends to Anthropic.

```toml
[identity]
mode = "spoof"
spoof_version = "2.1.89"
anti_distillation = false
```

| Field | Default | What / Why |
|---|---|---|
| `mode` | `"spoof"` | `"spoof"` mimics Claude Code (User-Agent, billing header, beta headers, system-prompt prelude). `"native"` identifies as archon-cli. Spoof mode lets archon-cli use Claude.ai subscriptions transparently because Anthropic's billing keys off the spoofed identity. Switch to `"native"` if you're on a non-Claude proxy that doesn't care or breaks on spoof headers. |
| `spoof_version` | `"2.1.89"` | Fallback version reported in the billing header when no Claude Code is detected on disk. archon-cli first tries to read the installed Claude Code version from `package.json`; this is the fallback. Bump this when Anthropic deprecates the previous spoof version (rare). |
| `anti_distillation` | `false` | Inject an anti-distillation field. Default off because most use cases don't need it. Turn on if you're concerned about training-data leakage in your prompts. |

See [Identity & spoofing](../integrations/identity-spoofing.md) for the full spoof layer list.

---

## `[personality]`

Agent personality injected into the system prompt.

```toml
[personality]
name = "Archon"
type = "INTJ"
enneagram = "4w5"
traits = ["strategic", "direct", "truth-over-comfort"]
communication_style = "terse"
```

| Field | What / Why |
|---|---|
| `name` | Shown in TUI header and self-references. |
| `type` | MBTI four-letter code. Auto-selects matching theme (`intj` → cold cyan / midnight blue palette). 16 valid types — see [TUI customization](../operations/tui-customization.md#mbti-themes). |
| `enneagram` | Enneagram type with optional wing (`4w5`). Cosmetic; flavors the personality prompt. |
| `traits` | Free-form trait list injected into system prompt. The model adopts these as voice characteristics. |
| `communication_style` | One of: `"terse"`, `"detailed"`, `"casual"`, `"formal"`. Controls verbosity of responses. |

---

## `[consciousness]`

Background "inner voice", behavioral rules engine, and personality persistence.

```toml
[consciousness]
inner_voice = true
energy_decay_rate = 0.02
persist_personality = true
personality_history_limit = 50
initial_rules = [
    "Always ask before modifying files",
    "Explain reasoning before acting",
]
```

| Field | Default | What / Why |
|---|---|---|
| `inner_voice` | `true` | Run a brief background monologue before generating each response. Improves response coherence at the cost of a small extra token use per turn. Disable for cheapest/fastest operation. |
| `energy_decay_rate` | `0.02` | Per-turn decay of the inner-voice "energy" state — controls how often it fires fully vs. minimally. Higher = quicker fade. Most users never touch this. |
| `persist_personality` | `true` | Save inner-voice + rule scores across sessions. Without this, every session starts cold. |
| `personality_history_limit` | `50` | Max snapshots retained for trend tracking. After 50, oldest is rotated out. |
| `initial_rules` | (3 starter rules) | Behavioral rules injected into the system prompt's `<rules>` block. Edit these to bake in project-specific guardrails. |

Combine with `[memory]` for persistent self-correction memory.

---

## `[tools]`

Tool execution defaults.

```toml
[tools]
bash_timeout = 120
bash_max_output = 102400
max_concurrency = 4
```

| Field | Default | What / Why |
|---|---|---|
| `bash_timeout` | `120` (seconds) | Hard timeout for `Bash` tool invocations. Long-running tasks (compiles, tests) need higher; raise to 600 if your project's build is slow. Tools wrapping long ops should use streaming via `Monitor` instead of bumping this for everything. |
| `bash_max_output` | `102400` (bytes, ~100KB) | Maximum bytes of stdout captured per Bash call. Output beyond this is truncated. Raise if you're parsing large logs; lower to keep token spend down. |
| `max_concurrency` | `4` | Maximum concurrent tool invocations the parent agent runs in parallel via `join_all`. Higher = faster multi-tool turns, more memory pressure. WSL2 tolerates 2-4; native machines can go higher. |

---

## `[permissions]`

Tool gating policy. See [Permissions reference](permissions.md) for full details.

```toml
[permissions]
mode = "default"
always_allow = ["Read:*", "Glob:*", "Grep:*"]
always_deny = ["Bash:rm -rf*", "Write:/etc/*"]
always_ask = ["Bash:git push*"]
allow_paths = ["/home/user/project"]
deny_paths = ["/etc", "/.ssh"]
sandbox = false
```

| Field | What / Why |
|---|---|
| `mode` | One of 7 canonical modes: `default` (prompt for risky), `acceptEdits` (auto-allow file edits), `plan` (read-only), `auto` (heuristic), `dontAsk` (allow except deny rules), `bubble` (sandbox), `bypassPermissions` (skip all). Legacy aliases: `ask` → `default`, `yolo` → `bypassPermissions`. |
| `always_allow` | List of `Tool:pattern` strings auto-approved regardless of mode. Patterns support glob (`*`, `**`). |
| `always_deny` | List of `Tool:pattern` strings ALWAYS denied — even in `bypassPermissions`. Use for hard guards (`rm -rf /`, secrets paths, etc.). |
| `always_ask` | Force a prompt even in auto/dontAsk modes. Useful for one specific operation you want eyes on. |
| `allow_paths` | Restrict file tools to these paths. Empty list = no restriction. |
| `deny_paths` | Block file tools from these paths. Takes precedence over `allow_paths`. |
| `sandbox` | When `true`, enforce read-only across all file tools regardless of mode. |

Rule precedence: `always_deny` > `always_ask` > `always_allow` > mode default.

---

## `[context]`

Context-window management.

```toml
[context]
compact_threshold = 0.8
preserve_recent_turns = 3
prompt_cache = true
```

| Field | Default | What / Why |
|---|---|---|
| `compact_threshold` | `0.8` | Trigger automatic compaction when context fill ratio crosses 80%. Lower = compact earlier (loses old detail sooner but stays responsive); higher = keep raw turns longer (more cost, smaller context budget for new turns). |
| `preserve_recent_turns` | `3` | Always keep the last N turns verbatim across compaction. Keeps the immediate working context intact while older turns get summarized. |
| `prompt_cache` | `true` | Set Anthropic's `cache_control` flag on static blocks (system prompt, tool catalog, memory briefing). Cache hits are billed at a fraction of input cost; reduces session cost dramatically over long conversations. Disable only if you're hitting cache-correctness bugs. |

---

## `[memory]`

Memory graph backbone (CozoDB).

```toml
[memory]
enabled = true
embedding_provider = "auto"
hybrid_alpha = 0.3
# db_path = "/custom/path/memory.db"
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Master switch for the memory graph. Disable for ephemeral / stateless sessions. |
| `db_path` | unset | Override CozoDB file path. Default: `~/.local/share/archon/memory.db`. Use to share a memory graph across multiple archon-cli installations or to keep memory on faster storage. |
| `embedding_provider` | `"auto"` | `"auto"` uses OpenAI if `OPENAI_API_KEY` (or `ARCHON_MEMORY_OPENAIKEY`) is set, else falls back to local fastembed. `"local"` forces local (768-dim BGE-base-en-v1.5, no network). `"openai"` forces OpenAI (1536-dim text-embedding-3-small, requires API key). Local is fine for most use cases. |
| `hybrid_alpha` | `0.3` | Hybrid search blend: `0.0` = pure vector, `1.0` = pure keyword (BM25). `0.3` = 70% vector / 30% keyword. Lower for semantic precision, higher for exact-term matching. |

---

## `[memory.garden]`

Background memory consolidation. Runs at session start when throttle elapses.

```toml
[memory.garden]
auto_consolidate = true
min_hours_between_runs = 24
dedup_similarity_threshold = 0.92
staleness_days = 30
staleness_importance_floor = 0.3
importance_decay_per_day = 0.01
max_memories = 5000
briefing_limit = 15
```

| Field | Default | What / Why |
|---|---|---|
| `auto_consolidate` | `true` | Run garden on session start if throttle elapsed. Disable to control consolidation manually via `/garden`. |
| `min_hours_between_runs` | `24` | Throttle. Won't auto-run twice within this window. Prevents consolidation churn on rapid session starts. |
| `dedup_similarity_threshold` | `0.92` | Cosine similarity above which two memories are treated as duplicates and merged. Raise to keep more variations; lower to dedupe more aggressively. |
| `staleness_days` | `30` | Days a memory can sit unaccessed before counting as stale. |
| `staleness_importance_floor` | `0.3` | Stale memories with importance below this floor get pruned. Raise to retain more borderline memories; lower to clean up aggressively. |
| `importance_decay_per_day` | `0.01` | Daily importance reduction for unaccessed memories. Memories regain importance when retrieved. Keeps the graph weighted toward live knowledge. |
| `max_memories` | `5000` | Hard cap. When exceeded, lowest-importance memories are pruned first. Raise for long-running projects with many decisions to track. |
| `briefing_limit` | `15` | Top-N memories injected into the session-start briefing. Higher = more context from prior sessions, more startup tokens. |

---

## `[memory.auto_capture]`

Regex-based memory detection at every turn boundary.

```toml
[memory.auto_capture]
enabled = true
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Detect "I'll remember that…", "store this…", and similar phrases in user messages, automatically calling `memory_store` for them. Disable if you want explicit `/memory store` invocations only. |

---

## `[memory.auto_extraction]`

LLM-driven structured fact extraction every N turns.

```toml
[memory.auto_extraction]
enabled = true
every_n_turns = 5
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Run an extraction agent in the background every N turns, pulling structured facts (entities, relationships, claims) into the memory graph. |
| `every_n_turns` | `5` | Frequency. Lower = more granular memory accrual, more background tokens. Higher = lighter cost. `5` is a good balance for typical 30-turn sessions. |

---

## `[learning.*]`

The 8 learning subsystems. Each is an independent toggle.

```toml
[learning.sona]            # Self-Organizing Network Architecture
enabled = true

[learning.provenance]      # L-Score system
enabled = true

[learning.desc]            # Detailed Episodic Storage and Compression
enabled = true

[learning.causal_memory]   # directed hypergraph of cause-effect
enabled = true

[learning.shadow_vector]   # contradiction detection
enabled = true

[learning.reasoning_bank]  # 12 reasoning modes + Hybrid + PatternMatch
enabled = true
```

Each subsystem only takes resources (memory, embedding compute, occasional LLM calls) when enabled. Disable individual subsystems if:
- You don't trust a particular subsystem's outputs
- Memory-graph cost is high
- You want to A/B test the agent's behavior with one subsystem off

See [Learning systems architecture](../architecture/learning-systems.md) for what each subsystem does.

---

## `[learning.gnn]`

Graph attention network for embedding enhancement. Faithful port of root archon's TS GNN.

```toml
[learning.gnn]
enabled = true
input_dim = 1536
output_dim = 1536
num_layers = 3
attention_heads = 12
max_nodes = 50
use_residual = true
use_layer_norm = true
activation = "relu"
weight_seed = 0
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Master switch. When on, embeddings flow through the GNN before being used for retrieval. |
| `input_dim` | `1536` | Embedding input dimension. Matches OpenAI embedding size. Local fastembed (768-dim) is auto-projected to 1536. |
| `output_dim` | `1536` | Round-trip preserves dimensionality so enhanced vectors can replace originals in-place in the vector store. Don't change unless you're rebuilding the whole embedding pipeline. |
| `num_layers` | `3` | 3-layer architecture: 1536 → 1024 (compress) → 1280 (expand) → 1536 (restore). Hardcoded to 3 in current code; future: configurable depth. |
| `attention_heads` | `12` | Multi-head attention heads per layer. Higher = finer-grained attention patterns at the cost of compute. |
| `max_nodes` | `50` | Graph pruning cap. Larger graphs are pruned to top-N nodes by edge weight before attention runs. |
| `use_residual` | `true` | Residual connections where input/output dims match. Improves gradient flow during training. |
| `use_layer_norm` | `true` | Layer norm after residual on each layer. Stabilizes activations. |
| `activation` | `"relu"` | One of: `"relu"`, `"leaky_relu"`, `"tanh"`, `"sigmoid"`. ReLU is standard; switch only for experimentation. |
| `weight_seed` | `0` | RNG seed for reproducible weight init. `0` = auto (timestamp-derived per run). Set to a fixed integer for reproducible training across runs. |

---

## `[learning.gnn.training]`

Hyperparameters used by manual training (`/learning-status retrain`) and the auto-trainer.

```toml
[learning.gnn.training]
learning_rate = 0.001
batch_size = 32
max_epochs = 10
early_stopping_patience = 3
validation_split = 0.2
ewc_lambda = 0.1
margin = 0.5
max_gradient_norm = 1.0
max_triplets_per_run = 256
max_runtime_ms = 300000
```

| Field | Default | What / Why |
|---|---|---|
| `learning_rate` | `0.001` | Adam learning rate. Standard for fine-tuning embedding networks. Raise for faster convergence on stable data; lower if loss is jumping around. |
| `batch_size` | `32` | Training batch size. Affects memory pressure during training. Lower for tight machines. |
| `max_epochs` | `10` | Hard cap on training epochs. Real-world training usually stops earlier via `early_stopping_patience`. Raise if validation loss is still improving at the cap. |
| `early_stopping_patience` | `3` | Stop training when validation loss hasn't improved for N consecutive epochs. Lower = more aggressive stop (avoids overfit), higher = more thorough convergence. |
| `validation_split` | `0.2` | Fraction of triplets held out for validation. `0.0` disables validation (training-loss-only stopping). |
| `ewc_lambda` | `0.1` | Elastic Weight Consolidation regularization strength. Penalizes drift from prior task knowledge. Higher = more conservative updates. |
| `margin` | `0.5` | Triplet contrastive loss margin. The gap between positive and negative similarities the model must achieve. Larger = more separation, harder to satisfy. |
| `max_gradient_norm` | `1.0` | Global L2 gradient clip threshold. Prevents gradient explosion. Lower if loss diverges; raise only if you see vanishing gradients. |
| `max_triplets_per_run` | `256` | Cap on triplets sampled per training run. Limits training duration; raise for more thorough updates if your machine can handle it. |
| `max_runtime_ms` | `300000` (5 min) | Wall-clock cap per training run. Training stops at this point regardless of progress. |

---

## `[learning.gnn.auto_trainer]`

Background auto-retraining.

```toml
[learning.gnn.auto_trainer]
enabled = false
min_throttle_ms = 3600000     # 1 hour
trigger_new_memories = 50
trigger_elapsed_ms = 21600000 # 6 hours
trigger_corrections = 5
first_run_threshold = 100
max_runtime_ms = 300000       # 5 minutes
tick_interval_ms = 60000
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `false` | OFF by default. Set `true` to let the GNN retrain itself in the background as memories accrue. Off means the GNN trains only on explicit `/learning-status retrain` commands. |
| `min_throttle_ms` | `3600000` (1h) | Minimum gap between training runs. Prevents thrashing on rapid memory churn. Lower for very active sessions. |
| `trigger_new_memories` | `50` | Fire training when N new memories have accrued since the last run. |
| `trigger_elapsed_ms` | `21600000` (6h) | Fire training when this much wall time has elapsed regardless of memory activity. |
| `trigger_corrections` | `5` | Fire training when N user corrections have been recorded. Corrections are the strongest training signal. |
| `first_run_threshold` | `100` | At session startup, if the existing memory count exceeds this, kick off an immediate training run. Lets you bootstrap a fresh archon-cli installation against a populated memory graph. |
| `max_runtime_ms` | `300000` (5 min) | Wall-clock cap per run. Same semantics as in `[learning.gnn.training]`. |
| `tick_interval_ms` | `60000` (1 min) | Background poll interval. The auto-trainer checks trigger conditions this often. Lower = more responsive triggers, higher CPU; higher = lazier, lower CPU. |

Triggers are OR-combined — ANY one firing is enough. The throttle gates ALL of them.

---

## `[learning.reflexion]`

N-attempt retry loop with self-critique on failed agent dispatch.

```toml
[learning.reflexion]
enabled = true
max_per_agent = 3
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | When an agent's task fails, capture the failure, generate a self-critique, retry up to `max_per_agent` times with the critique injected into context. |
| `max_per_agent` | `3` | Max retry attempts per agent invocation. Higher = more chance to recover from transient failures, more cost. `3` is a good balance. |

---

## `[cost]`

Spending guardrails (USD).

```toml
[cost]
warn_threshold = 30.0
hard_limit = 0.0
```

| Field | Default | What / Why |
|---|---|---|
| `warn_threshold` | `30.0` | Print a warning when session cost crosses this amount. Doesn't stop the session — just makes you aware. |
| `hard_limit` | `0.0` | Refuse new turns when session cost reaches this. `0.0` disables. Set to a real number for unattended pipelines or to enforce per-session budgets. |

Per-invocation override: `--max-budget-usd <AMOUNT>`.

---

## `[logging]`

Log file rotation.

```toml
[logging]
level = "info"
max_files = 50
max_file_size_mb = 10
```

| Field | Default | What / Why |
|---|---|---|
| `level` | `"info"` | Log level: `"trace"` (everything), `"debug"`, `"info"`, `"warn"`, `"error"`. `info` is fine for normal use; use `debug` or `trace` when investigating issues. |
| `max_files` | `50` | Maximum rotated log files retained. After 50, oldest is deleted. |
| `max_file_size_mb` | `10` | Per-file rotation size. When a log hits 10 MB, it rotates and a new file starts. |

Override level at runtime: `archon --debug api,llm,memory`, `RUST_LOG=archon=trace archon`.

---

## `[session]`

Session persistence.

```toml
[session]
auto_resume = false
# db_path = "/custom/sessions.db"
```

| Field | Default | What / Why |
|---|---|---|
| `auto_resume` | `false` | When `true`, automatically resume the most recent session in the current working directory at startup. Set `false` to start fresh every time. Manual resume always works via `archon -c` or `archon --resume <id>`. |
| `db_path` | unset | Override session DB path. Default: `~/.archon/sessions/`. |

---

## `[checkpoint]`

File snapshots for safe rollback.

```toml
[checkpoint]
enabled = true
max_checkpoints = 10
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Snapshot every file the agent modifies, keyed by turn number. Enables `/restore`, `/rewind`, `/undo`. |
| `max_checkpoints` | `10` | Per-file checkpoint retention. After 10, oldest is rotated. Raise for long sessions where you need deeper undo history. |

---

## `[tui]`

Terminal UI behavior.

```toml
[tui]
vim_mode = false
verbose = true
# theme = "intj"
```

| Field | Default | What / Why |
|---|---|---|
| `vim_mode` | `false` | Vim modal-input keybindings. **Default OFF.** Enable explicitly if you want vim normal/insert mode. Toggle in-session via `/vim`. |
| `verbose` | `true` | Show full turn-by-turn output (tool calls, thinking blocks) in the TUI. `Ctrl+V` toggles at runtime. Disable for cleaner UI when you only care about final answers. |
| `theme` | unset (uses personality.type) | Named color theme. Built-ins: `intj`, `intp`, `dark`, `light`, `ocean`, `fire`, `forest`, `mono`, `daltonized`, `auto`. When unset, the theme is auto-derived from `[personality] type`. |

See [TUI customization](../operations/tui-customization.md) for the full theme catalog and vim keybindings.

---

## `[update]`

Self-update channel.

```toml
[update]
channel = "stable"
auto_check = true
check_interval_hours = 24
```

| Field | Default | What / Why |
|---|---|---|
| `channel` | `"stable"` | Release channel: `"stable"` (only tagged releases), `"beta"` (release candidates). Use `beta` to get fixes earlier at the cost of less stability. |
| `auto_check` | `true` | Check GitHub Releases on startup. Disable to suppress update notifications. |
| `check_interval_hours` | `24` | Throttle. Won't re-check within this window. |

---

## `[remote]` and `[remote.ssh]`

SSH-based remote agent.

```toml
[remote]
sync_mode = "manual"

[remote.ssh]
host = ""
port = 22
user = ""
agent_forwarding = false
# key_file = "~/.ssh/id_ed25519"
```

| Field | Default | What / Why |
|---|---|---|
| `[remote] sync_mode` | `"manual"` | `"manual"` syncs working tree only on explicit command. `"auto"` syncs continuously (more bandwidth, more responsive). |
| `[remote.ssh] host` | `""` | SSH target hostname. Required for `archon remote ssh`. |
| `[remote.ssh] port` | `22` | SSH port. |
| `[remote.ssh] user` | `""` | SSH username. Empty = system default (current user). |
| `[remote.ssh] key_file` | unset | Path to private key. Default: SSH agent or `~/.ssh/id_*`. |
| `[remote.ssh] agent_forwarding` | `false` | Forward `$SSH_AUTH_SOCK` to the remote. Enable if the remote needs your local SSH keys (e.g., to pull from private git repos). |

---

## `[ws_remote]`

WebSocket server for browser/IDE connections.

```toml
[ws_remote]
port = 8420
# tls_cert = "/path/to/cert.pem"
# tls_key  = "/path/to/key.pem"
```

| Field | Default | What / Why |
|---|---|---|
| `port` | `8420` | TCP port for `archon serve`. |
| `tls_cert` / `tls_key` | unset | TLS material. When set, `archon serve` listens on `wss://` instead of `ws://`. Use for any internet-exposed deployment; not needed for `127.0.0.1`. |

---

## `[orchestrator]`

Multi-agent team coordination.

```toml
[orchestrator]
max_concurrent = 4
timeout_secs = 300
max_retries = 2
```

| Field | Default | What / Why |
|---|---|---|
| `max_concurrent` | `4` | Maximum concurrent agents in a parallel team. Limits memory pressure during fan-out. |
| `timeout_secs` | `300` (5 min) | Per-agent timeout in a team run. Raise for slow tasks; lower to fail fast. |
| `max_retries` | `2` | Retry attempts when a team agent fails. |

---

## `[voice]`

Speech-to-text input. Optional.

```toml
[voice]
enabled = false
device = "default"
vad_threshold = 0.02
stt_provider = "openai"
# stt_api_key = "..."   # prefer config.local.toml for secrets
stt_url = "https://api.openai.com"
hotkey = "ctrl+shift+v"
toggle_mode = false
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `false` | Spawn voice capture → STT pipeline. OFF by default — typing is the primary input. |
| `device` | `"default"` | Audio input device name. `"default"` = system default mic. |
| `vad_threshold` | `0.02` | Voice activity detection RMS floor. Higher = stricter (suppresses ambient noise but may clip soft speech); lower = more permissive. |
| `stt_provider` | `"openai"` | STT backend: `"openai"` (Whisper API), `"local"` (whisper.cpp server), `"mock"` (no-op for testing). |
| `stt_api_key` | `""` | OpenAI API key for Whisper. Prefer setting `OPENAI_API_KEY` env var or putting it in `config.local.toml`. |
| `stt_url` | `"https://api.openai.com"` | API endpoint. Override for local whisper.cpp HTTP server. |
| `hotkey` | `"ctrl+shift+v"` | TUI push-to-record (or toggle, depending on `toggle_mode`) hotkey. |
| `toggle_mode` | `false` | `false` = push-to-talk (hold hotkey while speaking, max 2s window). `true` = toggle (press once to start, again to stop). |

---

## `[web]`

Browser-based UI.

```toml
[web]
port = 8421
bind_address = "127.0.0.1"
open_browser = true
```

| Field | Default | What / Why |
|---|---|---|
| `port` | `8421` | TCP port for `archon web`. |
| `bind_address` | `"127.0.0.1"` | Bind address. `"127.0.0.1"` = local only (safe default). `"0.0.0.0"` = network-accessible (only if you also configured TLS + auth). |
| `open_browser` | `true` | Auto-open browser to the web UI URL on `archon web`. Disable for headless / CI deployments. |

---

## Authentication & secrets

Credentials are NOT in `config.toml`. archon-cli reads them in this order:

1. `~/.config/archon/oauth.json` (from `archon login` PKCE flow)
2. `ARCHON_OAUTH_TOKEN` / `ANTHROPIC_AUTH_TOKEN` env vars
3. `ANTHROPIC_API_KEY` / `ARCHON_API_KEY` env vars

If you must pin credentials in TOML, use `<workdir>/.archon/config.local.toml` (gitignored by convention). Never commit secrets to `config.toml` or `.archon/config.toml`.

See [env-vars](env-vars.md) for the full credential resolution.

---

## Local LLMs and proxies

archon-cli speaks to any Anthropic-compatible endpoint via `ANTHROPIC_BASE_URL` or `[api] base_url`.

### LiteLLM

```bash
pip install litellm
litellm --model ollama/llama3 --port 4000
```

```toml
[api]
base_url = "http://localhost:4000/v1/messages"
```

Or set per-invocation:
```bash
ANTHROPIC_BASE_URL=http://localhost:4000/v1/messages archon
```

### Beta header validation

On first startup, archon sends a cheap probe (Haiku, 1 token) to validate which `anthropic-beta` headers the endpoint accepts, then strips any rejected. Probed headers cache at `~/.local/share/archon/identity-cache.json`. Run `/refresh-identity` to clear and re-probe — useful when switching endpoints.

---

## Settings layers via CLI

```bash
archon --settings ~/.config/archon/strict.toml         # Add an overlay layer
archon --setting-sources user,project,local            # Restrict which layers load
```

Use `--setting-sources` to debug "where does this value come from?" — restrict to one layer at a time.

---

## Recipes

**"I want a strict review session — read-only, no network, hard cost cap."**
```toml
[permissions]
mode = "plan"
sandbox = true
always_deny = ["WebFetch:*", "RemoteTrigger:*"]

[cost]
hard_limit = 5.0

[memory.auto_capture]
enabled = false  # don't pollute memory graph during review

[memory.auto_extraction]
enabled = false
```

**"I want fastest, cheapest interactive use — Haiku, no extras."**
```toml
[api]
default_model = "claude-haiku-4-5-20251001"
default_effort = "low"
thinking_budget = 0

[consciousness]
inner_voice = false

[memory.auto_extraction]
enabled = false

[learning.gnn]
enabled = false   # skip embedding enhancement
```

**"I want autonomous agents in a long-running session — let learning compound."**
```toml
[learning.gnn.auto_trainer]
enabled = true              # background retraining on
trigger_new_memories = 25   # fire more often
trigger_elapsed_ms = 7200000  # 2h instead of 6h

[memory.garden]
min_hours_between_runs = 6  # consolidate more frequently
max_memories = 20000        # bigger graph

[session]
auto_resume = true
```

**"I'm running archon-cli for a team — server with auth, TLS, public endpoint."**
```toml
[ws_remote]
port = 8420
tls_cert = "/etc/letsencrypt/live/archon.example.com/fullchain.pem"
tls_key  = "/etc/letsencrypt/live/archon.example.com/privkey.pem"

[web]
bind_address = "0.0.0.0"
open_browser = false

[permissions]
mode = "plan"               # remote sessions are read-only by default
sandbox = true
```

---

## See also

- [Environment variables](env-vars.md) — `ARCHON_*` and `ANTHROPIC_*` overrides
- [CLI flags](cli-flags.md) — flag-based overrides at runtime
- [Permissions](permissions.md) — `[permissions]` deep dive
- [Identity & spoofing](../integrations/identity-spoofing.md) — `[identity]` deep dive
- [Learning systems](../architecture/learning-systems.md) — `[learning.*]` deep dive
- [TUI customization](../operations/tui-customization.md) — themes, vim mode, keybindings
- [Cost, effort, fast mode](../operations/cost-effort.md) — `[cost]`, `[api]`, `--fast`
- [Memory garden](../operations/session-management.md) — `[memory.garden]` operational details
