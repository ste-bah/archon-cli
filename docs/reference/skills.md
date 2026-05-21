# Skills

Skills are slash commands resolved through the Skill registry rather than the primary command registry. They're composable workflows — typically multi-step prompt templates with optional parameters.

## How skills compare to primary commands

| Aspect | Primary commands | Skills |
|---|---|---|
| Registry | `default_registry()` in `src/command/registry.rs` | `crates/archon-core/src/skills/{builtin,expanded}.rs` + user/plugin paths |
| Dispatch precedence | Higher | Lower (only invoked if no primary matches) |
| Implementation | Rust handler with `CommandHandler` trait | SKILL.md prompt workflow (or Rust for built-in) |
| User extension | Compile-time only (built-in) | Drop a SKILL.md file in a project/global skill root |

When you type `/foo`, archon first checks the primary registry. If no primary matches, it falls back to the skill registry.

## Built-in skills (68 total)

21 in `builtin.rs`, 35 in `expanded.rs`, 12 embedded prompt-template skills. Highlights:

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
| `/to-prd` | Generate a PRD from a feature description using ai-agent-prd template |
| `/prd-to-spec` | Convert a PRD to a phased task specification |
| `/grill-me` | Non-code alignment session — relentlessly interviews you about a plan |
| `/grill-with-docs` | Alignment + documentation — builds context glossary and ADRs |
| `/diagnose` | Systematic 6-phase debugging workflow |
| `/tdd` | Test-first red-green-refactor workflow |
| `/zoom-out` | Strategic re-grounding when lost in the weeds |
| `/spec-to-tasks` | Refine task tree into atomic dev-flow-ready task files |
| `/compose-pipeline` | Chain /to-prd → /prd-to-spec → /spec-to-tasks in one command |
| `/ci-gate-walker` | Run CI gate script and surface findings |
| `/setup-archon-skills` | Interactive 8-prompt first-run configuration wizard |
| `/write-a-skill` | Meta-skill for authoring new SKILL.md skills |

For the complete list, run `/skills` in the TUI.

## Embedded prompt-template skills (v0.1.33+)

12 skills (5 engineering + 5 archon + 2 foundation) are embedded at compile time via `include_str!()`. Their SKILL.md bodies ship in the binary and emit `Prompt` output — the agent executes the instructions using its own tools.

### Override system

Users can replace any embedded skill body without recompiling. The loader checks, in order:

1. `<workdir>/.archon/skills/<name>/SKILL.md` (subdir project)
2. `<workdir>/.archon/skills/<name>.md` (flat-file project)
3. `<workdir>/.claude/skills/<name>/SKILL.md` or `<name>.md` (legacy project)
4. User config skill roots, for example `~/.config/archon/skills/<name>/SKILL.md`
5. User data skill roots, for example `~/.local/share/archon/skills/<name>/SKILL.md`
6. Embedded fallback (binary)

This means a project can pin a custom `/ci-gate-walker` that runs its own gate script, or a user can tweak `/tdd` globally for their workflow — all without recompiling archon-cli.

## User-authored skills

Use SKILL.md files for user-authored skills. They are injected as prompts, so
the agent executes the workflow using its normal tools.

### SKILL.md format (recommended)

Drop a SKILL.md file in a project-local or user-global skill root:

- `<workdir>/.archon/skills/<name>/SKILL.md` (recommended project layout)
- `<workdir>/.archon/skills/<name>.md` (project flat-file layout)
- `~/.config/archon/skills/<name>/SKILL.md` or `<name>.md` (global config)
- Platform data dir + `archon/skills/<name>/SKILL.md` or `<name>.md` (global installed skill)

```markdown
---
name: deploy-staging
description: Deploy current branch to staging. Use when you want to push to staging.
---

# Deploy Staging

## Process

### 1. Verify build
Run `cargo build --release`

### 2. Run tests
Run `cargo test --workspace`

### 3. Push and deploy
Push to staging and trigger webhook.
```

Use `/write-a-skill` for an interactive authoring wizard.
Arguments passed to a skill are appended to the injected prompt, for example
`/deploy-staging release-candidate`.

## Plugin-supplied skills

Plugins can register skills in their `plugin.toml`:

```toml
[[skills]]
name = "my-plugin-skill"
description = "..."
trigger = "/my-plugin"
template_path = "templates/my-skill.md"
```

See [Plugins](../integrations/plugins.md) for the full plugin manifest schema.

## Discovery

Skills are loaded via a two-pass scan at startup:

1. **Subdir layout** — `<name>/SKILL.md` — scanned first
2. **Flat-file layout** — `<name>.md` — scanned second; skipped if subdir already registered the same name

This means subdir layout wins on collision. Project-local (`.archon/skills/`)
and user-global skill roots are scanned, with project taking precedence.

```bash
# In TUI
/skills                  # interactive picker
/help                    # also lists skills

# CLI
archon --list-skills     # if implemented (check --help)
```

User-authored skills are loaded when the TUI session starts. Restart `archon`
after adding or changing a skill.

## See also

- [Slash commands](slash-commands.md) — primary command registry
- [Plugins](../integrations/plugins.md) — distributing skills via plugins
- [Adding a skill](../development/adding-a-skill.md) — implementing built-in skills in Rust
