# Skills

Skills are slash commands resolved through the Skill registry rather than the primary command registry. They're composable workflows — typically multi-step prompt templates with optional parameters.

## How skills compare to primary commands

| Aspect | Primary commands | Skills |
|---|---|---|
| Registry | `default_registry()` in `src/command/registry.rs` | `crates/archon-core/src/skills/{builtin,expanded}.rs` + user/plugin paths |
| Dispatch precedence | Higher | Lower (only invoked if no primary matches) |
| Implementation | Rust handler with `CommandHandler` trait | TOML + prompt template (or Rust for built-in) |
| User extension | Compile-time only (built-in) | Drop a TOML file in `<workdir>/.archon/skills/` |

When you type `/foo`, archon first checks the primary registry. If no primary matches, it falls back to the skill registry.

## Built-in skills (67 total)

21 in `builtin.rs`, 34 in `expanded.rs`, 12 embedded prompt-template skills. Highlights:

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

1. `<workdir>/.archon/skills/<name>.md` (flat-file project)
2. `<workdir>/.archon/skills/<name>/SKILL.md` (subdir project)
3. `~/.config/archon/skills/<name>.md` (flat-file user)
4. `~/.config/archon/skills/<name>/SKILL.md` (subdir user)
5. Embedded fallback (binary)

This means a project can pin a custom `/ci-gate-walker` that runs its own gate script, or a user can tweak `/tdd` globally for their workflow — all without recompiling archon-cli.

## User-authored skills

Two formats are supported:

### SKILL.md format (recommended)

Drop a SKILL.md file in `<workdir>/.archon/skills/<name>.md` (flat) or `<workdir>/.archon/skills/<name>/SKILL.md` (subdir):

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

### TOML format (legacy)

Drop a TOML file in `<workdir>/.archon/skills/<name>.toml`:

```toml
name = "deploy-staging"
description = "Deploy current branch to staging"
trigger = "/deploy-staging"
template = '''
1. Verify build passes: `cargo build --release`
2. Run integration tests: `cargo test --workspace`
3. Push to staging branch: `git push origin HEAD:staging`
4. Trigger staging deployment via Vercel webhook
5. Smoke test the staging URL
'''
```

Fields:
- `name` — unique identifier
- `description` — shown in `/help` and `/skills`
- `trigger` — slash command that invokes this skill
- `template` — prompt body injected into the agent context
- `parameters` (optional) — array of named arguments
- `permissions` (optional) — required permission mode

### Parameterized skills

```toml
name = "release"
description = "Cut a release"
trigger = "/release"
parameters = [
    { name = "version", description = "Semver version (e.g. 0.1.28)", required = true },
    { name = "notes", description = "Release notes" },
]
template = '''
Cut release version {version}:
1. Update Cargo.toml workspace.package version
2. Update README release-notes section
3. Commit "chore(release): v{version}"
4. Tag v{version}
{notes}
'''
```

Invoke with: `/release 0.1.28 "GNN hygiene cleanup"`.

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

This means subdir layout always wins on collision. Both project-local (`.archon/skills/`) and user-global (`~/.config/archon/skills/`) paths are scanned, with project taking precedence.

```bash
# In TUI
/skills                  # interactive picker
/help                    # also lists skills

# CLI
archon --list-skills     # if implemented (check --help)
```

User-authored skills auto-reload on `/refresh` or `/reload`.

## See also

- [Slash commands](slash-commands.md) — primary command registry
- [Plugins](../integrations/plugins.md) — distributing skills via plugins
- [Adding a skill](../development/adding-a-skill.md) — implementing built-in skills in Rust
