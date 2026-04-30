# Slash commands

All slash commands work in the interactive TUI. Type `/help` to see them in-app.

As of v0.1.28 the registry contains **65 primary commands** (lockstep-tested at `EXPECTED_COMMAND_COUNT = 65` in `src/command/registry.rs`). Aliases come from each handler's `aliases()` method.

Beyond the 65 primaries, archon-cli ships **55 built-in skills** (21 in `crates/archon-core/src/skills/builtin.rs`, 34 in `expanded.rs`). Skills behave like slash commands but are resolved through the Skill registry — primary handlers take precedence at dispatch time.

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
| `/run-agent` | — | Invoke a custom agent by name with a task description (async via TaskService) |
| `/archon-code` | — | Run the 50-agent coding pipeline on a task |
| `/archon-research` | — | Run the 46-agent PhD research pipeline on a topic |
| `/managed-agents` | — | Show managed-agent (remote-registry) status |
| `/refresh` | — | Re-scan the agent registry from disk |

## Configuration & discovery

| Command | Aliases | Description |
|---|---|---|
| `/theme` | — | Change UI theme |
| `/color` | — | Change prompt bar accent color |
| `/model` | `m`, `switch-model` | Show or switch the active model |
| `/permissions` | — | Show current permission mode |
| `/sandbox` | `sandbox-toggle` | Toggle sandbox flag (enforcement not yet wired) |
| `/config` | `settings`, `prefs` | Show / modify settings |
| `/reload` | — | Force configuration reload |
| `/vim` | — | Toggle vim-style modal input |
| `/skills` | — | Browse and invoke available skills |
| `/providers` | — | List LLM providers ([gap] = stub, not configurable) |

## Infrastructure & resources

| Command | Aliases | Description |
|---|---|---|
| `/mcp` | — | Show MCP server status |
| `/connect` | — | List configured MCP servers (`/connect <name>` shows connection hint) |
| `/plugin` | — | List/inspect WASM plugins (enable/disable/install/reload not yet implemented) |
| `/reload-plugins` | — | Re-scan plugin directories from disk |
| `/hooks` | — | List hook registrations (enable/disable/reload not yet implemented) |
| `/voice` | — | Show voice input configuration (enable/disable/switch not yet implemented) |

## Analysis & insights

| Command | Aliases | Description |
|---|---|---|
| `/denials` | — | Show denied permissions in current session |
| `/rules` | — | View or edit behavioral rules |
| `/learning-status` | — | Status of all 8 learning subsystems |

## Utility

| Command | Aliases | Description |
|---|---|---|
| `/cancel` | `stop`, `abort` | Report idle state — use Ctrl+C to cancel a running task |
| `/compact` | — | Trigger context compaction |
| `/export` | `save` | Export session transcript |
| `/login` | — | Re-authenticate |
| `/logout` | — | Sign out |
| `/release-notes` | — | Show version changelog |
| `/bug` | — | Report bug (links to GitHub issues) |
| `/teleport` | — | Jump to a named conversation location (hidden from `/help`) |

## Built-in skills (selected)

55 skills total. Highlights:

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
| `/refresh-identity` | Clear beta header cache & reprobe |

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
