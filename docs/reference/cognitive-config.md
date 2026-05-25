# Cognitive Configuration

The Cognitive Executive Loop has a runtime config section and a separate policy
gate. Config decides which subsystems are used. Policy decides what the loop is
allowed to do.

## `[learning.cognitive]`

```toml
[learning.cognitive]
enabled = true
max_candidates = 5
trivial_turn_tool_policy = "none"
record_decisions = true
record_reflections = true
use_world_model = true
use_jepa = true
use_reasoning_quality = true
use_self_model = true
max_pipeline_ms = 500
situation_ttl_days = 90
reflection_ttl_days = 180
prediction_ttl_days = 90
ledger_dir = ".archon/cognitive"
```

| Field | Default | Notes |
|---|---|---|
| `enabled` | `false` in schema, `true` in project templates | Master switch. When disabled, the loop records no decisions and foreground work continues normally |
| `max_candidates` | `5` | Clamped to 2-5 |
| `trivial_turn_tool_policy` | `"none"` | Suppresses needless tools for greetings/trivial turns |
| `record_decisions` | `true` | Writes compact decision records |
| `record_reflections` | `true` | Writes compact outcome lessons |
| `use_world_model` | `false` in schema, template enables it | Allows latent-transition scoring when an active model exists |
| `use_jepa` | `false` in schema, template enables it | Allows promoted JEPA candidate scoring |
| `use_reasoning_quality` | `false` in schema, template enables it | Reads reasoning-quality risk/preflight signals |
| `use_self_model` | `false` in schema, template enables it | Reads domain trust, failures, and caution rules |
| `max_pipeline_ms` | `500` | Bounded control-loop budget |
| `ledger_dir` | `~/.local/share/archon/cognitive` | Project templates use `.archon/cognitive` for workspace-local inspection |

## `[policy.cognitive]`

```toml
[policy.cognitive]
enabled = true
allow_autonomous_tick = true
allow_tool_suppression = true
allow_jepa_action_scoring = true
allow_self_model_updates = true
allow_autonomous_low_risk_apply = false
max_autonomous_risk = "Low"
require_human_for_prompt_changes = true
require_human_for_policy_changes = true
require_human_for_network_changes = true
require_human_for_blocking_gate_changes = true
store_raw_turn_text = false
```

| Field | Safe default | Notes |
|---|---|---|
| `enabled` | `false` in schema | Policy master gate |
| `allow_autonomous_tick` | `false` in schema | Lets `archon cognitive tick` run maintenance work |
| `allow_tool_suppression` | `true` | Allows harmless suppression of needless tool calls |
| `allow_jepa_action_scoring` | `false` in schema | Lets promoted JEPA/world-model scores influence candidate ranking |
| `allow_self_model_updates` | `false` in schema | Lets tick update SelfModel state from safe reflections |
| `allow_autonomous_low_risk_apply` | `false` | Enables self-application only for low-risk governed proposals |
| `max_autonomous_risk` | `"Low"` | Must be `Low` or `Medium`; high/critical always require a human |
| `require_human_for_*` | `true` | Hard stops for prompt, policy, network, and blocking-gate changes |
| `store_raw_turn_text` | `false` | Leave off unless you explicitly accept raw text persistence |

Foreground user work continues if the cognitive store, policy, or world-model
scorer is unavailable. The loop records degraded status instead of failing the
session.
