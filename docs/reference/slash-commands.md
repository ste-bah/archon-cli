# Slash commands

All slash commands work in the interactive TUI. Type `/help` to see them in-app.

As of v1.1.0-beta.3 the registry contains **78 primary commands** (lockstep-tested at `EXPECTED_COMMAND_COUNT = 78` in `src/command/registry.rs` and `EXPECTED_PRIMARY_COUNT = 78` in `src/command/dispatcher.rs`). Aliases come from each handler's `aliases()` method.

For shell/TUI parity, see the generated [command surface matrix](../generated/command-surface-matrix.md). It is backed by `src/command/surface_matrix.rs` and has tests that fail when registered slash primaries drift.

Beyond the 78 primaries, archon-cli ships **68 built-in skills** (33 in `crates/archon-core/src/skills/builtin.rs`, 35 in `expanded.rs`). Skills behave like slash commands but are resolved through the Skill registry — primary handlers take precedence at dispatch time.

> **Version history.** v0.1.38 added 11 primaries (Evidence Engine: `/kb`, `/prov`, `/meaning`, `/constellation`, plus gametheory inspection subcommands and the slash mirror). v0.1.40 added 2 more (`/auth` and `/chat` for the OpenAI-Codex provider surface). v0.1.45 keeps the same command count but upgrades Codex from chat/TUI-only to provider-neutral agentic surfaces where `[llm].provider = "openai-codex"`. v0.1.52 adds `/learning gnn status` to expose GNN auto-trainer diagnostics from the learning command family. v1.0.0 keeps the slash count at 78; `/archon-code`, `/archon-research`, and `/pipeline` now use the audited pipeline runtime. v1.0.1 keeps the slash count at 78 and adds shell-only hybrid retrospective analyzer modes. v1.1.0-beta.3 keeps the same slash primary count while adding provider runtime, sandbox, permissions, and governed agent-evolution shell surfaces (supersedes the unpublished v1.1.0-beta.1 and v1.1.0-beta.2 drafts with OpenShell setup/gateway/upload-mode fixes and OAuth/Gemini security hardening plus JSON-RPC validation).

## Core & meta

| Command | Aliases | Description |
|---|---|---|
| `/help` | `?`, `h` | Show available commands and shortcuts |
| `/clear` | `cls` | Clear conversation history |
| `/exit` | `q` | Exit Archon (graceful shutdown) |
| `/context` | — | Show current context window usage |
| `/status` | `info` | Session status (model, effort, token use) |
| `/doctor` | — | Run diagnostics |
| `/cost` | — | Session token cost breakdown |
| `/usage` | — | Token usage, cost, turn count |
| `/extra-usage` | — | 6-section detailed usage report |
| `/summary` | — | One-line session headline |
| `/effort` | — | Set reasoning effort (`high`/`medium`/`low`) |
| `/fast` | — | Toggle fast mode |
| `/thinking` | — | Toggle extended thinking display |
| `/plan` | — | Toggle Plan Mode |
| `/copy` | — | Copy last assistant response to clipboard |

## Git integration

| Command | Aliases | Description |
|---|---|---|
| `/diff` | — | Show git diff |
| `/commit` | — | AI-assisted commit (gathers status/diff/log into a structured prompt) |
| `/review` | — | Review a PR (no arg lists open PRs; with number reviews the diff) |

## Session management

| Command | Aliases | Description |
|---|---|---|
| `/resume` | `continue`, `open-session` | Resume a previous session |
| `/tag` | — | Toggle a searchable tag on the current session |
| `/rename` | — | Rename current session |
| `/fork` | — | Fork the session into a new branch |
| `/rewind` | — | Open message-selector overlay to rewind |
| `/checkpoint` | — | Create or restore a session checkpoint |
| `/session` | — | Show remote-session QR code + URL |

## File & project

| Command | Aliases | Description |
|---|---|---|
| `/files` | — | File-picker overlay rooted at working dir (Enter injects `@<path> `) |
| `/search` | — | Recursive basename substring search (capped at 200 results) |
| `/add-dir` | — | Add working directory for file access |
| `/recall` | — | Search memories by keyword |
| `/garden` | — | Run memory consolidation now, print report |
| `/memory` | — | Store / recall / manage memories |
| `/tasks` | `todo`, `ps`, `jobs` | List background tasks |

## Agents & pipelines

