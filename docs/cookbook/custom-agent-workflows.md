# Custom agent workflows

archon-cli has a dynamic agent system separate from the built-in registry. You can create, run, evolve, and version custom agents from natural-language descriptions.

## Agent system slash commands

| Command | Purpose |
|---|---|
| `/create-agent` | Create a custom agent from a natural language description |
| `/run-agent` | Invoke a custom agent with a task description |
| `/adjust-behavior` | Modify behavioral rules for an agent |
| `/evolve-agent` | Apply evolution suggestions (FIX, DERIVED, CAPTURED) |
| `/list-agents` | List all custom agents with metadata |
| `/archive-agent` | Archive or restore an agent |
| `/agent-history` | View version history and evolution lineage |
| `/rollback-behavior` | Rollback agent rules to a previous version |

## Creating an agent

```
/create-agent
```

archon prompts for:
- **Name** — short identifier (kebab-case)
- **Description** — what the agent does
- **Tools** — which tools the agent gets access to (comma-separated)
- **Capabilities** — high-level behaviors (e.g. `code-review`, `security-audit`)
- **System prompt** — the agent's instructions

archon writes:
```
.claude/agents/custom/<name>/
├── agent.md           # entry point with frontmatter
├── context.md         # default context envelope
├── tools.md           # allowed tools list
├── behavior.md        # behavioral rules (versioned)
├── memory-keys.json   # memory access scope
└── meta.json          # metadata: created, last_used, invocation_count
```

## Running an agent

```
/run-agent <name> "task description"
```

The Context Envelope is assembled from `context.md` + memory + LEANN search results, then a Task tool subagent is spawned with the constructed prompt.

Async dispatch:
```bash
archon run-agent-async <name> --input task.txt --detach
archon task-status <task-id>
archon task-result <task-id>
```

## Behavioral evolution

archon-cli watches agent runs for patterns:
- **FIX** — repeated failures suggest an in-place rule fix
- **DERIVED** — emergent specialization suggests creating a variant
- **CAPTURED** — successful patterns suggest extracting a new rule

Review pending evolutions:
```
/evolve-agent <name>
```

archon presents proposed changes; you accept, modify, or reject. Accepted evolutions create a new behavior version (never destructive — old versions stay in history).

## Versioning

```
/agent-history <name>          # version log
/rollback-behavior <name> v3   # rollback to version 3
```

The behavior file is versioned via append-only diffs in `.claude/agents/custom/<name>/.history/`.

## Adjusting behavior

```
/adjust-behavior <name>
```

LLM-mediated merging with diff validation prevents hallucinated rule changes. archon shows the proposed diff before applying.

## Listing and metadata

```
/list-agents              # summary
/list-agents --verbose    # full metadata
/list-agents --all        # include archived
```

Metadata tracked per agent:
- created (ISO 8601)
- last_used (ISO 8601)
- invocation_count
- average_cost_usd
- success_rate (when available from feedback)

## Archiving

```
/archive-agent <name>           # move to archived/
/archive-agent <name> --restore # restore from archive
```

Archived agents don't appear in autocomplete but their definitions are preserved.

## Permission considerations

Custom agents run with the parent session's permission mode by default. Override per-spawn:

```
/run-agent <name> --mode plan "review crates/archon-llm"
```

Or in the agent's `behavior.md`:
```
permissions:
  default_mode: plan
  always_deny:
    - WebFetch:*
    - RemoteTrigger:*
```

The agent never gets MORE permission than the parent. If the parent is in `default` mode, the agent's `bypassPermissions` request is silently downgraded.

## Sharing agents

Custom agents are plain files. Tar and copy:
```bash
tar czf my-agents.tar.gz .claude/agents/custom/
```

Or check them into a project:
```bash
git add .claude/agents/custom/code-reviewer/
git commit -m "agent: code-reviewer"
```

## See also

- [Pipelines](../architecture/pipelines.md) — built-in pipeline agents
- [Adding an agent](../development/adding-an-agent.md) — for built-in agents in Rust
- [Strategic engagement cookbook](strategic-engagement.md) — example multi-agent orchestrator
