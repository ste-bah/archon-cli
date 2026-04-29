# Configuration

archon-cli reads layered TOML config from (in order of precedence, last wins):

1. `~/.config/archon/config.toml` — user-level
2. `<workdir>/.archon/config.toml` — project-level
3. `<workdir>/.archon/config.local.toml` — local overrides (gitignored)
4. `--settings <PATH>` — CLI overlay
5. Environment variables (see [env-vars](env-vars.md))
6. CLI flags

On first run archon generates a commented config file at `~/.config/archon/config.toml` with all defaults.

## Full schema

```toml
[api]
default_model = "claude-sonnet-4-6"   # Model used for the main agent
thinking_budget = 16384               # Max thinking tokens (extended thinking)
default_effort = "high"               # "low" | "medium" | "high"
max_retries = 3
# base_url = "http://localhost:4000/v1/messages"  # Override (LiteLLM/proxy)

[identity]
mode = "spoof"                        # "spoof" | "native"
spoof_version = "2.1.89"              # Version reported to the API
anti_distillation = false             # Inject anti-distillation field in spoof mode

[personality]
name = "Archon"                       # Shown in TUI header
type = "INTJ"                         # MBTI type, auto-selects theme
enneagram = "4w5"
traits = ["strategic", "direct", "truth-over-comfort"]
communication_style = "terse"         # Injected into system prompt

[consciousness]
inner_voice = true                    # Background monologue before responses
energy_decay_rate = 0.02
persist_personality = true            # Persist InnerVoice + rule scores across sessions
personality_history_limit = 50        # Max personality snapshots to retain
initial_rules = [
    "Always ask before modifying files",
    "Explain reasoning before acting",
    "Never create files unless explicitly requested",
]

[tools]
bash_timeout = 120
bash_max_output = 102400
max_concurrency = 4

[permissions]
mode = "Default"                      # See reference/permissions.md
allow_paths = []
deny_paths = []
sandbox = false                       # Read-only enforcement

[memory]
enabled = true                        # CozoDB memory graph
embedding_provider = "auto"           # "auto" | "fastembed" | "openai"

[memory.garden]
auto_consolidate = true               # Run consolidation on session start
min_hours_between_runs = 24           # Throttle auto-consolidation
dedup_similarity_threshold = 0.92     # Jaccard threshold for deduplication
staleness_days = 30                   # Days without access before stale
staleness_importance_floor = 0.3      # Stale memories below this are pruned
importance_decay_per_day = 0.01       # Daily importance reduction for unaccessed
max_memories = 5000                   # Hard cap (lowest importance pruned)
briefing_limit = 15                   # Top-N memories in session briefing

[context]
compact_threshold = 0.8               # Context fill % that triggers compaction
preserve_recent_turns = 3
prompt_cache = true                   # Anthropic prompt cache on static blocks

[session]
auto_resume = true                    # Resume last session on startup

[logging]
level = "info"                        # trace | debug | info | warn | error
max_files = 50
max_file_size_mb = 10

[cost]
warn_threshold = 30.0                 # Warn when session cost exceeds $N (default 30)
hard_limit = 0.0                      # 0.0 = no hard limit

[ws_remote]
port = 8420                           # archon serve listener port
# tls_cert = "/path/to/cert.pem"
# tls_key = "/path/to/key.pem"

[web]
port = 8421
bind_address = "127.0.0.1"
open_browser = true

[tui]
vim_mode = false                      # Enable vim keybindings

[orchestrator]
max_concurrent = 4                    # Max parallel team agents
timeout_secs = 300
max_retries = 2

[voice]                               # Voice input (optional)
enabled = false
device = "default"
vad_threshold = 0.02
stt_provider = "mock"                 # mock | openai | local
stt_api_key = ""                      # Required for stt_provider = "openai"
stt_url = "http://localhost:9000"     # For local whisper.cpp / server
hotkey = "ctrl+v"                     # TUI push-to-record hotkey
toggle_mode = true                    # true = toggle, false = push-to-talk

[remote.ssh]
agent_forwarding = false              # Try SSH agent even without SSH_AUTH_SOCK

[remote_triggers]
allowed_hosts = []                    # Allow-list for RemoteTrigger tool

# === Learning systems ===

[reasoning_bank]
default_max_results = 5
default_confidence_threshold = 0.7
default_min_l_score = 0.3
enable_trajectory_tracking = true
enable_auto_mode_selection = true
deductive_weight = 1.0
inductive_weight = 1.0
abductive_weight = 1.0
analogical_weight = 1.0
adversarial_weight = 1.0
counterfactual_weight = 1.0
temporal_weight = 1.0
constraint_weight = 1.0
decomposition_weight = 1.0
first_principles_weight = 1.0
causal_weight = 1.0
contextual_weight = 1.0
pattern_weight = 0.5

[reflexion]
enabled = true
max_attempts = 3

[auto_extraction]
enabled = true
min_confidence = 0.6

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
weight_seed = 0                       # 0 = auto (timestamp-derived)

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

[learning.gnn.auto_trainer]
enabled = true
min_throttle_ms = 3600000             # 1 hour
trigger_new_memories = 50
trigger_elapsed_ms = 21600000         # 6 hours
trigger_corrections = 5
first_run_threshold = 100
max_runtime_ms = 300000               # 5 minutes
max_triplets_per_run = 256
```

