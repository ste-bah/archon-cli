# Memory-driven coding

Using SONA (trajectory store) + ReasoningBank (12 modes) + GNN-enhanced embeddings to make the coding agent self-improving over a project lifetime.

## What "memory-driven" means here

Every agent action becomes a trajectory in SONA. Every successful completion strengthens the pattern. Future similar tasks query ReasoningBank for relevant patterns and the GNN enhances the query embedding with graph context. The result: archon-cli writes more idiomatic project-specific code over time.

## Initial setup

Memory-driven coding works without any setup, but you can seed it for faster convergence:

```
/memory store "this project uses anyhow::Result everywhere — never std::Result"
/memory store "all async functions in archon-pipeline use spawn_blocking for sync work"
/memory store "config sections must round-trip through serde — use #[serde(default)]"
```

Each `memory store` writes a Fact / Decision / Rule into the memory graph. AutoCapture handles the rest organically.

## Routine workflow

```
# 1. Define the task
"Add a retry layer with exponential backoff to the LLM client"

# 2. Let archon work
# Behind the scenes:
#   - ReasoningBank::reason() runs in Hybrid mode
#   - SONA queried for similar past trajectories
#   - GNN enhances the query embedding
#   - Top-3 patterns injected into agent context
#   - Agent generates code aware of project conventions
```

The agent's response will reference the project's existing patterns rather than reinventing them. If you've seeded with rules, those become part of system prompt's `<rules>` block on every turn.

## Auto-extraction

The auto-extraction subsystem watches every agent transcript and extracts structured facts:
- Entities (function names, types, files)
- Relationships (calls, depends-on, modifies)
- Claims ("X is the canonical pattern for Y")

These accrue in the memory graph automatically. After ~50 sessions in the same project, queries return increasingly project-specific results.

## Querying

```
/recall <keyword>                  # keyword search
/memory search <query>             # semantic search
```

In agent context, the model can call `memory_recall` directly:
```jsonc
{ "query": "how do we handle config validation", "max_results": 5 }
```

## Reflexion retry loop

When an agent task fails (e.g., generates code that doesn't compile), Reflexion captures the failure, generates a self-critique, and retries up to 3 times with the critique injected into context:

```toml
[reflexion]
enabled = true
max_attempts = 3
```

This is automatic — no slash command needed. The third attempt has access to all prior critiques.

## ReasoningBank in practice

The 12 reasoning modes auto-select based on query keywords (see [ModeSelector](../architecture/learning-systems.md#reasoningbank--12-reasoning-modes)). Examples:

| Query | Auto-selects mode |
|---|---|
| "why did the auth middleware fail" | Abductive |
| "what if we had used postgres instead of cozo" | Counterfactual |
| "from first principles, derive the rate limit formula" | FirstPrinciples |
| "this is similar to the OAuth flow" | Analogical |
| "the cause of the deadlock" | Causal |
| "break down the migration into steps" | Decomposition |
| "what could break this" | Adversarial |

You can force a mode:
```
/run-agent reasoning-bank --mode counterfactual "what if we had used postgres"
```

## GNN auto-retraining

The GNN trains itself in the background while you work:

| Trigger | Threshold |
|---|---|
| Memory threshold | 50 new memories since last run |
| Time elapsed | 6 hours since last run |
| Corrections | 5 user corrections since last run |

Throttled at 1 hour minimum between runs, 5 minutes max per run. Status:

```
/learning-status
```

Output includes the GNN section: weight version, last run, next eligible time, total runs, total rollbacks.

## Manual retrain

If you've fed in a lot of new context and want immediate retraining:
```
/learning-status retrain
```

Synchronous run, blocks the input loop while training. Outcome table prints after.

## See also

- [Learning systems](../architecture/learning-systems.md) — full deep dive
- [Configuration](../reference/config.md) — `[reasoning_bank]`, `[learning.gnn]`, `[reflexion]`
- [Custom agents](custom-agent-workflows.md) — wrapping ReasoningBank in your own agents