| Command | Aliases | Description |
|---|---|---|
| `/agent` | — | Umbrella: `/agent list`, `/agent info <name>`, `/agent run <name>` |
| `/run-agent` | — | Invoke a custom agent by name with a task description (async via TaskService, using the active provider) |
| `/archon-code` | — | Start the 50-agent coding pipeline on a task using the active provider |
| `/archon-research` | — | Start the 46-agent PhD research pipeline on a topic using the active provider |
| `/pipeline` | — | Shared pipeline control: `status`, `list`, `resume <session-id>`, `abort`, `verify`, `inspect`, `export-traces`. Use `/pipeline resume <session-id>` to continue interrupted `/archon-code` or `/archon-research` runs. |
| `/managed-agents` | — | Show managed-agent (remote-registry) status |
| `/refresh` | — | Re-scan the agent registry from disk |

## Configuration & discovery

| Command | Aliases | Description |
|---|---|---|
| `/theme` | — | Change UI theme |
| `/color` | — | Change prompt bar accent color |
| `/model` | `m`, `switch-model` | Show or switch the active model |
| `/permissions` | — | Show current permission mode |
| `/sandbox` | `sandbox-toggle` | Toggle sandbox restrictions (gates tool dispatch via SandboxBackend) |
| `/config` | `settings`, `prefs` | Show / modify settings |
| `/reload` | — | Force configuration reload |
| `/vim` | — | Toggle vim-style modal input |
| `/skills` | — | Browse and invoke available skills |
| `/providers` | — | List registered LLM providers; `/providers status --live` shows redacted endpoint reachability; `/providers capabilities` shows Anthropic/Codex surface support; `/providers doctor --live` runs opt-in endpoint checks |

## Infrastructure & resources

| Command | Aliases | Description |
|---|---|---|
| `/mcp` | — | Show MCP server status |
| `/connect` | — | List configured MCP servers (`/connect <name>` shows connection hint) |
| `/plugin` | — | Manage WASM plugins (`list`, `info`, `enable`, `disable`, `install`, `reload`) |
| `/reload-plugins` | — | Re-scan plugin directories from disk |
| `/hooks` | — | List or manage hook registrations (list, enable, disable, reload) |
| `/voice` | — | Show or toggle voice input configuration (status, on, off) |

## Authentication & providers (v0.1.40+)

| Command | Aliases | Description |
|---|---|---|
| `/auth` | — | Provider authentication umbrella: `/auth login --provider <anthropic\|openai-codex>`, `/auth status`, `/auth logout` |
| `/chat` | — | Single-turn chat against a selected provider: `/chat --provider openai-codex "<prompt>"`. Default provider is `anthropic`; full-session provider comes from `[llm].provider`. |
| `/login` | — | Re-authenticate the active Anthropic provider (preserved for backward compatibility — equivalent to `/auth login --provider anthropic`) |
| `/logout` | — | Sign out the active Anthropic provider (preserved for backward compatibility) |
| `/providers` | — | List registered LLM providers; `/providers status --live` shows redacted endpoint reachability; `/providers capabilities` shows the generated Archon surface-support matrix; `/providers doctor --live` adds opt-in endpoint reachability |
| `/refresh-identity` | — | Clear the `anthropic-beta` header cache and re-probe (skill, not primary) |

See [Codex authentication](../getting-started/codex-auth.md) for the ChatGPT-subscription user setup, and [identity-spoofing.md](../integrations/identity-spoofing.md) for the spoof-mode mechanics. With `[llm].provider = "openai-codex"`, `/run-agent`, `/btw`, `/archon-code`, `/archon-research`, `/gametheory`, and team-driven agentic surfaces route through Codex rather than silently constructing Anthropic clients.

## Evidence Engine (v0.1.38+)

Each command goes through the same persisted Cozo state as its `archon X` shell counterpart. See [evidence-engine.md](../evidence-engine.md) for the architecture.

| Command | Aliases | Description |
|---|---|---|
| `/docs` | — | Document intelligence: `open`, `list`, `status`, `show`, `inspect`, `chunks`, `provenance`, `model-status`, `ingest`, `index`, `search`, `answer` |
| `/kb` | — | Knowledge base: `ingest`, `list`, `search`, `process` (claims, entities, relations, contradictions), `claims`, `entities`, `relations`, `contradictions`, `stats` |
| `/prov` | — | Provenance: `trace <artifact-id>`, `export <artifact-id>` (W3C PROV JSON-LD), `verify <artifact-id>` |
| `/meaning` | — | Meaning compiler and GNN triplet source: `build --from learning-events|gametheory-runs`, `samples`, `contrastive`, `triplets`, `export --kind samples|triplets` |
| `/learning` | — | Learning diagnostics: `open`, `view`, `gnn status` |
| `/constellation` | — | Centroid profiles: `build --target project|research-domain|strategic-workflow`, `bootstrap --target memory|docs|session`, `score`, `drift`, `list` |
| `/completion` | — | Completion integrity: `inspect <run-id>`, `claims`, `evidence`, `incidents`, `verify`, `trust` |
| `/behaviour` | — | Governed learning: `list-events`, `list-proposals`, `show`, `apply`, `approve`, `deny`, `rollback`, `history`, `generate-proposals`, `status` |
| `/gametheory` | — | Game-theory umbrella: `run`, `classify-only`, `status`, `inspect`, `inspect-fingerprint`, `inspect-routing`, `list-runs`, `show`, `replay`, `list-agents`, `specimens` |
| `/learning-status` | — | Status pane for the 8 learning subsystems (separate from `/behaviour status`) |

