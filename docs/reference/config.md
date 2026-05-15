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
- [`[providers.openai-codex]`](#providersopenai-codex) — Codex OAuth spoof manifest controls
- [`[identity]`](#identity) — Claude Code spoofing
- [`[personality]`](#personality) — agent personality profile
- [`[consciousness]`](#consciousness) — inner voice and rule engine
- [`[tools]`](#tools) — tool execution defaults
- [Subagent turn limits](#subagent-turn-limits) — operator-only subagent turn caps
- [`[permissions]`](#permissions) — tool gating
- [`[sandbox]`](#sandbox) — Bash isolation backends
- [`[context]`](#context) — compaction and prompt cache
- [`[memory]`](#memory) — memory graph backbone
- [`[memory.garden]`](#memorygarden) — background consolidation
- [`[memory.auto_capture]`](#memoryauto_capture) — regex memory detection
- [`[memory.auto_extraction]`](#memoryauto_extraction) — LLM fact extraction
- [`[learning.*]`](#learning) — 8 learning subsystems
- [`[learning.agent_evolution]`](#learningagent_evolution) — governed profile overlay activation
- [`[learning.gnn]`](#learninggnn) — GNN model
- [`[learning.gnn.training]`](#learninggnntraining) — GNN training hyperparams
- [`[learning.gnn.auto_trainer]`](#learninggnnauto_trainer) — background retraining
- [`[learning.world_model]`](#learningworld_model) — local trace world model
- [`[learning.reasoning_quality]`](#learningreasoning_quality) — visible claim/evidence events
- [`[learning.session_briefing]`](#learningsession_briefing) — proactive session-start briefing
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
- [Workspace policy](#workspace-policy) — VLM, retrieval, game-theory, learning gates
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
# provider = "openai-codex" - Codex OAuth provider
# [llm.openai]   - alternative LLM provider for chat
# [llm.bedrock]  - AWS Bedrock
# [llm.vertex]   - GCP Vertex
# [llm.local]    - local model server
```

| Field | Default | What / Why |
|---|---|---|
| `provider` | `"anthropic"` | Active LLM provider. Use `"openai-codex"` for ChatGPT subscription OAuth, `"openai"` for API-key OpenAI routing, or the other listed providers for Bedrock, Vertex, and local servers. Anthropic is the default because archon-cli uses Claude features heavily. |

Sub-tables `[llm.openai]`, `[llm.bedrock]`, `[llm.vertex]`, `[llm.local]` configure each provider. Most users only need `[llm.openai]` if using OpenAI as primary or for embeddings — see [env-vars](env-vars.md#resolution-order-for-openai-key).

`archon self retrospective <session-id> --analyzer hybrid|llm` uses the same
active provider through the shared `LlmProvider` path. There is deliberately no
separate `[self_calibration]` provider block: switch retrospectives between
Anthropic, Codex OAuth, OpenAI-compatible, or local providers by changing
`[llm].provider` and the matching provider credentials.

---

## `[providers.openai-codex]`

Codex provider compatibility settings for ChatGPT subscription OAuth.

```toml
[providers.openai-codex]
enabled = true
runtime = "direct"
direct_fallback = false
app_server_transport = "websocket"
app_server_url = ""
app_server_command = "codex"
app_server_args = ["app-server"]
app_server_discovery_timeout_ms = 2500
app_server_model_catalog = ["gpt-5.5", "gpt-5.4"]

[providers.openai-codex.spoof]
# originator  = "openclaw"
# user_agent  = "openclaw/2026.5.1-beta.2"
# client_id   = "app_..."
# openai_beta = "responses=experimental"

[providers.openai-codex.spoof.extra_headers]
# "x-codex-version" = "0.34.1"

[providers.openai-codex.manifest]
# fetch_url   = "https://raw.githubusercontent.com/ste-bah/archon-cli/main/crates/archon-llm/resources/codex-compat.json"
# ttl_seconds = 21600
# cache_dir   = "~/.archon/cache/codex-compat"
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Master switch for resolving Codex OAuth credentials and spoof metadata. `ARCHON_CODEX_DISABLED=1` disables it at runtime. |
| `runtime` | `"direct"` | Codex runtime strategy. `"direct"` preserves Archon's current direct backend. `"auto"` selects app-server when configured and can fall back to direct only when `direct_fallback=true`. `"app_server"` requires a configured JSON-RPC app-server transport. |
| `direct_fallback` | `false` | Explicit policy switch for auto-mode app-server to direct fallback. Keeping this false prevents silent strategy changes. |
| `app_server_transport` | `"websocket"` | Codex app-server transport target. `"websocket"` uses `app_server_url`; `"stdio"` uses `app_server_command` and `app_server_args`. Both are JSON-RPC transports. |
| `app_server_url` | unset | Optional Codex app-server WebSocket endpoint. `ARCHON_CODEX_APP_SERVER_URL` overrides this value for local diagnostics. `ws`, `wss`, `http`, and `https` are accepted for compatibility; endpoint paths and query strings are redacted in status and persisted snapshots. |
| `app_server_command` | `"codex"` | Command used when `app_server_transport = "stdio"`. Status redacts this to the executable basename. |
| `app_server_args` | `["app-server"]` | Arguments used when spawning stdio app-server transport. Arguments are counted but not persisted verbatim in status snapshots. |
| `app_server_discovery_timeout_ms` | `2500` | App-server JSON-RPC request and idle timeout budget for discovery/startup/status decisions. |
| `app_server_model_catalog` | `["gpt-5.5", "gpt-5.4"]` | Fallback Codex app-server model catalog used when live model discovery is unavailable. |
| `spoof.originator` | bundled manifest | Product originator header used by the Codex compatibility layer. Leave unset unless a known-good manifest update requires an override. |
| `spoof.user_agent` | bundled manifest | User agent used for Codex requests. Archon rejects `ChatGPT-*`, `ChatGPT/`, `OpenAI-*`, and `OpenAI/` values to avoid impersonating OpenAI products. |
| `spoof.client_id` | bundled manifest | OAuth client id. Override only for diagnostics or when the manifest is stale. |
| `spoof.openai_beta` | bundled manifest | Optional `OpenAI-Beta` header value for the Codex responses endpoint. |
| `spoof.extra_headers` | `{}` | Additional compatibility headers. Never put secrets here. |
| `manifest.fetch_url` | bundled GitHub raw URL | Optional remote manifest source for refreshed spoof metadata. |
| `manifest.ttl_seconds` | `21600` | How long fetched manifest metadata is cached. |
| `manifest.cache_dir` | `~/.archon/cache/codex-compat` | Cache directory for fetched manifests. |

Set `[llm].provider = "openai-codex"` to use Codex across chat, TUI, subagents, pipelines, and game-theory runs. Credentials are stored by `archon auth login --provider openai-codex`.

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
| `mode` | `"spoof"` | `"spoof"` mimics Claude Code (User-Agent, billing header, beta headers, system-prompt prelude). `"clean"` identifies as archon-cli. Spoof mode lets archon-cli use Claude.ai subscriptions transparently because Anthropic's billing keys off the spoofed identity. Switch to `"clean"` if you're on a non-Claude proxy that doesn't care or breaks on spoof headers. |
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

### Subagent Turn Limits

`max_turns` is not exposed to the model via the `Agent` / `TaskCreate` tool schemas. Subagents default to `DEFAULT_MAX_TURNS = 100_000`, which is effectively unlimited for normal work. Operators can still set a cap explicitly per custom agent in `meta.json`, through built-in definitions such as the bounded `fork` agent, or with the CLI `--max-turns` flag in headless mode. The LLM cannot bound a subagent by adding `max_turns` to its own tool input.

---

## `[permissions]`

Tool gating policy. See [Permissions reference](permissions.md) for full details.

```toml
[permissions]
mode = "default"
allow_paths = ["/home/user/project"]
deny_paths = ["/etc", "/.ssh"]

[[permissions.always_allow]]
tool = "Bash"
pattern = "git status"

[[permissions.always_deny]]
tool = "Bash"
pattern = "rm -rf"

[[permissions.always_ask]]
tool = "Bash"
pattern = "git push"
```

| Field | What / Why |
|---|---|
| `mode` | One of 6 canonical modes: `default` (prompt for risky), `acceptEdits` (auto-allow file edits), `plan` (read-only), `auto` (heuristic), `dontAsk` (allow except deny rules), `bypassPermissions` (skip most prompts). Legacy aliases: `ask` -> `default`, `yolo` -> `bypassPermissions`. |
| `always_allow` | Array of `{ tool, pattern }` tables auto-approved regardless of mode. Patterns are prefix/glob-style matchers. |
| `always_deny` | Array of `{ tool, pattern }` tables always denied, even in permissive modes. Use for hard guards (`rm -rf`, secrets paths, etc.). |
| `always_ask` | Array of `{ tool, pattern }` tables that force a prompt even in auto/dontAsk modes. Useful for operations you want eyes on. |
| `allow_paths` | Restrict file tools to these paths. Empty list = no restriction. |
| `deny_paths` | Block file tools from these paths. Takes precedence over `allow_paths`. |

Rule precedence: `always_deny` > `always_ask` > `always_allow` > mode default.

---

## `[sandbox]`

Bash execution isolation. The default is host execution with permission
preflight only. Set a real backend plus its matching `enabled = true` section to
route Bash through Docker, SSH, or OpenShell.

Host runtime dependencies stay outside `config.toml`. Install optional Docker
and OpenShell tooling with `scripts/install-system-deps.sh --with-docker`,
`--with-openshell`, or `--with-sandbox`, then use
`archon sandbox doctor --backend <name>` before enabling a backend.

```toml
[sandbox]
backend = "disabled"       # "disabled" | "logical" | "docker" | "ssh" | "openshell"
mode = "risky"             # "risky" | "shell" route shell only; "all" is strict
scope = "session"          # "session" | "turn" | "tool"
workspace_access = "ro"    # "ro" | "rw" | "scratch"

[sandbox.docker]
enabled = false
binary = "docker"
image = "ubuntu:24.04"
network = "disabled"       # "disabled" | "limited" | "enabled"
memory_limit = "2g"
cpu_limit = "2"
writable_paths = []
env_allowlist = []
privileged = false
mount_docker_socket = false
mount_home = false

[sandbox.ssh]
enabled = false
binary = "ssh"
# host = "sandbox.example"
# user = "archon"
port = 22
# key_file = "~/.ssh/id_ed25519"
workspace_mode = "remote"  # "remote" | "mirror"
# remote_workdir = "/srv/archon/workspace"
host_key_checking = true
host_shell_fallback = false

[sandbox.openshell]
enabled = false
binary = "openshell"
workspace_mode = "upload"  # "upload" | "remote" | "mirror"
gateway = "openshell"
# remote_workdir = "/sandbox"
# policy = "locked-down"
providers = []
gpu = false
provider_injection = false
host_shell_fallback = false
```

| Field | Default | What / Why |
|---|---|---|
| `backend` | `"disabled"` | Selects the sandbox route for Bash. `logical` is a policy gate only; `docker`, `ssh`, and `openshell` are real isolation backends when their section is enabled. |
| `mode` | `"risky"` | Chooses how broadly a real backend applies. `risky` and `shell` route Bash/Shell through Docker/SSH/OpenShell while normal host-side tools such as `Write`, `Edit`, and `WebFetch` continue through permission preflight. `all` is strict and blocks unsupported host-side mutation, network, and agent-spawn tools. |
| `scope` | `"session"` | Backend lifecycle hint for session-, turn-, or tool-scoped isolation. |
| `workspace_access` | `"ro"` | Workspace mount policy. `rw` allows writes; `scratch` keeps the workspace read-only and provides ephemeral scratch space where supported. |
| `sandbox.docker.*` | see template | Docker binary/image, resource limits, network mode, writable paths, and mount hardening. Docker socket, home mount, and privileged mode default to off. |
| `sandbox.ssh.*` | see template | Remote SSH execution target. Remote mode requires `remote_workdir`; mirror mode assumes the same workspace path exists remotely. Host shell fallback is off. |
| `sandbox.openshell.*` | see template | OpenShell execution target. Provider injection and host shell fallback stay off by default so Claude Code spoofing and provider credentials remain host-side. |

See [Sandboxing](../security/sandboxing.md), [Docker sandbox](../security/docker-sandbox.md), [SSH sandbox](../security/ssh-sandbox.md), [OpenShell sandbox](../security/openshell-sandbox.md), and [Tool preflight](../security/tool-preflight.md).

---

## `[context]`

Context-window management.

```toml
[context]
compact_threshold = 0.8
preflight_safety_margin = 0.05
output_reserve_tokens = 8192
preserve_recent_turns = 3
manual_compact_force_strategy = "micro"
# rate_limit_pressure_tokens = 250000
# rate_limit_pressure_body_bytes = 1000000
# large_request_retry_body_bytes = 1000000
prompt_cache = true
prompt_cache_mode = "explicit"
prompt_cache_ttl = "5m"
prompt_cache_conversation = false
```

| Field | Default | What / Why |
|---|---|---|
| `compact_threshold` | `0.8` | Trigger automatic compaction when context fill ratio crosses 80%. Lower = compact earlier (loses old detail sooner but stays responsive); higher = keep raw turns longer (more cost, smaller context budget for new turns). |
| `preflight_safety_margin` | `0.05` | Starts proactive compaction before the exact threshold to absorb estimator/provider tokenizer drift. |
| `context_window_override` | unset | Emergency per-session context-window override. Prefer `context.toml` for provider/model facts. |
| `output_reserve_tokens` | `8192` | Reserved headroom for the next assistant response; prompt budgeting subtracts this before applying the threshold. |
| `preserve_recent_turns` | `3` | Always keep the last N turns verbatim across compaction. Keeps the immediate working context intact while older turns get summarized. |
| `manual_compact_force_strategy` | `"micro"` | Strategy used by `/compact force` when the session is below the normal threshold. Bare `/compact` remains thresholded and behaves like `/compact auto`. |
| `rate_limit_pressure_tokens` | unset | Optional proactive trigger for very large per-request token pressure. Leave unset unless calibrated from `/context`/logs after fixed prompt overhead is measured. |
| `rate_limit_pressure_body_bytes` | unset | Optional proactive trigger for serialized request-body pressure. Set above the measured first-turn body size so fresh sessions do not immediately compact. |
| `large_request_retry_body_bytes` | unset (`1_000_000` runtime fallback) | Requests at or above this serialized-body size do not blindly repeat identical provider 429 retries; Archon first tries one scoped compaction where supported. |
| `prompt_cache` | `true` | Set Anthropic's `cache_control` flag on static blocks (system prompt, tool catalog, memory briefing). Cache hits are billed at a fraction of input cost; reduces session cost dramatically over long conversations. Disable only if you're hitting cache-correctness bugs. |
| `prompt_cache_mode` | `"explicit"` | `"explicit"` uses cache breakpoints on stable prompt blocks; `"automatic"` strips explicit hints; `"hybrid"` keeps explicit hints and leaves room for provider-specific automatic caching where supported. |
| `prompt_cache_ttl` | `"5m"` | Cache lifetime hint for providers that support TTLs. Set `"1h"` only when you accept the higher cache-write cost. |
| `prompt_cache_conversation` | `false` | Reserved for provider-supported conversation caching. Unsupported providers ignore Anthropic cache hints. |

Model context-window limits live in a separate catalog, not in provider code.
Archon ships a bundled `context.toml` for Claude, Codex auth, and common
OpenAI defaults, then overlays these optional files in order. Later files win:

```text
~/.config/archon/context.toml
~/.archon/context.toml
<workspace>/.archon/context.toml
<workspace>/.archon/context.local.toml
```

This file is intentionally separate from `config.toml` because model limits are
provider/model facts, not session preferences. Use it for third-party routers,
local models, private deployments, or temporary corrections while upstream
model limits change.

Each entry is grouped by provider and model id:

```toml
[providers.openai-codex.models."gpt-5.5"]
context_window = 1_050_000
runtime_context_budget = 272_000
max_output_tokens = 128_000
source = "operator"
```

For Codex/OpenAI subscription routes, `context_window` is the native model
window while `runtime_context_budget` is the safer request-pressure budget used
for compaction and status. Local, OpenAI-compatible, and third-party providers
can omit `runtime_context_budget` when the native window is also the safe
runtime budget.

Example override for a local or third-party model:

```toml
[providers.local.models."qwen3-custom"]
context_window = 131_072
max_output_tokens = 16_384
source = "operator"
```

Conditional variants can raise the limit only when a provider identity or beta
header is active. Paid-plan Claude Sonnet 4.6 and Opus 4.7 base entries are
1M-token entries; Claude Code identity variants are kept explicit too:

```toml
[providers.anthropic.models."claude-opus-4-7"]
context_window = 1_000_000

[providers.anthropic.models."claude-opus-4-7".variants.claude_code]
context_window = 1_000_000
requires_identity = "spoof"
```

Unknown models resolve through the `fallback` source to an unknown limit, shown in the TUI as `ctx used/?`.
Archon skips proactive threshold compaction when the limit is unknown, but still
reacts to provider context-window errors.

Use `[context].max_tokens` only as an emergency per-session override; prefer
`context.toml` when the limit belongs to a provider/model.

Resolution precedence is: `context_window_override`, user/project
`context.toml`, bundled catalog, provider metadata, then unknown fallback. The
TUI and `/context` command show the active source label.

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

## `[learning.agent_evolution]`

Runtime activation for governed agent profile overlays. Profile versions,
proposals, shadow evaluations, reports, digest claims, and memory-promotion
candidates are stored in CozoDB regardless of this flag. This switch only
controls whether the active profile overlay is applied at runtime.

```toml
[learning.agent_evolution]
active_profile_overlay_enabled = false
```

| Field | Default | What / Why |
|---|---|---|
| `active_profile_overlay_enabled` | `false` | Keeps generated agent-profile overlays opt-in. Leave off while reviewing proposals and shadow results; enable only when you want approved active profiles to influence runtime agent behavior. |

See [Governed agent evolution](../agents/evolution.md) and [Governed agent evolution storage](../learning/governed-agent-evolution.md).

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
triplet_loss_coefficient = 0.1
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
| `triplet_loss_coefficient` | `0.1` | Auxiliary weight for hydrated `archon meaning` triplets. Conservative by default so trajectory-quality training remains primary. |
| `max_gradient_norm` | `1.0` | Global L2 gradient clip threshold. Prevents gradient explosion. Lower if loss diverges; raise only if you see vanishing gradients. |
| `max_triplets_per_run` | `256` | Cap on triplets sampled per training run. Limits training duration; raise for more thorough updates if your machine can handle it. |
| `max_runtime_ms` | `300000` (5 min) | Wall-clock cap per training run. Training stops at this point regardless of progress. |

---

## `[learning.gnn.auto_trainer]`

Background auto-retraining.

```toml
[learning.gnn.auto_trainer]
enabled = true
min_throttle_ms = 3600000     # 1 hour
trigger_new_memories = 20
trigger_elapsed_ms = 21600000 # 6 hours
trigger_corrections = 3
first_run_threshold = 30
max_runtime_ms = 300000       # 5 minutes
tick_interval_ms = 60000
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | ON by default. The 1h throttle and 5min runtime cap keep background training bounded; set `false` to opt out. |
| `min_throttle_ms` | `3600000` (1h) | Minimum gap between training runs. Prevents thrashing on rapid memory churn. Lower for very active sessions. |
| `trigger_new_memories` | `20` | Fire training when N new memories have accrued since the last run. |
| `trigger_elapsed_ms` | `21600000` (6h) | Fire training when this much wall time has elapsed regardless of memory activity. |
| `trigger_corrections` | `3` | Fire training when N user corrections have been recorded. Corrections are the strongest training signal. |
| `first_run_threshold` | `30` | At session startup, if the existing memory count is at least this value, kick off an immediate training run. Lets early workspaces train in the first 2-3 normal sessions. |
| `max_runtime_ms` | `300000` (5 min) | Wall-clock cap per run. Same semantics as in `[learning.gnn.training]`. |
| `tick_interval_ms` | `60000` (1 min) | Background poll interval. The auto-trainer checks trigger conditions this often. Lower = more responsive triggers, higher CPU; higher = lazier, lower CPU. |

Triggers are OR-combined — ANY one firing is enough. The throttle gates ALL of them.

---

## `[learning.world_model]`

Local ME-JEPA-inspired trace model for advisory next-state prediction.

```toml
[learning.world_model]
enabled = true
model_kind = "latent_transition"
auto_promote_advisory = true
require_approval_for_behavior_change = true
state_dim = 384
max_checkpoint_mb = 64
max_prediction_latency_ms = 100
max_counterfactual_actions = 5
store_raw_text = false
include_conversation_turns = true
include_agent_outputs = true

[learning.world_model.embeddings]
source = "local"
provider = "fastembed"
model = "bge-base-en-v1.5"
dimensions = 768
projection_dim = 384
cache_enabled = true
cache_max_mb = 1024
redact_before_embedding = true
allow_third_party = false
external_base_url = ""
external_api_key_env = ""

[learning.world_model.labeler]
analyzer = "hybrid"
llm_enabled = true
max_events_per_prompt = 120
max_prompt_chars = 32000

[learning.world_model.training]
backend = "auto"
allow_cpu_fallback = true
prefer_accelerator = true
precision = "fp32"
max_accelerator_memory_mb = 4096
batch_size = 32
max_epochs = 10
validation_split = 0.2
promotion_min_delta = 0.02
max_runtime_ms = 300000

[learning.world_model.eval]
bootstrap_iterations = 1000
confidence_level = 0.95
parity_precision = "fp32"
parity_min_cosine = 0.95
next_state_baseline_min_delta = 0.10
counterfactual_baseline_min_delta = 0.10
surprise_ks_min_p = 0.05
counterfactual_ndcg_min = 0.60

[learning.world_model.cold_start]
min_rows = 1000
min_sessions = 50
min_observed_days = 7

[learning.world_model.auto_trainer]
enabled = true
min_throttle_ms = 3600000
idle_required_ms = 300000
battery_suspend_below_percent = 30
trigger_new_rows = 100
trigger_surprises = 5
trigger_corrections = 3
trigger_elapsed_ms = 21600000
first_run_threshold = 300
max_runtime_ms = 300000
tick_interval_ms = 60000

[learning.world_model.retention]
jsonl_rotate_mb = 500
raw_retention_days = 90
retain_cozo_summaries = true
retain_checkpoint_count = 5
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Enables local world-model corpus and advisory surfaces. |
| `state_dim` | `384` | Latent state size. Small enough for consumer machines. |
| `max_prediction_latency_ms` | `100` | Advisor budget. Exceeding this must fail open. |
| `embeddings.source` | `"local"` | Local-first embeddings. Third-party embeddings require config and policy. |
| `embeddings.cache_enabled` | `true` | Caches redacted embeddings under `~/.archon/world-model/embeddings/cache`. |
| `embeddings.cache_max_mb` | `1024` | Prunes oldest cache rows after the cache exceeds this size. |
| `embeddings.redact_before_embedding` | `true` | Redacts common email, token, API-key, and long secret shapes before local or external embedding calls. |
| `labeler.analyzer` | `"hybrid"` | `"heuristic"`, `"llm"`, or `"hybrid"`. LLM mode is provider-neutral and requires policy. |
| `training.backend` | `"auto"` | Selects an accelerator only after its tensor self-test probe passes, otherwise CPU if fallback is allowed. |
| `auto_trainer.idle_required_ms` | `300000` | Suspends training while foreground work is active. |
| `retention.jsonl_rotate_mb` | `500` | Rotates raw JSONL ledgers. |
| `retention.raw_retention_days` | `90` | Deletes old raw ledgers, while Cozo summaries remain. |

See [Local world model](../architecture/world-model.md), [world-model backends](world-model-backends.md), and [world-model embeddings](world-model-embeddings.md).

---

## `[learning.reasoning_quality]`

Visible assistant claim/evidence learning signal. This is the canonical store for "claimed before verified", "corrected by user", and "later contradicted by source" events.

```toml
[learning.reasoning_quality]
enabled = true
emit_inline_events = true
post_turn_analysis = true
post_session_analysis = true
shadow_mode_days = 30
apply_trust_updates_after_shadow = true
max_claims_per_turn = 12
max_excerpt_chars = 600
store_raw_text = false
link_user_corrections = true
update_self_trust = true
feed_world_model = true
feed_retrospective = true

[learning.reasoning_quality.critic]
mode = "hybrid"
allow_llm = false
provider = "default"
model = ""
max_tokens = 1200
temperature = 0.0
max_turns_per_session = 50
run_async = true
fallback_to_heuristic = true

[learning.reasoning_quality.critic.budget]
per_session_token_cap = 200000
daily_usd_cap = 10.00
weekly_usd_cap = 50.00
respect_provider_cooldowns = true
emit_cost_events = true
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Enables reasoning-quality storage and command surfaces. |
| `emit_inline_events` | `true` | Emits rows from visible assistant turns as the session runs. |
| `shadow_mode_days` | `30` | Logs trust deltas without applying them until extractor precision is validated. |
| `store_raw_text` | `false` | Stores redacted excerpts and hashes by default; raw text requires policy approval. |
| `feed_world_model` | `true` | Writes reasoning-quality rows into the world-model trace corpus. |
| `critic.allow_llm` | `false` | Enables optional LLM critique through the active `LlmProvider`; policy must also allow it. |
| `critic.provider` | `"default"` | Uses the active provider. Separate critic providers are treated as third-party and policy-gated. |
| `critic.model` | `""` | Empty means use the active session model. |
| `critic.budget.*` | see block | Caps critic token/cost exposure and records cost ledger rows. |

See [Reasoning Quality](../architecture/reasoning-quality.md).

---

## `[learning.session_briefing]`

Controls the proactive first-turn briefing that can combine memory, reasoning warnings, pending behavior proposals, and world-model status.

```toml
[learning.session_briefing]
enabled = true
include_memory = true
include_reasoning_quality = true
include_pending_behaviour_proposals = true
include_world_model = true
max_items = 8
max_chars = 4000
world_model_requires_ready = true
```

| Field | Default | What / Why |
|---|---|---|
| `enabled` | `true` | Allows proactive briefing assembly. |
| `include_reasoning_quality` | `true` | Surfaces recent high-severity reasoning warnings. |
| `include_pending_behaviour_proposals` | `true` | Reminds the operator about governed-learning proposals waiting for review. |
| `include_world_model` | `true` | Includes world-model readiness or advisory status. |
| `max_chars` | `4000` | Hard cap before injection into the first turn. |

Preview with `archon briefing preview --task "..."` or `/briefing preview ...`.

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

Browser-based workbench. The frontend is embedded in the `archon` binary from
`web/dist`; normal users do not need a separate Node/Vite install.

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

Run `archon web` from the project root you want to inspect. For a blank
project, initialise the directory first with `scripts/archon-init.sh`; the web
workbench does not create project scaffolding. See
[Web workbench](../operations/web-workbench.md) for the tab guide and safety
model.

---

## Workspace policy

Runtime gates live in `.archon/policy.toml`, not `config.toml`. The repo and `archon-init.sh` templates include every current policy field:

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
max_cost_usd = 20.0
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

Local VLM requires `policy.docs.vlm.provider = "ollama"` and `policy.workers.vlm = "allow-local"`. Cloud VLM requires `policy.workers.vlm = "allow-cloud"`, `policy.docs.vlm.allow_cloud = true`, and `policy.network.allow_cloud_vlm = true`.

World-model cloud embeddings require both config and policy:
`learning.world_model.embeddings.allow_third_party = true`,
`policy.world_model.allow_third_party_embeddings = true`,
`policy.workers.embedding = "allow-cloud"`, and
`policy.network.default = "allow"`. LLM-assisted world-model labeling requires
`policy.world_model.allow_llm_labeler = true`.

Browser workbench actions require `[policy.web]` gates. The global
`policy.web.allow_mutating_actions` gate must be true, and the matching
action-family gate must also be true. World-model training or promotion
actions additionally require `policy.world_model.allow_behavior_changes =
true`; otherwise the web action evaluator denies the request even if the web
action-family gate is enabled.

Reasoning-quality LLM critique requires both config and policy:
`learning.reasoning_quality.critic.allow_llm = true`,
`policy.reasoning_quality.allow_llm_critic = true`, and
`policy.reasoning_quality.allow_critic_cloud_data_flow = true` when the active
provider is classified as cloud-hosted. Raw text persistence requires
`policy.reasoning_quality.allow_raw_text_storage = true`; otherwise Archon
stores redacted excerpts, hashes, and entity keys only.

PDF ingest uses `[policy.docs.pdf]` to decide whether to extract embedded
images with `pdfimages`, the minimum size filter for icons/decorations, whether
PDF-derived images should receive VLM descriptions, and whether native-text PDFs
should also be rendered page-by-page.

See [Policy](../policy.md) and [VLM Image Descriptions](../integrations/vlm.md) for the full operator guide.

---

## Authentication & secrets

Credentials are NOT in `config.toml`. archon-cli reads Anthropic credentials in this order:

1. `~/.archon/.credentials.json` (from `archon auth login --provider anthropic`)
2. `~/.claude/.credentials.json` (deprecated fallback when the Archon file is absent)
3. `ARCHON_OAUTH_TOKEN` / `ANTHROPIC_AUTH_TOKEN` env vars
4. `ANTHROPIC_API_KEY` / `ARCHON_API_KEY` env vars

If you must pin credentials in TOML, use `<workdir>/.archon/config.local.toml` (gitignored by convention). Never commit secrets to `config.toml` or `.archon/config.toml`.

See [env-vars](env-vars.md) for the full credential resolution.

Gemini VLM credentials are separate: set `GOOGLE_API_KEY` or run
`archon auth login --provider google`, which stores `googleApiKey` in
`~/.archon/.credentials.json` without overwriting Anthropic or Codex entries.

Codex OAuth credentials are also kept out of TOML. Run
`archon auth login --provider openai-codex`, then set
`[llm].provider = "openai-codex"` when you want chat, pipelines, or hybrid
retrospectives to use the Codex provider.

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

[[permissions.always_deny]]
tool = "WebFetch"
pattern = "*"

[[permissions.always_deny]]
tool = "RemoteTrigger"
pattern = "*"

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