## OpenAI key (optional)

The `OPENAI_API_KEY` is **not required** for core archon-cli functionality. Anthropic API (Claude) handles all chat, coding, and pipeline operations. OpenAI keys enable three optional features:

| Feature | What it does | Without it |
|---|---|---|
| Memory embeddings | 1536-dim OpenAI embeddings for semantic memory search | Falls back to local fastembed (768-dim BGE-base-en-v1.5, no network calls) |
| LLM provider | Use OpenAI models as primary LLM | Uses Anthropic (default) |
| Voice STT | OpenAI Whisper for speech-to-text | Use `"mock"` or local whisper.cpp server |

**Key resolution order:** `OPENAI_API_KEY` env > `ARCHON_MEMORY_OPENAIKEY` env (memory only) > `llm.openai.api_key` in config.

**Dimension compatibility:** Local 768-dim and OpenAI 1536-dim embeddings coexist. The GNN's `input_projection` layer handles both; vector search uses `min_len` cosine similarity. Local embeddings work end-to-end.

## Local LLMs and proxies

archon points at any Anthropic-compatible endpoint via `ANTHROPIC_BASE_URL` or `[api] base_url`. LiteLLM, Ollama (with Anthropic adapter), and other proxy gateways are supported.

### LiteLLM

```bash
pip install litellm
litellm --model ollama/llama3 --port 4000

ANTHROPIC_BASE_URL=http://localhost:4000/v1/messages archon
```

Or:
```toml
[api]
base_url = "http://localhost:4000/v1/messages"
```

### Beta header validation

On first startup, archon sends a cheap probe request (Haiku, 1 token) to validate which `anthropic-beta` headers the endpoint accepts, then strips any rejected headers. No manual configuration. Run `/refresh-identity` to clear the cache and re-probe.

## Settings layers via CLI

```bash
archon --settings ~/.config/archon/strict.toml         # Add overlay
archon --setting-sources user,project,local            # Restrict layers
```

## See also

- [Environment variables](env-vars.md) — `ARCHON_*` and `ANTHROPIC_*` overrides
- [CLI flags](cli-flags.md) — flag overrides
- [Permissions](permissions.md) — `[permissions]` section deep dive
- [Identity & spoofing](../integrations/identity-spoofing.md) — `[identity]` section
- [Learning systems](../architecture/learning-systems.md) — `[learning.*]` and `[reasoning_bank]` sections
