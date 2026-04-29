# Adding an agent

archon-cli supports two agent definition formats:
1. **Flat-file YAML frontmatter** in markdown — preferred for most agents (added v0.1.10)
2. **TOML manifest + Rust** — for built-in pipeline agents that need Rust state

Plus the dynamic agent system documented in [custom-agent-workflows](../cookbook/custom-agent-workflows.md).

## Flat-file YAML agents

Drop a markdown file at `<workdir>/.archon/agents/<name>.md`:

```markdown
---
name: code-reviewer
description: Adversarial code reviewer that doesn't trust prior outputs
version: 0.1.0
tools:
  - Read
  - Grep
  - Glob
  - Bash:git*
permissions:
  default_mode: plan
capabilities:
  - code-review
  - security-audit
tags:
  - review
  - quality
model: claude-opus-4-7
effort: high
---

You are an adversarial code reviewer. Independently re-read the code under review.
Never trust prior agent outputs or summaries. Surface concerns about:

1. Correctness (logic errors, off-by-one, NaN handling)
2. Security (injection, privilege escalation, secret leakage)
3. Reliability (error handling, retry logic, idempotency)
4. Performance (hot paths, allocation patterns, blocking I/O in async)
5. Maintainability (function length, file size, naming, docs)

For each concern, cite specific file:line and propose a concrete fix.

Output verdict: APPROVED, APPROVED-WITH-NITS, REJECTED. Include rationale.
```

Required frontmatter:
- `name` — unique identifier (kebab-case)
- `description` — one-line summary
- `version` — semver

Recommended:
- `tools` — explicit allow-list (defaults to all if omitted)
- `permissions.default_mode` — agent's effective permission mode
- `capabilities` — searchable tags
- `tags` — additional searchable tags
- `model`, `effort` — overrides

## Loading

The agent loader at `crates/archon-core/src/agents/loader.rs`:
- Scans `<workdir>/.archon/agents/` and `~/.config/archon/agents/`
- Parses frontmatter + body
- Validates against the schema
- Registers into the runtime `AgentRegistry`

Run `/refresh` to re-scan after dropping a new file.

## Discovery & search

```bash
archon agent-list
archon agent-search --tag review
archon agent-search --capability code-review
archon agent-search --name-pattern "code-*"
archon agent-info code-reviewer --json
```

In TUI:
```
/agent list
/agent info code-reviewer
/agent run code-reviewer "review crates/archon-llm"
```

## Invoking

```
/run-agent code-reviewer "review the last commit"
```

Or async:
```bash
archon run-agent-async code-reviewer --input task.txt
archon task-status <task-id>
```

## TOML manifest for built-in pipeline agents

Pipeline agents (50 coding + 46 research) use the dual format:

`crates/archon-pipeline/src/agents/coding/code-quality-improver.md`:

```markdown
---
name: code-quality-improver
description: Improves code quality (refactoring, naming, clarity)
phase: 5
parallelizable: true
---

You are the code-quality-improver in the Phase 5 (Quality) batch...
```

Plus a Rust manifest at `crates/archon-pipeline/src/manifests/code-quality-improver.toml`:

```toml
name = "code-quality-improver"
version = "0.1.0"
phase = "Quality"
position = 1               # phase ordering
dependencies = ["code-generator"]   # which agents must complete first
permission_mode = "auto"
```

The dual format lets the pipeline scheduler reason about phase ordering, parallelism, and dependencies without parsing the markdown.

## Tests

For flat-file agents: a registry test confirms loading.

```rust
#[test]
fn code_reviewer_loads() {
    let registry = AgentRegistry::load_from_path(".archon/agents/").unwrap();
    let agent = registry.get("code-reviewer").expect("missing code-reviewer");
    assert_eq!(agent.version, "0.1.0");
    assert!(agent.tools.contains(&"Read".to_string()));
}
```

For pipeline agents: phase-level tests at `crates/archon-pipeline/tests/coding_agents.rs` enumerate expected names.

## Permissions

The agent loader merges agent permissions with parent session permissions. Rules:
- Agent's `permissions.default_mode` becomes the agent's effective mode if NOT MORE permissive than the parent
- Parent in `default`, agent requests `bypassPermissions` → agent runs in `default`
- Parent in `auto`, agent requests `plan` → agent runs in `plan` (more restrictive is allowed)

This is enforced at spawn time, not trusted from the agent definition.

## Versioning

Multiple versions of the same agent name can coexist:
```
.archon/agents/
├── code-reviewer.md            # latest
├── archive/
│   ├── code-reviewer-v0.1.0.md
│   └── code-reviewer-v0.0.5.md
```

Invoke a specific version:
```bash
archon run-agent-async code-reviewer --version "0.1.0"
archon run-agent-async code-reviewer --version "^0.1"   # semver requirement
```

## See also

- [Pipelines](../architecture/pipelines.md)
- [Custom agents](../cookbook/custom-agent-workflows.md) — the dynamic agent system
- [Permissions](../reference/permissions.md)
- [Dev flow gates](dev-flow-gates.md)
