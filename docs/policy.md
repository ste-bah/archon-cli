# Archon Policy

Archon policy is a TOML gate for features that can change behaviour, use
networked providers, expose services, or auto-apply learned updates.

## Locations

Policy is loaded in this order, with later files overriding earlier files:

1. `/etc/archon/policy.toml`
2. `~/.archon/policy.toml`
3. `<workspace>/.archon/policy.toml`

If no policy file exists, Archon uses default-deny for network, VLM,
game-theory Tier 11, MCP exposure, and governed-learning auto-apply.

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

[policy.docs.vlm]
enabled = false
mode = "disabled" # disabled | local | cloud | hybrid
allow_cloud = false
require_user_confirmation_for_cloud = true
```

## Current Gates

`archon gametheory run --enable-tier11` only enables Tier 11 specialists when
`policy.gametheory.enable_tier11 = true`.

Document VLM descriptions are denied unless `[policy.docs.vlm]` enables a local,
cloud, or hybrid provider and the matching worker/network policy allows it.

Governed-learning auto-apply is denied by default. Low-risk updates can only
auto-apply when `policy.learning.auto_apply_low_risk = true`; prompt, blocking
gate, policy, and network changes remain approval-gated.
