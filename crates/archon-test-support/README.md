# archon-test-support

Test-only doubles and helpers shared across the archon-cli workspace.
Phase-0 skeleton created by TASK-AGS-008.

## Why this crate exists

Phases 2..9 each need to fake out one or more subsystems inside tests:

- A fake LLM provider that records requests (REQ-FOR-D6).
- A fake memory trait that records `store_memory` calls
  (REQ-FOR-PRESERVE-D8 regression guard).
- A scoped temp `.archon/` directory with the standard layout.

Defining these inline in every crate's `#[cfg(test)] mod tests`
duplicates code and makes it hard to share fixtures across crates.
This crate hosts them once.

## Non-dependency rule

**No production crate may depend on `archon-test-support`.** The
binary graph must not pull in `wiremock`, `tempfile`, or the doubles.
Consumers opt in via `[dev-dependencies]`:

```toml
[dev-dependencies]
archon-test-support = { path = "../archon-test-support" }
```

TASK-AGS-008 validation #4 greps the workspace to enforce the rule.

## What's here (phase-0)

| Module     | Type(s)                                   | Purpose                                              |
|------------|-------------------------------------------|------------------------------------------------------|
| `provider` | `MockProvider`, `ProviderCall`, `spawn_mock_server` | LLM provider double + baseline wiremock server |
| `memory`   | `MockMemoryTrait`, `MemoryTraitLike`, `StoredMemory` | Memory trait double recording store calls   |
| `tempdir`  | `ArchonTempDir`                           | RAII temp dir with `.archon/{tasks,pipelines,agents}` layout |

Each source file is kept under 150 lines per NFR-FOR-D4-MAINTAINABILITY.

## What's NOT here yet

- Blanket impls of the real `archon_memory::MemoryTrait` — phase-8 adds
  these once the trait is stable.
- The 31-provider registry iteration test — phase-7 owns it.
- Subagent / BACKGROUND_AGENTS mock — phase-1 owns it.

Later phase tasks extend this crate by adding modules; they do not
touch the ones above.