## Analysis & insights

| Command | Aliases | Description |
|---|---|---|
| `/denials` | — | Show denied permissions in current session |
| `/rules` | — | View or edit behavioral rules |

## Utility

| Command | Aliases | Description |
|---|---|---|
| `/cancel` | `stop`, `abort` | Cancel the in-flight task (fires cancel token + dispatcher abort) |
| `/compact` | — | Trigger context compaction |
| `/export` | `save` | Export session transcript |
| `/login` | — | Re-authenticate |
| `/logout` | — | Sign out |
| `/release-notes` | — | Show version changelog |
| `/bug` | — | Report bug (links to GitHub issues) |
| `/teleport` | — | Jump to a named conversation location (hidden from `/help`) |

## PRD-driven workflow skills

These skills compose the PRD → spec → tasks → code arc. Each emits a prompt that asks the LLM to write its output via the `Write` tool — the skill itself doesn't write files. See [PRD-driven development](../cookbook/prd-driven-development.md) for the end-to-end TUI walkthrough.

| Skill | Aliases | Description |
|---|---|---|
| `/to-prd` | `/prd` | Turn the current conversation context into a PRD using the `ai-agent-prd` framework. Writes to `prds/<slug>/PRD.md`. Optional positional args become "Additional input from the user". |
| `/prd-to-spec <path>` | `/decompose-prd` | Decompose a PRD into atomic per-phase task specs using the `prdtospec` framework. Writes to `tasks/phase<N>/task<M>.md` plus `tasks/INDEX.md`. Requires the PRD path as a positional argument. |
| `/spec-to-tasks` | — | Refine the task tree from `/prd-to-spec` into atomic, dev-flow-ready task files with verification checklists. Splits coarse tasks, adds acceptance criteria + test plans + files-to-modify. |
| `/compose-pipeline` | — | Chain `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` in one command. Stops before `/archon-code` so you can review the task tree before committing to a full pipeline run. |
| `/tdd` | — | Test-driven development with red-green-refactor loop. Use when building features or fixing bugs test-first. |

## Built-in skills (selected)

68 skills total (33 in `crates/archon-core/src/skills/builtin.rs`, 35 in `expanded.rs`). Highlights:

| Skill | Description |
|---|---|
| `/git-status` (alias `/gs`) | Show repo status |
| `/branch` | Manage branches (create / switch) |
| `/pr` | Create a pull request via `gh` |
| `/restore` | List, diff, or restore file checkpoints |
| `/undo` | Undo last file modification |
| `/init` | Initialize project with ARCHON.md template |
| `/sessions` | Search and list previous sessions (with filters) |
| `/keybindings` | Show keybinding reference |
| `/statusline` | Configure status line content |
| `/insights` | Session patterns, tool usage, error rates |
| `/stats` | Daily usage, session history, model preferences |
| `/security-review` | Analyze pending changes for vulnerabilities |
| `/feedback` | Submit feedback |
| `/schedule` | Create a scheduled task (delegates to `CronCreate`) |
| `/remote-control` | Show remote control mode info |
| `/btw` | Aside marker (tangent, don't change focus) |
| `/refresh-identity` | Clear `anthropic-beta` header cache & reprobe (Anthropic only) |
| `/setup-archon-skills` | Interactive first-run wizard (8 prompts) for project bootstrapping |
| `/write-a-skill` | Meta-skill that helps author new SKILL.md skills with proper structure |
| `/zoom-out` | Tell the agent to give broader context or higher-level perspective |

For the full list, run `/skills` in the TUI or read `crates/archon-core/src/skills/{builtin,expanded}.rs`.

## Custom skills

User-authored skills live in `<workdir>/.archon/skills/<name>.toml`:

```toml
name = "my-skill"
description = "Custom workflow"
trigger = "/my-skill"
template = '''
Run these steps:
1. {step_one}
2. {step_two}
'''
```

See [Skills reference](skills.md) for the full TOML schema.

## See also

- [Skills](skills.md) — full skills documentation
- [CLI flags](cli-flags.md) — command-line flags (alternative to slash commands)
- [Tools](tools.md) — what agents can call (different from slash commands)
- [Game theory](../gametheory.md) — `/gametheory` subcommands and tool surface
- [Document intelligence](../docs.md) — `/docs` command family and evidence inspection
