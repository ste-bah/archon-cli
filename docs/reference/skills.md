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

## Built-in skills (55 total)

21 in `builtin.rs`, 34 in `expanded.rs`. Highlights:

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

For the complete list, run `/skills` in the TUI.

## User-authored skills

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
